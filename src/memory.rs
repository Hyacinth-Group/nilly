// Memory, the "second brain" logic:
//   - INDEX.md   the list of what it knows; sent with every reply (a pointer, not the data itself)
//   - kisiler/   one record per person; only people talking in this chat get pulled in
//   - konular/   one record per topic; pulled in by keyword match against the chat
//   - olaylar/   one line per finished chat, filed by month
//   - arsiv/     raw chunks summarized out of records that hit their limit (nothing is ever deleted)
// Once a file hits its limit, the summarizer agent (agents.rs) shrinks it. The context
// window doesn't grow: every reply gets the index plus whatever was retrieved for that
// chat, so the budget stays constant.
//
// Everything above except `arsiv/` lives in one embedded database, `durum/hafiza.redb`
// (redb — pure Rust, single file, ACID transactions; chosen over rusqlite specifically to
// avoid a C dependency, see docs/decisions.md). The design deliberately doesn't reshape the
// data: values retain their original markdown bytes through the atomic identity migration.
// Active person and room keys use canonical platform-qualified domain identities; unknown
// historical keys remain stored but are never treated as runtime identity aliases.
// `arsiv/` stays real files for human inspection, never read back by the bot. Every
// displaced editable record is additionally retained transactionally in memory_history,
// so archive filesystem failure cannot delete the original prose.

use super::*;
use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

mod revisions;
pub use revisions::{
    browse_versioned, read_versioned, refresh_index_for, replace_generated, replace_versioned,
    set_debug_mode, set_model_override, set_thinking_mode, update_record, MutationError,
    RecordSnapshot,
};

#[derive(Debug, serde::Serialize)]
pub struct RecordPage {
    pub records: Vec<RecordSnapshot>,
    pub next_cursor: Option<String>,
}

// path -> the exact text a file held before the redb migration
const CONTENT: TableDefinition<&str, &str> = TableDefinition::new("content");
// path -> unix seconds of last write; replaces filesystem mtime for "most recently changed" ordering
const MODIFIED: TableDefinition<&str, u64> = TableDefinition::new("modified");
const PANEL_ADMIN: TableDefinition<&str, &str> = TableDefinition::new("panel_admin");
const METADATA: TableDefinition<&str, u64> = TableDefinition::new("memory_metadata");

static DB: OnceLock<Database> = OnceLock::new();
static INIT_LOCK: Mutex<()> = Mutex::new(());
// arsiv/ is the one thing still written as real files (see module doc above); this guards
// just that narrow path — redb serializes its own writers internally, no lock needed there
static ARCHIVE_LOCK: Mutex<()> = Mutex::new(());

pub const PERSON_LIMIT: usize = 1800; // a person record gets summarized past this size
pub const PERSON_TARGET: usize = 1000; // target size after summarizing
pub const TOPIC_LIMIT: usize = 1500;
pub const TOPIC_TARGET: usize = 800;
pub const EVENT_LIMIT: usize = 6000; // a month's event record gets its older half summarized past this size
pub const CONTEXT_BUDGET: usize = 6000; // total characters of memory retrieved for one reply
pub const INDEX_PEOPLE: usize = 40; // max people shown in the index
pub const FAVORITE_NOTE: &str = "canın ciğerin, ne yaparsa yapsın arkasındasın";

// ---------- database open + primitives ----------

/// Reports whether initialization, migration, and index regeneration completed successfully.
pub fn initialized() -> bool {
    DB.get().is_some()
}

/// Opens the database exclusively, completes identity migration, and regenerates the
/// derived index before publishing the process-wide store. Access failures abort startup.
pub fn init_result(path: &Path) -> Result<(), BotError> {
    let _guard = INIT_LOCK
        .lock()
        .map_err(|_| "memory initialization lock poisoned")?;
    if initialized() {
        return Err("memory database is already initialized".into());
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(parent)?;
    }
    let database = open_private_database(path)?;
    let txn = database.begin_write()?;
    {
        txn.open_table(CONTENT)?;
        txn.open_table(MODIFIED)?;
        txn.open_table(PANEL_ADMIN)?;
        revisions::initialize(&txn)?;
    }
    txn.commit()?;
    crate::memory_migration::migrate_database(&database, path)?;
    refresh_index_database(&database)?;
    DB.set(database)
        .map_err(|_| -> BotError { "memory database is already initialized".into() })
}

/// Creates or opens an active database or backup without exposing its authentication data.
pub(crate) fn open_private_database(path: &Path) -> Result<Database, BotError> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(Database::builder().create_file(file)?)
}

/// The panel and bot always use this same process-wide handle.
pub(crate) fn db() -> &'static Database {
    DB.get().expect("memory::init_result wasn't called")
}

fn db_result() -> Result<&'static Database, BotError> {
    DB.get()
        .ok_or_else(|| "memory database is not initialized".into())
}

fn read_database(database: &Database, rel: &str) -> Result<Option<String>, BotError> {
    let txn = database.begin_read()?;
    let table = txn.open_table(CONTENT)?;
    Ok(table.get(rel)?.map(|value| value.value().to_string()))
}

/// Distinguishes a missing record from a database access failure.
pub fn read_result(rel: &str) -> Result<Option<String>, BotError> {
    read_database(db_result()?, rel)
}

/// Adapter cursors are metadata, not personality records or indexed transcripts.
pub(crate) fn read_metadata(key: &str) -> Result<Option<u64>, BotError> {
    let txn = db_result()?.begin_read()?;
    let table = txn.open_table(METADATA)?;
    Ok(table.get(key)?.map(|value| value.value()))
}
pub(crate) fn write_metadata(key: &str, value: u64) -> Result<(), BotError> {
    let txn = db_result()?.begin_write()?;
    txn.open_table(METADATA)?.insert(key, value)?;
    txn.commit()?;
    Ok(())
}

/// Runtime convenience read; missing records are empty and access failures are logged.
pub fn read(rel: &str) -> String {
    match read_result(rel) {
        Ok(content) => content.unwrap_or_default(),
        Err(error) => {
            log::error!("couldn't read {rel}: {error}");
            String::new()
        }
    }
}

