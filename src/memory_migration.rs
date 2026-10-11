//! Offline identity cutover. The caller owns the exclusively opened original database.
//! Rollback uses the independent pre-cutover redb snapshot with the old binary, while stopped.
use crate::platform::{
    keys,
    types::{RoomKey, SpaceKey, UserKey},
};
use crate::BotError;
use redb::{Database, ReadableTable, TableDefinition, WriteTransaction};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const CONTENT: TableDefinition<&str, &str> = TableDefinition::new("content");
const MODIFIED: TableDefinition<&str, u64> = TableDefinition::new("modified");
const METADATA: TableDefinition<&str, u64> = TableDefinition::new("memory_metadata");
const VERSION_KEY: &str = "schema_version";
const CURRENT_VERSION: u64 = 2;
static BACKUP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MigrationOutcome {
    Unchanged,
    Initialized,
    Migrated { backup_dir: PathBuf },
}

#[derive(Debug, Serialize)]
struct Mapping {
    old_key: String,
    new_key: String,
    modified: Option<u64>,
}

#[derive(Debug, Serialize)]
struct UnresolvedLine {
    line: usize,
    original: String,
}

#[derive(Debug, Serialize)]
struct OriginalRecord<'a> {
    body: &'a str,
    modified: Option<u64>,
}

#[derive(Serialize)]
struct Manifest<'a> {
    old_schema_version: Option<u64>,
    new_schema_version: u64,
    backup_database: &'static str,
    mappings: &'a [Mapping],
    unresolved_keys: &'a [String],
    unresolved_scan_lines: &'a [UnresolvedLine],
    conflicts: &'a [String],
    original_scan: Option<OriginalRecord<'a>>,
    original_index: Option<OriginalRecord<'a>>,
    rollback: &'static str,
}

struct Snapshot {
    content: BTreeMap<String, String>,
    modified: BTreeMap<String, u64>,
    metadata: BTreeMap<String, u64>,
}

struct Plan {
    mappings: Vec<Mapping>,
    unresolved_keys: Vec<String>,
    unresolved_scan_lines: Vec<UnresolvedLine>,
    conflicts: Vec<String>,
    scan: Option<String>,
}

/// Only this explicit legacy boundary interprets decimal IDs as Discord IDs.
/// Leading zero spellings map to the same destination and are checked for collisions.
pub(crate) fn normalize_legacy_record_key(key: &str) -> String {
    for (prefix, person) in [("kisiler/", true), ("kanallar/", false)] {
        if let Some(id) = key.strip_prefix(prefix).and_then(|s| s.strip_suffix(".md")) {
            if let Some(id) = legacy_id(id) {
                return if person {
                    keys::person_record(&UserKey::discord(id))
                } else {
                    keys::room_record(&crate::platform::types::ConversationKey {
                        room: RoomKey::discord(id),
                        thread: None,
                    })
                };
            }
        }
    }
    key.to_owned()
}

