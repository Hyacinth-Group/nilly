// One-time (safe to re-run) migrator: reads an old-style `durum/` markdown tree — from
// before the redb migration (see memory.rs's module doc) — and imports every record into a
// redb database, preserving each file's real modification time so `memory::files`'s
// "most recently changed first" ordering doesn't reset to "migration day" for everyone.
// `durum/arsiv/` is never touched: it stays real files, see memory.rs.
//
//   cargo run -- migrate-durum [--from <dir>] [--to <redb-path>] [--dry-run] [--force]
//
// Never deletes or moves the source .md files — non-destructive, so the operator can start
// the bot against the new database, sanity-check `/durum`/`/zihin`, and only then remove
// the old tree by hand.

use super::*;
use std::fs;
use std::path::{Path, PathBuf};

/// One source record: its database key, content, and exact source file mtime in Unix
/// seconds. Unreadable files or timestamps fail the import rather than inventing data.
struct Record {
    key: String,
    content: String,
    modified: u64,
}

pub fn run(args: &[String]) -> Result<(), BotError> {
    let mut from = PathBuf::from(STATE_DIR);
    let mut to = PathBuf::from(STATE_DIR).join("hafiza.redb");
    let mut dry_run = false;
    let mut force = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--from" => {
                i += 1;
                from = PathBuf::from(args.get(i).ok_or("--from bir değer ister")?);
            }
            "--to" => {
                i += 1;
                to = PathBuf::from(args.get(i).ok_or("--to bir değer ister")?);
            }
            "--dry-run" => dry_run = true,
            "--force" => force = true,
            other => return Err(format!("bilinmeyen argüman: {other}").into()),
        }
        i += 1;
    }

    let records = normalize_records(collect(&from, &from, "arsiv")?)?;
    let (kisi, konu, olay, tekil) = counts(&records);
    println!(
        "{} altında bulundu: {kisi} kişi, {konu} konu, {olay} ay (olaylar), {tekil} tekil kayıt — toplam {}",
        from.display(),
        records.len()
    );

    if records.is_empty() {
        println!("taşınacak .md dosyası yok, çıkılıyor");
        return Ok(());
    }
    if dry_run {
        println!("--dry-run: hiçbir şey yazılmadı");
        return Ok(());
    }

    memory::init_result(&to)?;
    let existing = memory::record_count_result()?;
    // Startup rebuilds the derived index even on a fresh database.
    let generated_index = u64::from(memory::read_result("INDEX.md")?.is_some());
    if existing > generated_index && !force {
        return Err(format!(
            "{} zaten {existing} kayıt içeriyor; üzerine yazmak için --force ekle",
            to.display()
        )
        .into());
    }

    for r in &records {
        memory::write_with_mtime_result(&r.key, &r.content, r.modified)?;
    }
    memory::refresh_index_result()?;

    println!(
        "tamamlandı: {} kayıt {} içine yazıldı",
        records.len(),
        to.display()
    );
    println!("orijinal .md dosyaları dokunulmadan kaldı — /durum ve /zihin ile doğruladıktan sonra elle silebilirsin");
    Ok(())
}

/// Recursively collects markdown records with `/`-joined keys relative to `base`.
/// Skips `skip_top_level` only directly under `base`, so same-named person/topic
/// files are still included.
fn collect(dir: &Path, base: &Path, skip_top_level: &str) -> Result<Vec<Record>, BotError> {
    let mut out = Vec::new();
    let entries = fs::read_dir(dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            if dir == base && path.file_name().and_then(|n| n.to_str()) == Some(skip_top_level) {
                continue; // historical archives are not active lookup data
            }
            out.extend(collect(&path, base, skip_top_level)?);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let rel = path.strip_prefix(base)?;
        let key = rel
            .components()
            .map(|c| c.as_os_str().to_str().ok_or("import filename is not UTF-8"))
            .collect::<Result<Vec<_>, _>>()?
            .join("/");
        let content = fs::read_to_string(&path)?;
        let modified = fs::metadata(&path)?
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        out.push(Record {
            key,
            content,
            modified,
        });
    }
    Ok(out)
}

/// Normalize the complete input set before opening or writing the destination.
/// Two historical spellings (or a historical and canonical file) must not overwrite each other.
fn normalize_records(records: Vec<Record>) -> Result<Vec<Record>, BotError> {
    let mut seen = std::collections::BTreeMap::new();
    let mut normalized = Vec::with_capacity(records.len());
    for record in records {
        let key = memory_migration::normalize_legacy_record_key(&record.key);
        if let Some(previous) = seen.insert(key.clone(), record.key.clone()) {
            return Err(format!(
                "import destination collision: {previous} and {} -> {key}",
                record.key
            )
            .into());
        }
        let content = if key == "taranan.md" {
            memory_migration::normalize_legacy_scan_text(&record.content)
        } else {
            record.content
        };
        normalized.push(Record {
            key,
            content,
            modified: record.modified,
        });
    }
    Ok(normalized)
}