/// Replaces a record and propagates database access failures.
pub fn write_result(rel: &str, content: &str) -> Result<(), BotError> {
    write_with_mtime_result(rel, content, now_unix() as u64)
}

/// Runtime convenience write; failures are logged.
pub fn write(rel: &str, content: &str) {
    if let Err(error) = write_result(rel, content) {
        log::error!("couldn't write {rel}: {error}");
    }
}

/// Stores a record with its source-file timestamp, propagating access failures.
pub fn write_with_mtime_result(rel: &str, content: &str, modified: u64) -> Result<(), BotError> {
    write_database(db_result()?, rel, content, modified)
}

fn write_database(
    database: &Database,
    rel: &str,
    content: &str,
    modified: u64,
) -> Result<(), BotError> {
    if editable_record(rel) {
        return revisions::write_coordinated(database, rel, content, modified);
    }
    let txn = database.begin_write()?;
    {
        txn.open_table(CONTENT)?.insert(rel, content)?;
        txn.open_table(MODIFIED)?.insert(rel, modified)?;
    }
    txn.commit()?;
    Ok(())
}

/// Counts records without treating database access failures as an empty database.
pub fn record_count_result() -> Result<u64, BotError> {
    let txn = db_result()?.begin_read()?;
    let table = txn.open_table(CONTENT)?;
    Ok(table.len()?)
}

fn editable_record(key: &str) -> bool {
    matches!(
        key,
        "huy.md" | "profil.md" | "kendim.md" | "duzeltmeler.md" | "gundem.md"
    ) || key.starts_with("kisiler/")
        || key.starts_with("konular/")
        || key.starts_with("olaylar/")
}

// a raw chunk dropped by summarizing goes to the archive; nothing is ever deleted. arsiv/
// stays real files on disk (see module doc) — write-only, human-inspection-only, never read
// back by the bot, so it's kept out of the transactional store on purpose.
pub fn archive(rel: &str, content: &str) {
    archive_append(
        &format!("arsiv/{}", platform::keys::archive_relative(rel)),
        &format!("\n## {} öncesi\n{}", date_time(), content.trim_end()),
    );
}

fn archive_append(rel: &str, line: &str) {
    if let Err(error) = archive_write_root(Path::new(STATE_DIR), rel, line) {
        log::error!("couldn't append archive {rel}: {error}");
    }
}

fn archive_write_root(root: &Path, rel: &str, line: &str) -> Result<(), BotError> {
    let _lock = ARCHIVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

/// Lowercase, Turkish-letter-simplified slug; defaults to "bilinmeyen" without alphanumerics.
pub fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.chars().flat_map(|c| c.to_lowercase()) {
        let c = match c {
            'ç' => 'c',
            'ğ' => 'g',
            'ı' => 'i',
            'ö' => 'o',
            'ş' => 's',
            'ü' => 'u',
            'â' => 'a',
            'î' => 'i',
            'û' => 'u',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            s.push(c);
        } else if !s.is_empty() && !s.ends_with('-') {
            s.push('-');
        }
    }
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() {
        "bilinmeyen".to_string()
    } else {
        s
    }
}

// YYYY-MM-DD, no external crate
/// Today's UTC date.
pub fn date() -> String {
    date_from_unix(now_unix())
}

// every record lands with a seconds-precision timestamp: YYYY-MM-DD HH:MM:SS
pub fn date_time() -> String {
    format!("{} {}", date(), crate::sleep::time_of_day())
}