fn legacy_id(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

/// Retains every unrecognized line and the original whitespace/newline convention.
pub(crate) fn normalize_legacy_scan_text(body: &str) -> String {
    transform_scan(body).0
}

fn transform_scan(body: &str) -> (String, Vec<UnresolvedLine>) {
    let mut normalized = String::with_capacity(body.len());
    let mut unresolved = Vec::new();
    for (index, original) in body.split_inclusive('\n').enumerate() {
        let value = original.trim();
        if let Some(id) = legacy_id(value) {
            let start = original.len() - original.trim_start().len();
            let end = original.trim_end().len();
            normalized.push_str(&original[..start]);
            normalized.push_str(SpaceKey::discord(id).as_str());
            normalized.push_str(&original[end..]);
        } else {
            normalized.push_str(original);
            if !value.is_empty() && keys::parse_space_line(value).is_none() {
                unresolved.push(UnresolvedLine {
                    line: index + 1,
                    original: original.to_owned(),
                });
            }
        }
    }
    (normalized, unresolved)
}

fn snapshot(txn: &WriteTransaction) -> Result<Snapshot, BotError> {
    let mut content = BTreeMap::new();
    for row in txn.open_table(CONTENT)?.iter()? {
        let (key, value) = row?;
        content.insert(key.value().to_owned(), value.value().to_owned());
    }
    let mut modified = BTreeMap::new();
    for row in txn.open_table(MODIFIED)?.iter()? {
        let (key, value) = row?;
        modified.insert(key.value().to_owned(), value.value());
    }
    let mut metadata = BTreeMap::new();
    for row in txn.open_table(METADATA)?.iter()? {
        let (key, value) = row?;
        metadata.insert(key.value().to_owned(), value.value());
    }
    Ok(Snapshot {
        content,
        modified,
        metadata,
    })
}

fn plan(snapshot: &Snapshot) -> Plan {
    let mut mappings = Vec::new();
    let mut destinations = BTreeSet::new();
    let mut unresolved_keys = BTreeSet::new();
    let mut conflicts = Vec::new();
    for old_key in snapshot.content.keys() {
        let new_key = normalize_legacy_record_key(old_key);
        if new_key != *old_key {
            if snapshot.content.contains_key(&new_key) || snapshot.modified.contains_key(&new_key) {
                conflicts.push(format!(
                    "memory migration destination already exists: {old_key} -> {new_key}"
                ));
            }
            if !destinations.insert(new_key.clone()) {
                conflicts.push(format!("memory migration duplicate destination: {new_key}"));
            }
            mappings.push(Mapping {
                old_key: old_key.clone(),
                new_key,
                modified: snapshot.modified.get(old_key).copied(),
            });
        } else if (old_key.starts_with("kisiler/") && keys::parse_person_record(old_key).is_none())
            || (old_key.starts_with("kanallar/") && keys::parse_room_record(old_key).is_none())
        {
            unresolved_keys.insert(old_key.clone());
        }
    }
    // Orphan timestamps are original data too: preserve them and expose them for repair.
    for key in snapshot.modified.keys() {
        if !snapshot.content.contains_key(key) {
            unresolved_keys.insert(key.clone());
        }
    }
    let (scan, unresolved_scan_lines) = match snapshot.content.get("taranan.md") {
        Some(body) => {
            let (normalized, unresolved) = transform_scan(body);
            (
                if normalized != *body {
                    Some(normalized)
                } else {
                    None
                },
                unresolved,
            )
        }
        None => (None, Vec::new()),
    };
    Plan {
        mappings,
        unresolved_keys: unresolved_keys.into_iter().collect(),
        unresolved_scan_lines,
        conflicts,
        scan,
    }
}

fn backup_directory(path: &Path) -> Result<PathBuf, BotError> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let filename = path
        .file_name()
        .ok_or("memory database path has no filename")?;
    let epoch = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    loop {
        let sequence = BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let directory = parent.join(format!(
            "{}.migration-v2-{epoch}-{}-{sequence}",
            filename.to_string_lossy(),
            std::process::id()
        ));
        match fs::create_dir(&directory) {
            Ok(()) => return Ok(directory),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
}

fn save_backup(snapshot: &Snapshot, plan: &Plan, directory: &Path) -> Result<(), BotError> {
    // Never copy the open original file: these exact table rows are read under its write lock.
    let backup = Database::create(directory.join("original.redb"))?;
    let txn = backup.begin_write()?;
    {
        let mut content = txn.open_table(CONTENT)?;
        for (key, body) in &snapshot.content {
            content.insert(key.as_str(), body.as_str())?;
        }
        let mut modified = txn.open_table(MODIFIED)?;
        for (key, value) in &snapshot.modified {
            modified.insert(key.as_str(), *value)?;
        }
        let mut metadata = txn.open_table(METADATA)?;
        for (key, value) in &snapshot.metadata {
            metadata.insert(key.as_str(), *value)?;
        }
    }
    txn.commit()?;
    drop(backup);
    let original = |key: &str| {
        snapshot.content.get(key).map(|body| OriginalRecord {
            body: body.as_str(),
            modified: snapshot.modified.get(key).copied(),
        })
    };
    let manifest = Manifest {
        old_schema_version: snapshot.metadata.get(VERSION_KEY).copied(),
        new_schema_version: CURRENT_VERSION,
        backup_database: "original.redb",
        mappings: &plan.mappings,
        unresolved_keys: &plan.unresolved_keys,
        unresolved_scan_lines: &plan.unresolved_scan_lines,
        conflicts: &plan.conflicts,
        original_scan: original("taranan.md"),
        original_index: original("INDEX.md"),
        rollback: "Stop the process; restore original.redb as the active database and use the old binary. Leave the historical arsiv directory unchanged.",
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("manifest.json"))?;
    serde_json::to_writer_pretty(&mut file, &manifest)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    File::open(directory)?.sync_all()?;
    File::open(directory.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()?;
    Ok(())
}

fn apply(txn: &WriteTransaction, snapshot: &Snapshot, plan: &Plan) -> Result<(), BotError> {
    {
        let mut content = txn.open_table(CONTENT)?;
        let mut modified = txn.open_table(MODIFIED)?;
        for mapping in &plan.mappings {
            content.insert(
                mapping.new_key.as_str(),
                snapshot.content[&mapping.old_key].as_str(),
            )?;
            if let Some(value) = mapping.modified {
                modified.insert(mapping.new_key.as_str(), value)?;
            }
            content.remove(mapping.old_key.as_str())?;
            modified.remove(mapping.old_key.as_str())?;
        }
        if let Some(scan) = &plan.scan {
            content.insert("taranan.md", scan.as_str())?;
        }
    }
    txn.open_table(METADATA)?
        .insert(VERSION_KEY, CURRENT_VERSION)?;
    Ok(())
}

/// Plans, backs up, renames and advances the schema under a single original write transaction.
/// Every error leaves the active database unchanged; any diagnostic backup already created is kept.
pub(crate) fn migrate_database(db: &Database, path: &Path) -> Result<MigrationOutcome, BotError> {
    let txn = db.begin_write()?;
    let version = txn
        .open_table(METADATA)?
        .get(VERSION_KEY)?
        .map(|v| v.value());
    match version {
        Some(CURRENT_VERSION) => return Ok(MigrationOutcome::Unchanged),
        Some(version) if version > CURRENT_VERSION => {
            return Err(format!(
                "unsupported memory schema version {version} (current {CURRENT_VERSION})"
            )
            .into())
        }
        _ => {}
    }
    let original = snapshot(&txn)?;
    if original.content.is_empty() && original.modified.is_empty() {
        txn.open_table(METADATA)?
            .insert(VERSION_KEY, CURRENT_VERSION)?;
        txn.commit()?;
        return Ok(MigrationOutcome::Initialized);
    }
    let plan = plan(&original);
    let backup_dir = backup_directory(path)?;
    save_backup(&original, &plan, &backup_dir)?;
    if !plan.conflicts.is_empty() {
        return Err(format!(
            "{}; original snapshot and conflict manifest: {}",
            plan.conflicts.join("; "),
            backup_dir.display()
        )
        .into());
    }
    apply(&txn, &original, &plan)?;
    txn.commit()?;
    Ok(MigrationOutcome::Migrated { backup_dir })
}

#[cfg(test)]
mod tests {
    use super::*;
    use redb::ReadableDatabase;

    struct Fixture {
        root: PathBuf,
    }
    impl Fixture {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "memory-migration-{label}-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }
        fn path(&self) -> PathBuf {
            self.root.join("active.redb")
        }
        fn db(&self) -> Database {
            Database::create(self.path()).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn seed(db: &Database, content: &[(&str, &str, Option<u64>)]) {
        let txn = db.begin_write().unwrap();
        {
            let mut bodies = txn.open_table(CONTENT).unwrap();
            let mut modified = txn.open_table(MODIFIED).unwrap();
            for (key, body, mtime) in content {
                bodies.insert(*key, *body).unwrap();
                if let Some(mtime) = mtime {
                    modified.insert(*key, *mtime).unwrap();
                }
            }
        }
        txn.commit().unwrap();
    }

    fn rows(
        db: &Database,
    ) -> (
        BTreeMap<String, String>,
        BTreeMap<String, u64>,
        BTreeMap<String, u64>,
    ) {
        let txn = db.begin_write().unwrap();
        let original = snapshot(&txn).unwrap();
        (original.content, original.modified, original.metadata)
    }

    fn version(db: &Database) -> Option<u64> {
        let txn = db.begin_read().unwrap();
        let version = txn
            .open_table(METADATA)
            .unwrap()
            .get(VERSION_KEY)
            .unwrap()
            .map(|v| v.value());
        version
    }

    #[test]
    fn realistic_v1_copy_is_lossless_and_backup_is_usable() {
        let fixture = Fixture::new("realistic-copy");
        let source_path = fixture.root.join("source.redb");
        let source = Database::create(&source_path).unwrap();
        let mut records = Vec::new();
        for id in 1..=48 {
            records.push((format!("kisiler/{id}.md"), format!("# Kişi {id}\npuan: +2\n\n## Bildiklerin\nUnicode: İ, 🦀\n\n## Son olaylar\n- Tarihçe {id}\n"), Some(1_700_000_000 + id)));
        }
        for id in 101..=124 {
            records.push((
                format!("kanallar/{id}.md"),
                format!("Ali: ilk konuşma {id}\r\nAyşe: değişmeden kalsın\r\n"),
                Some(1_690_000_000 + id),
            ));
        }
        for (key, body) in [
            ("profil.md", "profil\n"),
            ("huy.md", "huy"),
            ("kendim.md", "kimlik"),
            ("model.md", "model"),
            ("dusunme.md", "yüksek"),
            ("uyku.md", "8"),
            ("buyume.md", "23"),
            ("konular/rust.md", "# Rust\nnotlar"),
            ("olaylar/2025-03.md", "eski olaylar\n"),
            ("INDEX.md", "numeric v1 derived index\n1 Ali\n"),
            ("kisiler/eski-ali.md", "slug record"),
            ("kanallar/unknown.md", "unknown room"),
            ("kisiler/discord:user:999.md", "already canonical"),
            (
                "taranan.md",
                "9\n 10 \r\ndiscord:space:11\nnot-a-guild\n18446744073709551616\n\n",
            ),
        ] {
            records.push((key.to_owned(), body.to_owned(), Some(1234)));
        }
        records.push(("kisiler/88.md".to_owned(), "no mtime\n".to_owned(), None));
        seed(
            &source,
            &records
                .iter()
                .map(|(k, b, m)| (k.as_str(), b.as_str(), *m))
                .collect::<Vec<_>>(),
        );
        let txn = source.begin_write().unwrap();
        txn.open_table(METADATA)
            .unwrap()
            .insert("operator_flag", 77)
            .unwrap();
        txn.open_table(METADATA)
            .unwrap()
            .insert(VERSION_KEY, 1)
            .unwrap();
        txn.open_table(MODIFIED)
            .unwrap()
            .insert("orphan.md", 555)
            .unwrap();
        txn.commit().unwrap();
        let before = rows(&source);
        // Explicitly exercise a copy of realistic v1 data, never copy an open writable DB.
        drop(source);
        fs::copy(&source_path, fixture.path()).unwrap();
        fs::create_dir_all(fixture.root.join("arsiv/kisiler")).unwrap();
        let archive = fixture.root.join("arsiv/kisiler/1.md");
        fs::write(&archive, b"original archived raw bytes\r\n\xff").unwrap();
        let archived_mtime = fs::metadata(&archive).unwrap().modified().unwrap();
        let db = fixture.db();
        let MigrationOutcome::Migrated { backup_dir } =
            migrate_database(&db, &fixture.path()).unwrap()
        else {
            panic!("expected backup");
        };
        let after = rows(&db);
        for (old, body, mtime) in &records {
            let new = normalize_legacy_record_key(old);
            if old == "taranan.md" {
                assert_eq!(after.0[&new], "discord:space:9\n discord:space:10 \r\ndiscord:space:11\nnot-a-guild\n18446744073709551616\n\n");
            } else {
                assert_eq!(after.0[&new], *body);
            }
            assert_eq!(after.1.get(&new).copied(), *mtime);
            if new != *old {
                assert!(!after.0.contains_key(old));
                assert!(!after.1.contains_key(old));
            }
        }
        assert_eq!(after.0.len(), before.0.len());
        assert_eq!(after.1.len(), before.1.len());
        assert_eq!(after.2[VERSION_KEY], 2);
        assert_eq!(after.2["operator_flag"], 77);
        let sorted = |modified: &BTreeMap<String, u64>| {
            let mut rows: Vec<_> = modified
                .iter()
                .map(|(key, mtime)| (*mtime, normalize_legacy_record_key(key)))
                .collect();
            rows.sort();
            rows
        };
        assert_eq!(sorted(&before.1), sorted(&after.1));
        let backup = Database::open(backup_dir.join("original.redb")).unwrap();
        assert_eq!(rows(&backup), before);
        assert_eq!(version(&backup), Some(1));
        assert_eq!(
            rows(&backup).0["INDEX.md"],
            "numeric v1 derived index\n1 Ali\n"
        );
        let manifest_bytes = fs::read(backup_dir.join("manifest.json")).unwrap();
        let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
        assert_eq!(manifest["mappings"].as_array().unwrap().len(), 73);
        assert!(manifest["mappings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["old_key"] == "kisiler/88.md" && r["modified"].is_null()));
        assert_eq!(manifest["original_scan"]["body"], before.0["taranan.md"]);
        assert_eq!(manifest["original_scan"]["modified"], 1234);
        assert_eq!(manifest["original_index"]["body"], before.0["INDEX.md"]);
        let unresolved = manifest["unresolved_keys"].as_array().unwrap();
        for key in ["kisiler/eski-ali.md", "kanallar/unknown.md", "orphan.md"] {
            assert!(unresolved.iter().any(|v| v == key));
        }
        assert_eq!(
            manifest["unresolved_scan_lines"].as_array().unwrap().len(),
            2
        );
        assert_eq!(
            fs::read(&archive).unwrap(),
            b"original archived raw bytes\r\n\xff"
        );
        assert_eq!(
            fs::metadata(&archive).unwrap().modified().unwrap(),
            archived_mtime
        );
        let files_before: BTreeSet<_> = fs::read_dir(&fixture.root)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            migrate_database(&db, &fixture.path()).unwrap(),
            MigrationOutcome::Unchanged
        );
        assert_eq!(rows(&db), after);
        assert_eq!(
            fs::read(backup_dir.join("manifest.json")).unwrap(),
            manifest_bytes
        );
        assert_eq!(
            fs::read_dir(&fixture.root)
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect::<BTreeSet<_>>(),
            files_before
        );
    }

    #[test]
    fn content_timestamp_and_duplicate_collisions_abort_every_record() {
        for collision in ["content", "timestamp", "duplicate"] {
            let fixture = Fixture::new(collision);
            let db = fixture.db();
            seed(
                &db,
                &[
                    ("kisiler/1.md", "first", Some(11)),
                    ("kisiler/2.md", "second", Some(22)),
                    ("kanallar/7.md", "room", Some(33)),
                ],
            );
            match collision {
                "content" => seed(&db, &[("kisiler/discord:user:2.md", "existing", None)]),
                "duplicate" => seed(&db, &[("kisiler/02.md", "duplicate spelling", Some(44))]),
                _ => {
                    let txn = db.begin_write().unwrap();
                    txn.open_table(MODIFIED)
                        .unwrap()
                        .insert("kisiler/discord:user:2.md", 99)
                        .unwrap();
                    txn.commit().unwrap();
                }
            }
            let before = rows(&db);
            assert!(migrate_database(&db, &fixture.path()).is_err());
            assert_eq!(rows(&db), before);
            let directories: Vec<_> = fs::read_dir(&fixture.root)
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| p.is_dir())
                .collect();
            assert_eq!(directories.len(), 1);
            let backup = Database::open(directories[0].join("original.redb")).unwrap();
            assert_eq!(rows(&backup), before);
            let manifest: serde_json::Value =
                serde_json::from_slice(&fs::read(directories[0].join("manifest.json")).unwrap())
                    .unwrap();
            assert!(!manifest["conflicts"].as_array().unwrap().is_empty());
        }
    }

    #[test]
    fn backup_or_manifest_failure_leaves_original_unchanged() {
        let fixture = Fixture::new("io-rollback");
        let db = fixture.db();
        seed(&db, &[("kisiler/1.md", "person", Some(77))]);
        let before = rows(&db);
        let blocker = fixture.root.join("not-a-directory");
        fs::write(&blocker, "block").unwrap();
        assert!(migrate_database(&db, &blocker.join("active.redb")).is_err());
        assert_eq!(rows(&db), before);
        let txn = db.begin_write().unwrap();
        let original = snapshot(&txn).unwrap();
        let plan = plan(&original);
        let backup_dir = backup_directory(&fixture.path()).unwrap();
        // Force manifest failure after the independent redb snapshot has been committed.
        fs::create_dir(backup_dir.join("manifest.json")).unwrap();
        assert!(save_backup(&original, &plan, &backup_dir).is_err());
        drop(txn);
        assert_eq!(rows(&db), before);
        let backup = Database::open(backup_dir.join("original.redb")).unwrap();
        assert_eq!(rows(&backup), before);
    }

    #[test]
    fn transaction_error_after_renames_rolls_back_body_mtime_and_version() {
        let fixture = Fixture::new("transaction-rollback");
        let db = fixture.db();
        seed(
            &db,
            &[
                ("kisiler/1.md", "body", Some(9)),
                ("taranan.md", "7\n", None),
            ],
        );
        let before = rows(&db);
        let result: Result<(), BotError> = (|| {
            let txn = db.begin_write()?;
            let original = snapshot(&txn)?;
            let plan = plan(&original);
            apply(&txn, &original, &plan)?;
            // A real redb table-type error after all mutations: dropping txn must undo all.
            txn.open_table(TableDefinition::<&str, &str>::new("memory_metadata"))?;
            txn.commit()?;
            Ok(())
        })();
        assert!(result.is_err());
        assert_eq!(rows(&db), before);
    }

    #[test]
    fn future_version_rejected_and_empty_database_initialized_without_backup() {
        let fixture = Fixture::new("empty");
        let db = fixture.db();
        assert_eq!(
            migrate_database(&db, &fixture.path()).unwrap(),
            MigrationOutcome::Initialized
        );
        assert_eq!(version(&db), Some(2));
        assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 1);
        assert_eq!(
            migrate_database(&db, &fixture.path()).unwrap(),
            MigrationOutcome::Unchanged
        );
        let txn = db.begin_write().unwrap();
        txn.open_table(METADATA)
            .unwrap()
            .insert(VERSION_KEY, 3)
            .unwrap();
        txn.commit().unwrap();
        let before = rows(&db);
        assert!(migrate_database(&db, &fixture.path()).is_err());
        assert_eq!(rows(&db), before);
        assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 1);
    }

    #[test]
    fn malformed_and_canonical_scan_lines_are_not_rewritten() {
        let input =
            "discord:space:9\r\ntelegram:space:-100\nword\n+42\n4.2\n18446744073709551616\n\n0012";
        let (output, unresolved) = transform_scan(input);
        assert_eq!(output, "discord:space:9\r\ntelegram:space:-100\nword\n+42\n4.2\n18446744073709551616\n\ndiscord:space:12");
        assert_eq!(unresolved.len(), 4);
        assert_eq!(unresolved[0].line, 3);
        assert_eq!(unresolved[0].original, "word\n");
        for key in [
            "kisiler/old-slug.md",
            "kisiler/+42.md",
            "kisiler/18446744073709551616.md",
            "kanallar/sub/7.md",
            "kisiler/telegram:user:42.md",
        ] {
            assert_eq!(normalize_legacy_record_key(key), key);
        }
    }
}