fn counts(records: &[Record]) -> (usize, usize, usize, usize) {
    let mut kisi = 0;
    let mut konu = 0;
    let mut olay = 0;
    let mut tekil = 0;
    for r in records {
        if r.key.starts_with("kisiler/") {
            kisi += 1;
        } else if r.key.starts_with("konular/") {
            konu += 1;
        } else if r.key.starts_with("olaylar/") {
            olay += 1;
        } else {
            tekil += 1;
        }
    }
    (kisi, konu, olay, tekil)
}

#[cfg(test)]
mod test {
    use super::*;

    /// Verifies `collect` finds `.md` files recursively, computes `/`-joined keys, skips
    /// only the top-level `arsiv/` (not a same-named file elsewhere), and ignores non-`.md`
    /// files — against a small fixture tree in the documented on-disk format.
    #[test]
    fn collect_walks_and_excludes_top_level_arsiv() {
        let root = std::env::temp_dir().join(format!(
            "nibby-test-migrate-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("kisiler")).unwrap();
        fs::create_dir_all(root.join("konular")).unwrap();
        fs::create_dir_all(root.join("arsiv/kisiler")).unwrap();
        fs::write(root.join("profil.md"), "grup profili").unwrap();
        fs::write(root.join("model.md"), "bir-model").unwrap();
        fs::write(
            root.join("kisiler/1.md"),
            "# Ali\nid: 1\npuan: +2\netiket: \nnot: \n\n## Bildiklerin\n\n## Son olaylar\n",
        )
        .unwrap();
        fs::write(root.join("konular/rust.md"), "# rust\netiket: \n\n- not\n").unwrap();
        fs::write(root.join("arsiv/kisiler/1.md"), "eski hal").unwrap();
        fs::write(root.join("not-markdown.txt"), "atlanmalı").unwrap();

        let records = collect(&root, &root, "arsiv").unwrap();
        let keys: Vec<&str> = records.iter().map(|r| r.key.as_str()).collect();

        assert!(keys.contains(&"profil.md"));
        assert!(keys.contains(&"model.md"));
        assert!(keys.contains(&"kisiler/1.md"));
        assert!(keys.contains(&"konular/rust.md"));
        assert!(!keys.iter().any(|k| k.starts_with("arsiv/")));
        assert!(!keys.iter().any(|k| k.contains("not-markdown")));

        let (kisi, konu, olay, tekil) = counts(&records);
        assert_eq!(kisi, 1);
        assert_eq!(konu, 1);
        assert_eq!(olay, 0);
        assert_eq!(tekil, 2); // profil.md + model.md

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn import_normalizes_identity_records_and_preserves_everything_else() {
        let record = |key: &str, content: &str, modified| Record {
            key: key.to_owned(),
            content: content.to_owned(),
            modified,
        };
        let records = normalize_records(vec![
            record("kisiler/42.md", "person bytes\r\n", 123),
            record("kanallar/7.md", "room bytes\n", 456),
            record("taranan.md", "9\nbad\ntelegram:space:-100\n", 789),
            record("kisiler/unknown.md", "unknown", 10),
            record("profil.md", "profile", 11),
            record("kisiler/telegram:user:42.md", "other platform", 12),
        ])
        .unwrap();
        assert_eq!(records.len(), 6);
        assert_eq!(records[0].key, "kisiler/discord:user:42.md");
        assert_eq!(records[0].content, "person bytes\r\n");
        assert_eq!(records[0].modified, 123);
        assert_eq!(records[1].key, "kanallar/discord:room:7.md");
        assert_eq!(records[1].content, "room bytes\n");
        assert_eq!(records[1].modified, 456);
        assert_eq!(
            records[2].content,
            "discord:space:9\nbad\ntelegram:space:-100\n"
        );
        assert_eq!(records[2].modified, 789);
        assert_eq!(records[3].key, "kisiler/unknown.md");
        assert_eq!(records[3].content, "unknown");
        assert_eq!(records[4].key, "profil.md");
        assert_eq!(records[5].key, "kisiler/telegram:user:42.md");
    }

    #[test]
    fn import_rejects_all_canonical_collisions_before_writing() {
        for (first, second) in [
            ("kisiler/42.md", "kisiler/discord:user:42.md"),
            ("kisiler/42.md", "kisiler/042.md"),
            ("kanallar/7.md", "kanallar/discord:room:7.md"),
            ("profil.md", "profil.md"),
        ] {
            let records = [first, second]
                .into_iter()
                .map(|key| Record {
                    key: key.to_owned(),
                    content: "untouched".to_owned(),
                    modified: 22,
                })
                .collect();
            assert!(normalize_records(records).is_err());
        }
    }

    #[test]
    fn unreadable_source_fails_instead_of_importing_empty_content() {
        let root = std::env::temp_dir().join(format!(
            "nibby-invalid-import-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("profil.md"), [0xff, 0xfe]).unwrap();
        assert!(collect(&root, &root, "arsiv").is_err());
        assert!(collect(&root.join("missing"), &root, "arsiv").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