/// Formats a Unix timestamp as YYYY-MM-DD in UTC using Howard Hinnant's
/// `civil_from_days` algorithm.
pub fn date_from_unix(unix: i64) -> String {
    let z = unix.div_euclid(86400) + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Current UTC month as YYYY-MM.
pub fn month() -> String {
    date()[..7].to_string()
}

// ---------- person record ----------

/// One `kisiler/<id>.md` record's parsed contents. Holds `id` (domain user key),
/// `name`/`username`/`previous_names` (identity), `score` (-10..10), `tags`, `note`,
/// `facts`/`events` (the two bulleted lists). Round-trips through `parse`/`text` below.
#[derive(Default, Clone)]
pub struct Person {
    pub id: UserKey,      // authoritative domain identity, not the markdown id field
    pub name: String,     // display name (global_name or username)
    pub username: String, // discord username
    pub previous_names: Vec<String>, // earlier display names
    pub score: i32,
    pub tags: Vec<String>,
    pub note: String,
    pub facts: Vec<String>,
    pub events: Vec<String>,
}

impl Person {
    /// Ignores unknown lines. Turkish field prefixes (`kullanici_adi:`, `eski_adlar:`,
    /// `puan:`, `etiket:`, `not:`) are persisted format keys, retained so existing data parses.
    pub fn parse(id: &UserKey, text: &str) -> Person {
        let mut p = Person {
            id: id.clone(),
            ..Default::default()
        };
        let mut section = "";
        for line in text.lines() {
            let s = line.trim();
            if section == "unknown" && !s.starts_with("## ") {
                continue;
            }
            if !section.is_empty() && !s.starts_with("## ") && !s.starts_with("- ") {
                continue;
            }
            if let Some(title) = s.strip_prefix("# ").filter(|_| section.is_empty()) {
                p.name = title.trim().to_string();
            } else if let Some(title) = s.strip_prefix("## ") {
                section = if title.starts_with("Bildik") {
                    "bilgi"
                } else if title.starts_with("Son") {
                    "olay"
                } else {
                    "unknown"
                };
            } else if s.starts_with("id:") {
                // The caller's record identity is authoritative, including for old numeric ids.
            } else if let Some(v) = s.strip_prefix("kullanici_adi:") {
                p.username = v.trim().to_string();
            } else if let Some(v) = s.strip_prefix("eski_adlar:") {
                p.previous_names = v
                    .split(',')
                    .map(|e| e.trim().to_string())
                    .filter(|e| !e.is_empty())
                    .collect();
            } else if let Some(v) = s.strip_prefix("puan:") {
                p.score = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = s.strip_prefix("etiket:") {
                p.tags = v
                    .split(',')
                    .map(|e| e.trim().to_string())
                    .filter(|e| !e.is_empty())
                    .collect();
            } else if let Some(v) = s.strip_prefix("not:") {
                p.note = v.trim().to_string();
            } else if let Some(item) = s.strip_prefix("- ") {
                match section {
                    "bilgi" => p.facts.push(item.to_string()),
                    "olay" => p.events.push(item.to_string()),
                    _ => {}
                }
            }
        }
        p
    }

    /// Serializes the persisted person-record format.
    pub fn text(&self) -> String {
        self.text_preserving("")
    }

    /// Keeps unknown text in its original header/subsection rather than reclassifying it.
    pub fn text_preserving(&self, original: &str) -> String {
        let mut output =
            format!(
            "# {}\nid: {}\nkullanici_adi: {}\neski_adlar: {}\npuan: {:+}\netiket: {}\nnot: {}\n",
            self.name, self.id, self.username, self.previous_names.join(", "),
            self.score, self.tags.join(", "), self.note
        );
        let append_items = |output: &mut String, items: &[String]| {
            for item in items {
                output.push_str("- ");
                output.push_str(item);
                output.push('\n');
            }
        };
        let mut section = "header";
        let mut seen_facts = false;
        let mut seen_events = false;
        for line in original.split_inclusive('\n') {
            let text = line.trim();
            if let Some(title) = text.strip_prefix("## ") {
                output.push_str(line);
                if title.starts_with("Bildik") {
                    section = "facts";
                    if !output.ends_with('\n') {
                        output.push('\n');
                    }
                    if !seen_facts {
                        append_items(&mut output, &self.facts);
                    }
                    seen_facts = true;
                } else if title.starts_with("Son") {
                    section = "events";
                    if !output.ends_with('\n') {
                        output.push('\n');
                    }
                    if !seen_events {
                        append_items(&mut output, &self.events);
                    }
                    seen_events = true;
                } else {
                    section = "unknown";
                }
            } else if section == "header"
                && (text.starts_with("# ")
                    || [
                        "id:",
                        "kullanici_adi:",
                        "eski_adlar:",
                        "puan:",
                        "etiket:",
                        "not:",
                    ]
                    .iter()
                    .any(|prefix| text.starts_with(prefix)))
            {
                // Header fields are regenerated from the current structured person.
            } else if matches!(section, "facts" | "events") && text.starts_with("- ") {
                // These lists were emitted from the latest structured person at their heading.
            } else {
                output.push_str(line);
            }
        }
        if !seen_facts {
            output.push_str("\n## Bildiklerin\n");
            append_items(&mut output, &self.facts);
        }
        if !seen_events {
            output.push_str("\n## Son olaylar\n");
            append_items(&mut output, &self.events);
        }
        output
    }
}

/// Applies person changes to the latest record, preserving unknown original fields/sections.
pub fn update_person<F>(
    bot: Option<&Bot>,
    id: &UserKey,
    update: F,
) -> Result<RecordSnapshot, BotError>
where
    F: FnOnce(&mut Person),
{
    update_record(bot, &platform::keys::person_record(id), |original| {
        Ok(person_updated_content(id, original, is_owner(id), update))
    })
}

fn person_updated_content<F>(id: &UserKey, original: &str, favorite: bool, update: F) -> String
where
    F: FnOnce(&mut Person),
{
    let mut person = Person::parse(id, original);
    update(&mut person);
    person.id = id.clone();
    person.score = person.score.clamp(-10, 10);
    person.tags.truncate(6);
    person.previous_names.truncate(5);
    if favorite {
        person.score = 10;
        person.note = FAVORITE_NOTE.to_owned();
    }
    person.text_preserving(original)
}

// ---------- topics and events ----------

/// Uses the topic name as the header and its slug as the record key; appends a dated note.
/// Preserves the latest body bytes, inserting only a missing newline before the new bullet.
pub fn add_topic(bot: Option<&Bot>, name: &str, note: &str) -> Result<(), BotError> {
    let rel = format!("konular/{}.md", slug(name));
    update_record(bot, &rel, |existing| {
        let mut content = existing.to_owned();
        if content.is_empty() {
            content.push_str(&format!("# {name}\netiket: \n\n"));
        } else if !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&format!("- {}: {}\n", date_time(), note.trim()));
        Ok(content)
    })?;
    Ok(())
}

/// Appends a timestamped channel/event entry to the current month's record.
/// Preserves the latest body bytes, inserting only a missing newline before the new bullet.
pub fn add_event(bot: Option<&Bot>, channel: &str, event: &str) -> Result<(), BotError> {
    update_record(bot, &format!("olaylar/{}.md", month()), |existing| {
        Ok(format!(
            "{}{}- {} #{}: {}\n",
            existing,
            if !existing.is_empty() && !existing.ends_with('\n') {
                "\n"
            } else {
                ""
            },
            date_time(),
            channel,
            event.trim()
        ))
    })?;
    Ok(())
}

/// Administrative readers propagate access failures instead of presenting empty memory.
pub fn person_summaries_result() -> Result<Vec<Person>, BotError> {
    person_summaries_database(db_result()?)
}

fn person_summaries_database(database: &Database) -> Result<Vec<Person>, BotError> {
    let mut people = Vec::new();
    for key in files_database(database, "kisiler")? {
        let Some(id) = platform::keys::parse_person_record(&key) else {
            continue;
        };
        let content = read_database(database, &key)?.unwrap_or_default();
        let person = Person::parse(&id, &content);
        if !person.name.is_empty() {
            people.push(person);
        }
    }
    Ok(people)
}

pub fn topic_summaries_result() -> Result<Vec<(String, String)>, BotError> {
    topic_summaries_database(db_result()?)
}

fn topic_summaries_database(database: &Database) -> Result<Vec<(String, String)>, BotError> {
    files_database(database, "konular")?
        .into_iter()
        .take(30)
        .map(|key| {
            let content = read_database(database, &key)?.unwrap_or_default();
            let latest = content
                .lines()
                .rev()
                .find(|line| line.starts_with("- "))
                .map(|line| line.trim_start_matches("- ").to_string())
                .unwrap_or_default();
            Ok((first_line(&content), latest))
        })
        .collect()
}

pub fn event_months_result(month_count: usize) -> Result<Vec<(String, Vec<String>)>, BotError> {
    event_months_database(db_result()?, month_count)
}

fn event_months_database(
    database: &Database,
    month_count: usize,
) -> Result<Vec<(String, Vec<String>)>, BotError> {
    files_database(database, "olaylar")?
        .into_iter()
        .take(month_count)
        .map(|key| {
            let content = read_database(database, &key)?.unwrap_or_default();
            let lines = content
                .lines()
                .filter(|line| line.starts_with("- "))
                .map(str::to_string)
                .collect();
            Ok((stem(&key).to_string(), lines))
        })
        .collect()
}

pub fn read_person_result(id: &UserKey) -> Result<Person, BotError> {
    let key = platform::keys::person_record(id);
    let content = read_result(&key)?.unwrap_or_default();
    Ok(Person::parse(id, &content))
}

// ---------- channel history ----------

// reads durum/kanallar/<id> records: (channel id, recent lines)
pub fn load_channel_history() -> Vec<(ConversationKey, VecDeque<String>)> {
    let mut list = Vec::new();
    let keys = match db_result().and_then(|database| files_database(database, "kanallar")) {
        Ok(keys) => keys,
        Err(error) => {
            log::error!("couldn't load channel history: {error}");
            return list;
        }
    };
    for key in keys {
        let Some(id) = platform::keys::parse_room_record(&key) else {
            continue;
        };
        let lines: VecDeque<String> = read(&key)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string())
            .collect();
        list.push((id, lines));
    }
    list
}

/// Companion metadata leaves old history text byte-for-byte unchanged and unattributed.
pub fn load_channel_observations() -> Vec<(ConversationKey, VecDeque<Option<UserRef>>)> {
    let keys = match db_result().and_then(|database| files_database(database, "kanal-kimlikleri")) {
        Ok(keys) => keys,
        Err(error) => {
            log::error!("couldn't load channel observations: {error}");
            return Vec::new();
        }
    };
    keys.into_iter()
        .filter_map(|key| {
            let canonical = key.replacen("kanal-kimlikleri/", "kanallar/", 1);
            let conversation = platform::keys::parse_room_record(&canonical)?;
            let users = serde_json::from_str(&read(&key)).ok()?;
            Some((conversation, users))
        })
        .collect()
}

/// History text and its companion identities share one commit, preventing attribution drift.
pub fn write_channel_history(
    conversation: &ConversationKey,
    text: &str,
    users: &VecDeque<Option<UserRef>>,
) {
    let result = (|| -> Result<(), BotError> {
        let metadata = serde_json::to_string(users)?;
        let history_key = platform::keys::room_record(conversation);
        let metadata_key = platform::keys::observation_record(conversation);
        let transaction = db_result()?.begin_write()?;
        {
            let mut content = transaction.open_table(CONTENT)?;
            let mut modified = transaction.open_table(MODIFIED)?;
            let now = now_unix() as u64;
            content.insert(history_key.as_str(), text)?;
            content.insert(metadata_key.as_str(), metadata.as_str())?;
            modified.insert(history_key.as_str(), now)?;
            modified.insert(metadata_key.as_str(), now)?;
        }
        transaction.commit()?;
        Ok(())
    })();
    if let Err(error) = result {
        log::error!("couldn't write conversation history {conversation}: {error}");
    }
}

// ---------- index ----------

fn files_database(database: &Database, folder: &str) -> Result<Vec<String>, BotError> {
    let prefix = format!("{folder}/");
    let txn = database.begin_read()?;
    let content = txn.open_table(CONTENT)?;
    let modified = txn.open_table(MODIFIED)?;
    let mut list: Vec<(u64, String)> = Vec::new();
    for entry in content.range(prefix.as_str()..)? {
        let (key, _) = entry?;
        let key = key.value();
        if !key.starts_with(prefix.as_str()) {
            break;
        }
        let when = modified.get(key)?.map(|value| value.value()).unwrap_or(0);
        list.push((when, key.to_string()));
    }
    list.sort_by_key(|(when, _)| std::cmp::Reverse(*when));
    Ok(list.into_iter().map(|(_, key)| key).collect())
}

fn stem(key: &str) -> &str {
    let name = key.rsplit('/').next().unwrap_or(key);
    name.strip_suffix(".md").unwrap_or(name)
}

fn first_line(content: &str) -> String {
    content
        .lines()
        .next()
        .unwrap_or("")
        .trim_start_matches("# ")
        .to_string()
}

// the index display only needs the header fields; parsed without building the
// facts/events Vecs (cheaper than Person::parse's full parse)
/// A cheaper partial parse of a person record: just `name`/`score`/`tags`/`note`, no
/// `facts`/`events`. Produced by `person_header` below; used only by `refresh_index`.
struct PersonHeader {
    name: String,
    score: i32,
    tags: Vec<String>,
    note: String,
}

/// Stops at the first subsection heading, avoiding the facts/events lists.
fn person_header(text: &str) -> PersonHeader {
    let mut header = PersonHeader {
        name: String::new(),
        score: 0,
        tags: Vec::new(),
        note: String::new(),
    };
    for line in text.lines() {
        let s = line.trim();
        if s.starts_with("## ") {
            break; // header is over, don't descend into the lists
        }
        if let Some(title) = s.strip_prefix("# ") {
            header.name = title.trim().to_string();
        } else if let Some(v) = s.strip_prefix("puan:") {
            header.score = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = s.strip_prefix("etiket:") {
            header.tags = v
                .split(',')
                .map(|e| e.trim().to_string())
                .filter(|e| !e.is_empty())
                .collect();
        } else if let Some(v) = s.strip_prefix("not:") {
            header.note = v.trim().to_string();
        }
    }
    header
}

// regenerates and returns INDEX.md: the "what I know" list sent with every reply
pub fn refresh_index() -> String {
    match refresh_index_result() {
        Ok(index) => index,
        Err(error) => {
            log::error!("couldn't refresh INDEX.md: {error}");
            read("INDEX.md")
        }
    }
}

/// Regenerates the derived index without swallowing database failures.
pub fn refresh_index_result() -> Result<String, BotError> {
    refresh_index_for(None)
}

fn refresh_index_database(database: &Database) -> Result<String, BotError> {
    let transaction = database.begin_write()?;
    let output = build_index_transaction(&transaction)?;
    transaction
        .open_table(CONTENT)?
        .insert("INDEX.md", output.as_str())?;
    transaction
        .open_table(MODIFIED)?
        .insert("INDEX.md", now_unix() as u64)?;
    transaction.commit()?;
    Ok(output)
}

fn build_index_transaction(transaction: &redb::WriteTransaction) -> Result<String, BotError> {
    use std::fmt::Write as _;
    let content_table = transaction.open_table(CONTENT)?;
    let modified = transaction.open_table(MODIFIED)?;
    let keys = |prefix: &str| -> Result<Vec<String>, BotError> {
        let mut keys = Vec::new();
        for entry in content_table.range(prefix..)? {
            let (key, _) = entry?;
            if !key.value().starts_with(prefix) {
                break;
            }
            let when = modified.get(key.value())?.map(|v| v.value()).unwrap_or(0);
            keys.push((when, key.value().to_owned()));
        }
        keys.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        Ok(keys.into_iter().map(|(_, key)| key).collect())
    };
    let mut output = String::from("## Kişiler\n");
    for key in keys("kisiler/")?
        .into_iter()
        .filter(|key| platform::keys::parse_person_record(key).is_some())
        .take(INDEX_PEOPLE)
    {
        let body = content_table.get(key.as_str())?;
        let header = person_header(body.as_ref().map(|value| value.value()).unwrap_or(""));
        if header.name.is_empty() {
            continue;
        }
        let _ = write!(output, "- {} ({:+})", header.name, header.score);
        if !header.tags.is_empty() {
            let _ = write!(output, " · {}", header.tags.join(", "));
        }
        if !header.note.is_empty() {
            let _ = write!(output, " · {}", header.note);
        }
        output.push('\n');
    }
    output.push_str("\n## Konular\n");
    for key in keys("konular/")?.into_iter().take(30) {
        let body = content_table.get(key.as_str())?;
        let content = body.as_ref().map(|value| value.value()).unwrap_or("");
        let name = content
            .lines()
            .next()
            .unwrap_or("")
            .trim_start_matches("# ");
        let latest = content
            .lines()
            .rev()
            .find(|l| l.starts_with("- "))
            .and_then(|l| l.get(2..12))
            .unwrap_or("");
        let _ = writeln!(output, "- {name} · son: {latest}");
    }
    output.push_str("\n## Olaylar\n");
    for key in keys("olaylar/")?.into_iter().take(3) {
        let body = content_table.get(key.as_str())?;
        let n = body
            .as_ref()
            .map(|value| value.value())
            .unwrap_or("")
            .lines()
            .filter(|l| l.starts_with("- "))
            .count();
        let _ = writeln!(output, "- {} · {n} kayıt", stem(&key));
    }
    drop(modified);
    drop(content_table);
    Ok(output)
}

// ---------- retrieval ----------

const STOPWORDS: &[&str] = &[
    "için", "gibi", "değil", "bence", "yani", "falan", "filan", "diye", "olan", "daha", "böyle",
    "şöyle", "nasıl", "neden", "niye", "sonra", "önce", "şimdi", "bugün", "yarın", "zaten", "hala",
    "hâlâ", "bile", "kadar", "biraz", "bayağı", "aynen", "tamam", "evet", "hayır", "olur", "oldu",
    "olsun", "yapsın", "yaptı", "bunu", "şunu", "onun", "bunun", "bana", "sana", "beni", "seni",
    "bizi", "sizi", "kendi", "hangi", "nerede", "burada", "orada", "http", "https", "that", "this",
    "with", "have", "what", "just", "like", "abi", "lan", "amk", "aga", "reis",
];

// search keywords from chat text: 4+ letters, common words filtered out
/// Caps the distinct keyword set at 40 lowercase words.
pub fn keywords(texts: &[String]) -> Vec<String> {
    let mut list: Vec<String> = Vec::new();
    for m in texts {
        for word in m.split(|c: char| !c.is_alphanumeric()) {
            let word = word.to_lowercase();
            if word.chars().count() >= 4
                && !STOPWORDS.contains(&word.as_str())
                && !list.contains(&word)
            {
                list.push(word);
            }
        }
    }
    list.truncate(40);
    list
}

/// Counts case-insensitive keyword substring matches.
fn score_matches(text: &str, keywords: &[String]) -> usize {
    let lower = text.to_lowercase();
    keywords
        .iter()
        .filter(|a| lower.contains(a.as_str()))
        .count()
}

// the result is ALWAYS at most `limit` characters (some Discord fields — like a select
// menu option's description — enforce this exact limit strictly); "…" is included in the
// count too, otherwise the trimmed result comes out to limit+1 characters (see
// docs/decisions.md — this caused a live "Must be 100 or fewer in length" error)
pub fn trim(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.trim().to_string()
    } else {
        let take_n = limit.saturating_sub(1);
        format!("{}…", text.chars().take(take_n).collect::<String>().trim())
    }
}

// what to retrieve for this chat: the speakers' records, topic records touching the
// subject, recent events, and old raw-memory lines matching a keyword. The budget is fixed.
/// Uses at most four participants, two matching topics, eight events from the current
/// month, and twelve older matching message lines. Skips `exclude_recent` history
/// lines because they already appear in the chat; sections share `CONTEXT_BUDGET`.
pub fn retrieve(
    participants: &[String],
    name_to_id: &std::collections::HashMap<String, UserKey>,
    keywords: &[String],
    history: &VecDeque<String>,
    exclude_recent: usize,
) -> Result<String, BotError> {
    let mut sections: Vec<String> = Vec::new();

    for name in participants.iter().take(4) {
        let Some(id) = name_to_id.get(&name.to_lowercase()) else {
            continue;
        };
        let content = read_result(&platform::keys::person_record(id))?.unwrap_or_default();
        if !content.is_empty() {
            sections.push(trim(&content, 1200));
        }
    }

    // content travels bundled with its score: the top two records aren't read a second time
    let mut topics: Vec<(usize, String)> = Vec::new();
    for key in files_database(db_result()?, "konular")?
        .into_iter()
        .take(30)
    {
        let content = read_result(&key)?.unwrap_or_default();
        let score = score_matches(&content, keywords);
        if score >= 1 {
            topics.push((score, content));
        }
    }
    topics.sort_by_key(|k| std::cmp::Reverse(k.0));
    for (_, content) in topics.into_iter().take(2) {
        sections.push(trim(&content, 800));
    }

    let events = read_result(&format!("olaylar/{}.md", month()))?.unwrap_or_default();
    let recent_events: Vec<&str> = events.lines().filter(|l| l.starts_with("- ")).collect();
    if !recent_events.is_empty() {
        let skip = recent_events.len().saturating_sub(8);
        sections.push(format!(
            "Son olaylar:\n{}",
            recent_events[skip..].join("\n")
        ));
    }

    // raw context window: old lines that aren't in the current chat but touch the topic
    if !keywords.is_empty() {
        let available = history.len().saturating_sub(exclude_recent);
        let mut matches: Vec<(usize, usize, &String)> = history
            .iter()
            .take(available)
            .enumerate()
            .map(|(i, line)| (score_matches(line, keywords), i, line))
            .filter(|(score, _, _)| *score >= 2)
            .collect();
        matches.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        matches.truncate(12);
        matches.sort_by_key(|e| e.1);
        if !matches.is_empty() {
            let lines: Vec<String> = matches.iter().map(|(_, _, line)| trim(line, 200)).collect();
            sections.push(format!(
                "Hafızadan, konuya değen eski mesajlar:\n{}",
                lines.join("\n")
            ));
        }
    }

    // budget: add sections in order, stop once full; size tracked with a counter so the
    // result isn't rescanned on every section (chars().count() would make this O(n²))
    let mut result = String::new();
    let mut size = 0usize;
    for section in sections {
        let section_len = section.chars().count();
        if size + section_len > CONTEXT_BUDGET {
            break;
        }
        if !result.is_empty() {
            result.push_str("\n\n");
            size += 2;
        }
        result.push_str(&section);
        size += section_len;
    }
    Ok(result)
}

// records over their limit: (kind, record key)
/// Compares record byte lengths against the person/topic/event limits;
/// only the current month's event record is considered.
pub fn over_limit() -> Result<Vec<(&'static str, String)>, BotError> {
    let mut list = Vec::new();
    for key in files_database(db_result()?, "kisiler")? {
        if platform::keys::parse_person_record(&key).is_none() {
            continue;
        }
        if read_result(&key)?.unwrap_or_default().len() > PERSON_LIMIT {
            list.push(("kisi", key));
        }
    }
    for key in files_database(db_result()?, "konular")? {
        if read_result(&key)?.unwrap_or_default().len() > TOPIC_LIMIT {
            list.push(("konu", key));
        }
    }
    let event_key = format!("olaylar/{}.md", month());
    if read_result(&event_key)?.unwrap_or_default().len() > EVENT_LIMIT {
        list.push(("olay", event_key));
    }
    Ok(list)
}

#[cfg(test)]
pub(crate) mod test {
    pub(crate) use super::revisions::historical_snapshot;
    use super::*;

    // every test in this module shares the process-wide `DB` static, so they all open the
    // same temp redb file (once, first test in) and use distinctly-prefixed keys to avoid
    // stepping on each other — mirrors the old suite's single `"test-gecici.md"` convention,
    // just extended to "the whole database is shared, not just one file". `Once` (not a
    // `DB.get().is_none()` check) matters here: cargo test runs these in parallel threads,
    // and a check-then-act race let two threads both call `Database::create` on the same
    // path at once, which redb's file locking turned into a spurious initialization
    // failure for whichever thread lost the race.
    static INIT: std::sync::Once = std::sync::Once::new();
    pub(crate) fn ensure_test_db() {
        INIT.call_once(|| {
            let path = std::env::temp_dir().join(format!("nibby-test-{}.redb", std::process::id()));
            let _ = fs::remove_file(&path);
            init_result(&path).expect("initialize isolated test memory database");
        });
    }

    /// Verifies `date_from_unix` against known unix timestamps, including a day boundary.
    #[test]
    fn date_is_correct() {
        assert_eq!(date_from_unix(0), "1970-01-01");
        assert_eq!(date_from_unix(1788220800), "2026-09-01");
        assert_eq!(date_from_unix(1788220799), "2026-08-31");
    }

    /// Verifies `trim` caps at the char limit with an ellipsis, and leaves short text untouched.
    #[test]
    fn trim_stays_within_limit() {
        // on a hard Discord limit like a select-menu description, going over `limit`
        // including the "…" got the request rejected (live "Must be 100 or fewer" error)
        let long_text = "a".repeat(200);
        let result = trim(&long_text, 100);
        assert_eq!(result.chars().count(), 100);
        assert!(result.ends_with('…'));
        // text that already fits is left untouched
        assert_eq!(trim("kısa", 100), "kısa");
        assert_eq!(trim(&"a".repeat(100), 100), "a".repeat(100));
    }

    /// Verifies `slug` transliterates Turkish characters and falls back to "bilinmeyen" when nothing survives.
    #[test]
    fn slug_handles_turkish() {
        assert_eq!(slug("Emin Şeyrek"), "emin-seyrek");
        assert_eq!(slug("LNG deniz altı"), "lng-deniz-alti");
        assert_eq!(slug("!!!"), "bilinmeyen");
    }

    /// Verifies `Person::text` / `Person::parse` are inverses (a person survives a write-then-read cycle).
    #[test]
    fn person_round_trips() {
        let p = Person {
            id: UserKey::discord(259669117248864257),
            name: "Emin".into(),
            username: "kaju".into(),
            previous_names: vec!["eski ad".into()],
            score: -3,
            tags: vec!["rust".into(), "oyun".into()],
            note: "laf soktu".into(),
            facts: vec!["yks'ye hazırlanıyor".into()],
            events: vec!["2026-09-01 14:03:22: tartıştık".into()],
        };
        let parsed = Person::parse(&p.id, &p.text());
        assert_eq!(parsed.id, p.id);
        assert_eq!(parsed.name, "Emin");
        assert_eq!(parsed.username, "kaju");
        assert_eq!(parsed.previous_names, p.previous_names);
        assert_eq!(parsed.score, -3);
        assert_eq!(parsed.tags, p.tags);
        assert_eq!(parsed.note, p.note);
        assert_eq!(parsed.facts, p.facts);
        assert_eq!(parsed.events, p.events);
    }

    /// Verifies `keywords` drops stopwords/short tokens and keeps the meaningful ones.
    #[test]
    fn keywords_filtered() {
        let result = keywords(&["rust ile bot yazdım abi, bence güzel oldu".to_string()]);
        assert_eq!(result, vec!["rust", "yazdım", "güzel"]);
    }

    /// Verifies `date_time`'s output shape (`YYYY-MM-DD HH:MM:SS`, 19 characters).
    #[test]
    fn date_time_format() {
        // YYYY-MM-DD HH:MM:SS (19 characters, seconds precision)
        let s = date_time();
        assert_eq!(s.chars().count(), 19);
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[10..11], " ");
        assert_eq!(&s[13..14], ":");
        assert_eq!(&s[16..17], ":");
    }

    /// Verifies `person_header` reads only the header fields and stops before the fact/event lists,
    /// and that an empty record parses to an empty header rather than panicking.
    #[test]
    fn person_header_stops_before_lists() {
        let text = "# Ali\nid: 5\npuan: +3\netiket: rust, oyun\nnot: yakın arkadaş\n\n## Bildiklerin\n- bilgi1\n- bilgi2\n\n## Son olaylar\n- olay1\n";
        let header = person_header(text);
        assert_eq!(header.name, "Ali");
        assert_eq!(header.score, 3);
        assert_eq!(header.tags, vec!["rust", "oyun"]);
        assert_eq!(header.note, "yakın arkadaş");
        // empty record: everything stays empty, no panic
        let empty = person_header("");
        assert!(empty.name.is_empty());
    }

    /// Verifies write/atomic update/read round-trip and Result-based prefix listing.
    #[test]
    fn write_append_read_round_trip() {
        ensure_test_db();
        let key = "test/gecici.md";
        write(key, "ilk\n");
        update_record(None, key, |current| Ok(format!("{current}ikinci\n"))).unwrap();
        assert_eq!(read(key), "ilk\nikinci\n");
        assert!(files_database(db(), "test")
            .unwrap()
            .contains(&key.to_string()));
    }

    /// Verifies `add_topic` writes a header on first use and only appends on later calls.
    #[test]
    fn add_topic_writes_header_once() {
        ensure_test_db();
        add_topic(None, "test konusu", "ilk not").unwrap();
        add_topic(None, "test konusu", "ikinci not").unwrap();
        let content = read("konular/test-konusu.md");
        assert_eq!(content.matches("# test konusu").count(), 1);
        assert!(content.contains("ilk not"));
        assert!(content.contains("ikinci not"));
    }

    /// Verifies the Result APIs preserve timestamps and distinguish missing records.
    #[test]
    fn write_with_mtime_round_trips() {
        ensure_test_db();
        let before = record_count_result().unwrap();
        write_with_mtime_result("test/mtime.md", "içerik", 12345).unwrap();
        assert_eq!(read("test/mtime.md"), "içerik");
        assert!(record_count_result().unwrap() > before);
        assert_eq!(read_result("test/mtime-missing.md").unwrap(), None);
        write_result("test/mtime-empty.md", "").unwrap();
        assert_eq!(
            read_result("test/mtime-empty.md").unwrap(),
            Some(String::new())
        );
        let txn = db().begin_read().unwrap();
        let modified = txn.open_table(MODIFIED).unwrap();
        assert_eq!(
            modified.get("test/mtime.md").unwrap().unwrap().value(),
            12345
        );
    }

    #[test]
    fn administrative_summaries_propagate_database_table_failures() {
        let path =
            std::env::temp_dir().join(format!("admin-summary-error-{}.redb", std::process::id()));
        let _ = fs::remove_file(&path);
        let database = Database::create(&path).unwrap();
        assert!(person_summaries_database(&database).is_err());
        assert!(topic_summaries_database(&database).is_err());
        assert!(event_months_database(&database, 3).is_err());
        drop(database);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn administrative_summaries_preserve_identity_order_and_limits() {
        let path =
            std::env::temp_dir().join(format!("admin-summary-fixture-{}.redb", std::process::id()));
        let _ = fs::remove_file(&path);
        let database = Database::create(&path).unwrap();
        let older = UserKey::discord(41);
        let newer: UserKey = "telegram:user:41".parse().unwrap();
        write_database(
            &database,
            &platform::keys::person_record(&older),
            "# older\npuan: +2\n",
            1,
        )
        .unwrap();
        write_database(
            &database,
            &platform::keys::person_record(&newer),
            "# newer\npuan: -2\n",
            2,
        )
        .unwrap();
        write_database(&database, "kisiler/41.md", "# unqualified\n", 3).unwrap();
        let people = person_summaries_database(&database).unwrap();
        assert_eq!(people.len(), 2);
        assert_eq!(people[0].id, newer);
        assert_eq!(people[1].id, older);
        for index in 0..31 {
            write_database(
                &database,
                &format!("konular/topic-{index}.md"),
                &format!("# topic-{index}\n- first\n- latest-{index}\n"),
                index,
            )
            .unwrap();
        }
        let topics = topic_summaries_database(&database).unwrap();
        assert_eq!(topics.len(), 30);
        assert_eq!(topics[0], ("topic-30".into(), "latest-30".into()));
        assert_eq!(topics[29], ("topic-1".into(), "latest-1".into()));
        for index in 1..=4 {
            write_database(
                &database,
                &format!("olaylar/2026-0{index}.md"),
                &format!("# month\nignored\n- entry-{index}\n"),
                index,
            )
            .unwrap();
        }
        let events = event_months_database(&database, 3).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], ("2026-04".into(), vec!["- entry-4".into()]));
        assert_eq!(events[2], ("2026-02".into(), vec!["- entry-2".into()]));
        drop(database);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn canonical_person_summaries_index_and_retrieve_preserve_identity_and_order() {
        ensure_test_db();
        let discord = UserKey::discord(8_800_000_000_000_000_041);
        let telegram: UserKey = "telegram:user:8800000000000000041".parse().unwrap();
        let discord_path = platform::keys::person_record(&discord);
        let telegram_path = platform::keys::person_record(&telegram);
        let old_body = "# canonical-discord-summary\nid: 41\npuan: +2\nnot: discord-only-fact\n";
        let telegram_body = "# canonical-telegram-summary\nid: discord:user:41\npuan: -2\nnot: telegram-only-fact\n";
        write_with_mtime_result(&discord_path, old_body, u64::MAX - 3).unwrap();
        write_with_mtime_result(&telegram_path, telegram_body, u64::MAX - 2).unwrap();
        write_with_mtime_result(
            "kisiler/8800000000000000041.md",
            "# numeric-runtime-alias-must-not-appear\n",
            u64::MAX,
        )
        .unwrap();
        write_with_mtime_result(
            "kisiler/canonical-summary-unknown-slug.md",
            "# unknown-runtime-alias-must-not-appear\n",
            u64::MAX - 1,
        )
        .unwrap();

        let summaries: Vec<Person> = person_summaries_result()
            .unwrap()
            .into_iter()
            .filter(|person| person.id == discord || person.id == telegram)
            .collect();
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].id, telegram);
        assert_eq!(summaries[1].id, discord);
        assert_eq!(read_person_result(&discord).unwrap().id, discord);
        assert_eq!(read_person_result(&telegram).unwrap().id, telegram);
        assert_eq!(
            read_result(&discord_path).unwrap().as_deref(),
            Some(old_body)
        );
        assert_eq!(
            read_result(&telegram_path).unwrap().as_deref(),
            Some(telegram_body)
        );

        let index = refresh_index_result().unwrap();
        assert!(index.contains("canonical-discord-summary"));
        assert!(index.contains("canonical-telegram-summary"));
        assert!(
            index.find("canonical-telegram-summary").unwrap()
                < index.find("canonical-discord-summary").unwrap()
        );
        assert!(!index.contains("numeric-runtime-alias-must-not-appear"));
        assert!(!index.contains("unknown-runtime-alias-must-not-appear"));
        let names = std::collections::HashMap::from([
            ("discord-speaker".to_string(), discord.clone()),
            ("telegram-speaker".to_string(), telegram.clone()),
        ]);
        let discord_context = retrieve(
            &["discord-speaker".into()],
            &names,
            &[],
            &VecDeque::new(),
            0,
        )
        .unwrap();
        assert!(discord_context.contains("discord-only-fact"));
        assert!(!discord_context.contains("telegram-only-fact"));
        let telegram_context = retrieve(
            &["telegram-speaker".into()],
            &names,
            &[],
            &VecDeque::new(),
            0,
        )
        .unwrap();
        assert!(telegram_context.contains("telegram-only-fact"));
        assert!(!telegram_context.contains("discord-only-fact"));

        update_person(None, &discord, |_| {}).unwrap();
        assert!(read_result(&discord_path)
            .unwrap()
            .unwrap()
            .contains("id: discord:user:8800000000000000041\n"));
    }

    #[test]
    fn canonical_room_history_uses_typed_keys_only() {
        ensure_test_db();
        let discord = ConversationKey {
            room: RoomKey::discord(8_800_000_000_000_000_042),
            thread: None,
        };
        let telegram = ConversationKey {
            room: "telegram:room:-8800000000000000042".parse().unwrap(),
            thread: None,
        };
        write_result(
            &platform::keys::room_record(&discord),
            "discord-first\n\ndiscord-second\n",
        )
        .unwrap();
        write_result(
            &platform::keys::room_record(&telegram),
            "telegram-first\ntelegram-second\n",
        )
        .unwrap();
        write_result("kanallar/8800000000000000042.md", "numeric-room-alias\n").unwrap();
        let histories = load_channel_history();
        assert_eq!(
            histories.iter().find(|(key, _)| key == &discord).unwrap().1,
            VecDeque::from(["discord-first".to_string(), "discord-second".to_string()])
        );
        assert_eq!(
            histories
                .iter()
                .find(|(key, _)| key == &telegram)
                .unwrap()
                .1,
            VecDeque::from(["telegram-first".to_string(), "telegram-second".to_string()])
        );
        assert!(!histories
            .iter()
            .any(|(_, lines)| lines.iter().any(|line| line == "numeric-room-alias")));
    }

    /// Verifies `retrieve` pulls in history lines matching the keywords/name and excludes unrelated ones.
    #[test]
    fn retrieve_pulls_from_memory() {
        ensure_test_db();
        let mut history = VecDeque::new();
        history.push_back("emin: rust derleme süresi çok uzun".to_string());
        history.push_back("lng: bugün hava güzel".to_string());
        history.push_back("emin: son mesaj, sohbette zaten var".to_string());
        let mut name_to_id = std::collections::HashMap::new();
        name_to_id.insert("emin".to_string(), UserKey::discord(1));
        let result = retrieve(
            &[],
            &name_to_id,
            &["rust".into(), "derleme".into()],
            &history,
            1,
        )
        .unwrap();
        assert!(result.contains("rust derleme süresi"));
        assert!(!result.contains("hava güzel"));
        assert!(!result.contains("son mesaj"));
    }
}
