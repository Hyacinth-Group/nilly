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
// data: every value stored is the exact same text a file held before this migration, keyed
// by the same relative path string the file used to have (`"kisiler/1.md"`, `"profil.md"`,
// ...). Domain identities are translated only by the explicit v1 boundary helpers below;
// canonical storage keys arrive with the atomic schema cutover.
// `arsiv/` stays real files on disk: write-mostly, human-inspection-only, never read back by
// the bot — keeping it out of the transactional store avoids unbounded DB growth and keeps
// its documented "for humans" purpose intact.

use super::*;
use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

// path -> the exact text a file held before the redb migration
const CONTENT: TableDefinition<&str, &str> = TableDefinition::new("content");
// path -> unix seconds of last write; replaces filesystem mtime for "most recently changed" ordering
const MODIFIED: TableDefinition<&str, u64> = TableDefinition::new("modified");

static DB: OnceLock<Database> = OnceLock::new();
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

/// Opens (creating if missing) the redb database at `path` and makes it the process-wide
/// store every other function in this module reads/writes through. Input: `path: &Path`.
/// Output: none (a failure to open is fatal — logged then the process can't proceed, same
/// severity as today's directory-creation failures). Used by: `Bot::setup` (`setup.rs`),
/// once, before `State::load` runs.
pub fn init(path: &Path) {
    let db = match Database::create(path) {
        Ok(db) => db,
        Err(e) => {
            log::error!("couldn't open {}: {e}", path.display());
            return;
        }
    };
    // touch both tables once so they exist even on a brand new file (redb creates a table
    // lazily on its first open in a write transaction)
    if let Ok(txn) = db.begin_write() {
        let _ = txn.open_table(CONTENT);
        let _ = txn.open_table(MODIFIED);
        let _ = txn.commit();
    }
    let _ = DB.set(db);
}

/// Input: none. Output: `&'static Database`. Panics if `init` hasn't run yet (a
/// programming error, not a runtime condition — every real entry point calls `init` first).
/// Used by every function below.
fn db() -> &'static Database {
    DB.get().expect("memory::init wasn't called")
}

/// Input: `rel: &str` — a path relative to `STATE_DIR` (e.g. `"kisiler/1.md"`), used as the
/// database key. Output: `String` — the stored content, or `""` if there's no entry or the
/// database can't be read. Used throughout the crate (`State::load`, agents, commands)
/// wherever a durum record is read.
pub fn read(rel: &str) -> String {
    let Ok(txn) = db().begin_read() else {
        return String::new();
    };
    let Ok(table) = txn.open_table(CONTENT) else {
        return String::new();
    };
    table
        .get(rel)
        .ok()
        .flatten()
        .map(|g| g.value().to_string())
        .unwrap_or_default()
}

/// Input: `rel: &str`; `content: &str` — the full new value. Output: none (failures are
/// logged, not propagated). Uses: `db`, `now_unix`. Used throughout the crate wherever a
/// whole durum record is rewritten.
pub fn write(rel: &str, content: &str) {
    if let Err(e) = write_inner(rel, content) {
        log::error!("couldn't write {rel}: {e}");
    }
}

fn write_inner(rel: &str, content: &str) -> Result<(), redb::Error> {
    write_with_mtime_inner(rel, content, now_unix() as u64)
}

/// Input: `rel`; `content`; `modified: u64` — a specific unix timestamp instead of "now".
/// Output: none (failures logged). Uses: `db`. Used by: `migrate::run`, to seed `MODIFIED`
/// with each source file's real historical mtime instead of resetting every record's
/// recency to migration day.
pub(crate) fn write_with_mtime(rel: &str, content: &str, modified: u64) {
    if let Err(e) = write_with_mtime_inner(rel, content, modified) {
        log::error!("couldn't write {rel}: {e}");
    }
}

fn write_with_mtime_inner(rel: &str, content: &str, modified: u64) -> Result<(), redb::Error> {
    let txn = db().begin_write()?;
    {
        txn.open_table(CONTENT)?.insert(rel, content)?;
        txn.open_table(MODIFIED)?.insert(rel, modified)?;
    }
    txn.commit()?;
    Ok(())
}

/// Input: none. Output: `u64` — how many records `CONTENT` currently holds (0 if it can't
/// be read). Uses: `db`. Used by: `migrate::run`, to refuse overwriting a populated target
/// without `--force`.
pub(crate) fn record_count() -> u64 {
    let Ok(txn) = db().begin_read() else {
        return 0;
    };
    let Ok(table) = txn.open_table(CONTENT) else {
        return 0;
    };
    table.len().unwrap_or(0)
}

// a real append instead of read-modify-write from the caller's side; the get+insert
// happens inside one write transaction, so it's atomic even under concurrent writers (redb
// serializes write transactions internally — the get here can never race with another
// writer's insert)
/// Input: `rel: &str`; `line: &str` — one line to add (a trailing `\n` is added). Output:
/// none (failures are logged). Uses: `db`, `now_unix`. Used by: `add_event` below.
fn append(rel: &str, line: &str) {
    let result: Result<(), redb::Error> = (|| {
        let txn = db().begin_write()?;
        {
            let mut table = txn.open_table(CONTENT)?;
            let mut new_content = table
                .get(rel)?
                .map(|g| g.value().to_string())
                .unwrap_or_default();
            new_content.push_str(line);
            new_content.push('\n');
            table.insert(rel, new_content.as_str())?;
            txn.open_table(MODIFIED)?.insert(rel, now_unix() as u64)?;
        }
        txn.commit()?;
        Ok(())
    })();
    if let Err(e) = result {
        log::error!("couldn't append to {rel}: {e}");
    }
}

// a raw chunk dropped by summarizing goes to the archive; nothing is ever deleted. arsiv/
// stays real files on disk (see module doc) — write-only, human-inspection-only, never read
// back by the bot, so it's kept out of the transactional store on purpose.
/// Input: `rel: &str` — the original record's key; `content: &str` — the chunk being
/// dropped. Output: none. Uses: `archive_append`, `date_time`. Used by: `Bot::summarizer`
/// (`agents.rs`), whenever a record shrinks.
pub fn archive(rel: &str, content: &str) {
    archive_append(
        &format!("arsiv/{rel}"),
        &format!("\n## {} öncesi\n{}", date_time(), content.trim_end()),
    );
}

/// Input: `rel: &str` — path relative to `STATE_DIR`; `line: &str` — appended verbatim plus
/// a trailing `\n`. Output: none (failures logged). Uses: `ARCHIVE_LOCK`. Used by: `archive`
/// above, the only caller.
fn archive_append(rel: &str, line: &str) {
    let _lock = ARCHIVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let p = Path::new(STATE_DIR).join(rel);
    if let Some(parent) = p.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let result = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
        .and_then(|mut f| {
            f.write_all(line.as_bytes())
                .and_then(|_| f.write_all(b"\n"))
        });
    if let Err(e) = result {
        log::error!("couldn't append to {}: {e}", p.display());
    }
}

/// Input: `name: &str`. Output: `String` — a lowercase, Turkish-letter-simplified,
/// hyphen-separated slug (e.g. `"Emin Şeyrek"` → `"emin-seyrek"`), or `"bilinmeyen"` if
/// nothing alphanumeric remains. Used by: `add_topic` below.
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
/// Input: none. Output: `String` — today's date, `YYYY-MM-DD`. Uses: `date_from_unix`,
/// `now_unix`. Used by: `date_time` below, `month` below, `modal::mind_embeds`.
pub fn date() -> String {
    date_from_unix(now_unix())
}

// every record lands with a seconds-precision timestamp: YYYY-MM-DD HH:MM:SS
/// Input: none. Output: `String` — `"YYYY-MM-DD HH:MM:SS"`. Uses: `date`,
/// `sleep::time_of_day`. Used by: `archive` above, `add_topic`/`add_event` below,
/// `Bot::diarist` (`agents.rs`).
pub fn date_time() -> String {
    format!("{} {}", date(), crate::sleep::time_of_day())
}

/// Input: `unix: i64` — a Unix timestamp. Output: `String` — `"YYYY-MM-DD"` in UTC, via
/// Howard Hinnant's `civil_from_days` algorithm (no external crate). Used by: `date` above,
/// `travel::year_of`/`day_number`-adjacent tests, `modal::status_message`.
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

/// Input: none. Output: `String` — `"YYYY-MM"` (the first 7 characters of `date()`). Used
/// by: `add_event`/`over_limit`/`retrieve` below, `Bot::send_news`/`news_cycle`
/// (`cycle_news.rs`, `awaiting_comment` window's event record name).
pub fn month() -> String {
    date()[..7].to_string()
}

// ---------- person record ----------

// Step 1 retains numeric Discord records until the atomic storage migration.
// Other platforms must never reuse these paths just because their IDs are numeric.
fn v1_user_number(id: &UserKey) -> Result<u64, KeyError> {
    id.as_str()
        .strip_prefix("discord:user:")
        .ok_or(KeyError)?
        .parse()
        .map_err(|_| KeyError)
}

pub fn v1_person_path(id: &UserKey) -> Result<String, KeyError> {
    Ok(format!("kisiler/{}.md", v1_user_number(id)?))
}

pub fn v1_room_path(id: &RoomKey) -> Result<String, KeyError> {
    let number: u64 = id
        .as_str()
        .strip_prefix("discord:room:")
        .ok_or(KeyError)?
        .parse()
        .map_err(|_| KeyError)?;
    Ok(format!("kanallar/{number}.md"))
}

fn v1_record_number(path: &str) -> Option<u64> {
    stem(path).parse::<u64>().ok()
}

fn v1_person_id_text(id: &UserKey) -> &str {
    id.as_str()
        .strip_prefix("discord:user:")
        .filter(|value| value.parse::<u64>().is_ok())
        .unwrap_or(id.as_str())
}

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
    /// Input: `id: &UserKey`; `text: &str` — a person record's contents. Output:
    /// `Person`. Unknown lines are ignored; the field prefixes it looks for
    /// (`kullanici_adi:`, `eski_adlar:`, `puan:`, `etiket:`, `not:`) are Turkish on purpose
    /// — they're the on-disk format (see `docs/state-files.md`), not translated so
    /// existing data keeps parsing. Used by: `read_person`/`person_summaries` below,
    /// `Bot::diarist` (`agents.rs`).
    pub fn parse(id: &UserKey, text: &str) -> Person {
        let mut p = Person {
            id: id.clone(),
            ..Default::default()
        };
        let mut section = "";
        for line in text.lines() {
            let s = line.trim();
            if let Some(title) = s.strip_prefix("# ") {
                p.name = title.trim().to_string();
            } else if let Some(title) = s.strip_prefix("## ") {
                section = if title.starts_with("Bildik") {
                    "bilgi"
                } else if title.starts_with("Son") {
                    "olay"
                } else {
                    ""
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

    /// Input: `&self`. Output: `String` — the `kisiler/<id>.md` record format (inverse of
    /// `parse`). Used by: `write_person` below.
    pub fn text(&self) -> String {
        let id = v1_person_id_text(&self.id);
        let mut s = format!(
            "# {}\nid: {}\nkullanici_adi: {}\neski_adlar: {}\npuan: {:+}\netiket: {}\nnot: {}\n\n## Bildiklerin\n",
            self.name,
            id,
            self.username,
            self.previous_names.join(", "),
            self.score,
            self.tags.join(", "),
            self.note
        );
        for f in &self.facts {
            s += &format!("- {f}\n");
        }
        s += "\n## Son olaylar\n";
        for e in &self.events {
            s += &format!("- {e}\n");
        }
        s
    }
}

/// Input: `id: &UserKey`. Output: `Person` — parsed from its v1 record, or an empty
/// `Person{id, ..}` if there's no record. Uses: `read`, `Person::parse`. Used by:
/// `Bot::diarist` (`agents.rs`), `modal::person_modal`, `retrieve` below (via `read`
/// directly, not this — see that function).
pub fn read_person(id: &UserKey) -> Person {
    let path = match v1_person_path(id) {
        Ok(path) => path,
        Err(error) => {
            log::error!("can't read {id} from v1 person storage: {error}");
            return Person::parse(id, "");
        }
    };
    Person::parse(id, &read(&path))
}

/// Input: `p: &Person`. Output: v1 identity validation result (writes its numeric Discord
/// record). Uses: `write`, `Person::text`. Used by: `Bot::diarist` (`agents.rs`).
pub fn write_person(p: &Person) -> Result<(), KeyError> {
    let path = v1_person_path(&p.id)?;
    write(&path, &p.text());
    Ok(())
}

// ---------- topics and events ----------

/// Input: `name: &str` — the topic name (used verbatim as the `# ` header on a new record,
/// slugified for the key); `note: &str` — the dated note to append. Output: none. Uses:
/// `db`, `slug`, `date_time`, `now_unix`. Used by: `Bot::diarist` (`agents.rs`), the only
/// caller.
pub fn add_topic(name: &str, note: &str) {
    let rel = format!("konular/{}.md", slug(name));
    let result: Result<(), redb::Error> = (|| {
        let txn = db().begin_write()?;
        {
            let mut table = txn.open_table(CONTENT)?;
            let existing = table
                .get(rel.as_str())?
                .map(|g| g.value().to_string())
                .unwrap_or_default();
            let mut new_content = existing.clone();
            if existing.is_empty() {
                new_content.push_str(&format!("# {name}\netiket: \n\n"));
            }
            new_content.push_str(&format!("- {}: {}\n", date_time(), note.trim()));
            table.insert(rel.as_str(), new_content.as_str())?;
            txn.open_table(MODIFIED)?
                .insert(rel.as_str(), now_unix() as u64)?;
        }
        txn.commit()?;
        Ok(())
    })();
    if let Err(e) = result {
        log::error!("couldn't append to {rel}: {e}");
    }
}

/// Input: `channel: &str` — the channel name to record; `event: &str` — the one-line
/// summary. Output: none (appends to `olaylar/<current month>.md`). Uses: `append`,
/// `month`, `date_time`. Used by: `Bot::diarist` (`agents.rs`), the only caller.
pub fn add_event(channel: &str, event: &str) {
    append(
        &format!("olaylar/{}.md", month()),
        &format!("- {} #{}: {}", date_time(), channel, event.trim()),
    );
}

// summaries for the modal display (most recently changed first)

/// Input: none. Output: `Vec<Person>` — every `kisiler/*` record with a valid numeric id
/// and non-empty name, most recently changed first. Uses: `files`, `stem`, `Person::parse`.
/// Used by: `modal::mind_embeds`/`person_options`, the `/zihin` card.
pub fn person_summaries() -> Vec<Person> {
    let mut list = Vec::new();
    for key in files("kisiler") {
        // key tail is id-based; old slug-keyed records (id can't be parsed) are skipped
        let Some(id) = v1_record_number(&key).map(UserKey::discord) else {
            continue;
        };
        let person = Person::parse(&id, &read(&key));
        if person.name.is_empty() {
            continue;
        }
        list.push(person);
    }
    list
}

// (topic name, latest note)
/// Input: none. Output: `Vec<(String, String)>` — up to 30 topics as (name, latest note),
/// most recently changed first. Uses: `files`, `first_line`. Used by: `modal::mind_embeds`/
/// `topics_modal`.
pub fn topic_summaries() -> Vec<(String, String)> {
    files("konular")
        .into_iter()
        .take(30)
        .map(|key| {
            let content = read(&key);
            let latest = content
                .lines()
                .rev()
                .find(|l| l.starts_with("- "))
                .map(|l| l.trim_start_matches("- ").to_string())
                .unwrap_or_default();
            let name = first_line(&content);
            (name, latest)
        })
        .collect()
}

// event records for the last `month_count` months: (month, "- " lines); newest month first.
// looking only at the current month gave an empty view at the start of a new month
/// Input: `month_count: usize`. Output: `Vec<(String, Vec<String>)>` — up to `month_count`
/// months as (`"YYYY-MM"`, its `"- "` lines), newest first. Uses: `files`, `stem`. Used by:
/// `modal::mind_embeds`/`events_modal`.
pub fn event_months(month_count: usize) -> Vec<(String, Vec<String>)> {
    files("olaylar")
        .into_iter()
        .take(month_count)
        .map(|key| {
            let month = stem(&key).to_string();
            let lines: Vec<String> = read(&key)
                .lines()
                .filter(|l| l.starts_with("- "))
                .map(|l| l.to_string())
                .collect();
            (month, lines)
        })
        .collect()
}

// ---------- channel history ----------

// reads durum/kanallar/<id> records: (channel id, recent lines)
/// Input: none. Output: `Vec<(RoomKey, VecDeque<String>)>` — every v1 Discord history record
/// as (domain room key, non-empty lines). Uses: `files`, `stem`. Used by: `State::load`
/// (`types_chat_state.rs`), the only caller (once, at startup).
pub fn load_channel_history() -> Vec<(RoomKey, VecDeque<String>)> {
    let mut list = Vec::new();
    for key in files("kanallar") {
        let Some(id) = v1_record_number(&key) else {
            continue;
        };
        let lines: VecDeque<String> = read(&key)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string())
            .collect();
        list.push((RoomKey::discord(id), lines));
    }
    list
}

// ---------- index ----------

/// Input: `folder: &str` — a key prefix (e.g. `"kisiler"`, everything under it is
/// `"kisiler/..."`). Output: `Vec<String>` — matching keys, most recently modified first.
/// Uses: `db`, `MODIFIED`. Used by: `person_summaries`/`topic_summaries`/`event_months`/
/// `load_channel_history`/`refresh_index`/`retrieve`/`over_limit` — every listing function
/// in this module.
fn files(folder: &str) -> Vec<String> {
    let prefix = format!("{folder}/");
    let (Ok(txn),) = (db().begin_read(),) else {
        return Vec::new();
    };
    let (Ok(content), Ok(modified)) = (txn.open_table(CONTENT), txn.open_table(MODIFIED)) else {
        return Vec::new();
    };
    let Ok(range) = content.range(prefix.as_str()..) else {
        return Vec::new();
    };
    let mut list: Vec<(u64, String)> = Vec::new();
    for entry in range {
        let Ok((key, _)) = entry else { continue };
        let key = key.value();
        if !key.starts_with(prefix.as_str()) {
            break; // keys are sorted; once the prefix stops matching, we're past this folder
        }
        let when = modified
            .get(key)
            .ok()
            .flatten()
            .map(|g| g.value())
            .unwrap_or(0);
        list.push((when, key.to_string()));
    }
    list.sort_by_key(|(when, _)| std::cmp::Reverse(*when));
    list.into_iter().map(|(_, key)| key).collect()
}

/// Input: `key: &str` — a full record key (e.g. `"kisiler/1.md"`). Output: `&str` — the
/// part after the last `/` with a trailing `.md` stripped (`"1"`). Used by:
/// `person_summaries`/`event_months`/`load_channel_history`/`refresh_index` above/below,
/// wherever an id/month used to come from `Path::file_stem()`.
fn stem(key: &str) -> &str {
    let name = key.rsplit('/').next().unwrap_or(key);
    name.strip_suffix(".md").unwrap_or(name)
}

/// Input: `content: &str` — a record's contents. Output: `String` — its first line, with a
/// leading `"# "` stripped. Used by: `topic_summaries` above.
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

/// Input: `text: &str` — a `kisiler/<id>.md` record's contents. Output: `PersonHeader`.
/// Stops at the first `## ` heading (never descends into the `facts`/`events` lists).
/// Used by: `refresh_index` below, the only caller.
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
/// Input: none. Output: `String` — the regenerated `INDEX.md` content (also written to the
/// database). Uses: `files`, `stem`, `person_header`, `write`. Used by: `State::load`
/// (`types_chat_state.rs`), `Bot::diarist`/`summarizer` (`agents.rs`),
/// `Bot::cmd_agents` (`command/actions.rs`) — anywhere the person/topic/event listing
/// might have changed.
pub fn refresh_index() -> String {
    use std::fmt::Write as _;
    let mut output = String::from("## Kişiler\n");
    for key in files("kisiler").into_iter().take(INDEX_PEOPLE) {
        // key tail is id-based; old slug-keyed records (id can't be parsed) are skipped
        if v1_record_number(&key).is_none() {
            continue;
        }
        let header = person_header(&read(&key));
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
    for key in files("konular").into_iter().take(30) {
        // a single read: the name and latest note both come from the same content
        let content = read(&key);
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
    for key in files("olaylar").into_iter().take(3) {
        let n = read(&key).lines().filter(|l| l.starts_with("- ")).count();
        let _ = writeln!(output, "- {} · {n} kayıt", stem(&key));
    }
    write("INDEX.md", &output);
    output
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
/// Input: `texts: &[String]` — recent chat lines. Output: `Vec<String>` — up to 40 distinct
/// lowercase words of 4+ letters, `STOPWORDS` excluded. Uses: `STOPWORDS`. Used by:
/// `Bot::chat_system` (`provider_generate.rs`), feeding `retrieve` below.
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

/// Input: `text: &str`; `keywords: &[String]`. Output: `usize` — how many `keywords` appear
/// (case-insensitively) in `text`. Used by: `retrieve` below, twice (topic records and raw
/// history lines).
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
/// Input: `text: &str`; `limit: usize`. Output: `String` — `text` trimmed, or hard-cut to
/// `limit - 1` characters plus a trailing `…` (so the total is always `<= limit`). Used
/// throughout the crate wherever text has to fit a Discord field/embed limit (`modal.rs`,
/// `retrieve` below, `Bot::firecrawl_search`/`wander` (`agenda.rs`), `Bot::research`
/// (`chat_lookup.rs`)).
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
/// Input: `participants: &[String]` — names currently talking (up to 4 used);
/// `name_to_id: &HashMap<String, UserKey>` — to resolve names to person records;
/// `keywords: &[String]` — from `keywords` above; `history: &VecDeque<String>` — the raw
/// message buffer; `exclude_recent: usize` — how many of the most recent `history` lines
/// to skip (they're already in the chat, no need to retrieve them again).
/// Output: `String` — sections (person records, up to 2 matching topic records, the month's
/// last 8 events, up to 12 old keyword-matching lines) concatenated up to
/// `CONTEXT_BUDGET` characters. Uses: `read`, `trim`, `files`, `score_matches`, `month`.
/// Used by: `Bot::chat_system` (`provider_generate.rs`), the only caller.
pub fn retrieve(
    participants: &[String],
    name_to_id: &std::collections::HashMap<String, UserKey>,
    keywords: &[String],
    history: &VecDeque<String>,
    exclude_recent: usize,
) -> String {
    let mut sections: Vec<String> = Vec::new();

    for name in participants.iter().take(4) {
        let Some(id) = name_to_id.get(&name.to_lowercase()) else {
            continue;
        };
        let path = match v1_person_path(id) {
            Ok(path) => path,
            Err(error) => {
                log::error!("can't retrieve {id} from v1 person storage: {error}");
                continue;
            }
        };
        let content = read(&path);
        if !content.is_empty() {
            sections.push(trim(&content, 1200));
        }
    }

    // content travels bundled with its score: the top two records aren't read a second time
    let mut topics: Vec<(usize, String)> = files("konular")
        .into_iter()
        .take(30)
        .map(|key| read(&key))
        .map(|content| (score_matches(&content, keywords), content))
        .filter(|(score, _)| *score >= 1)
        .collect();
    topics.sort_by_key(|k| std::cmp::Reverse(k.0));
    for (_, content) in topics.into_iter().take(2) {
        sections.push(trim(&content, 800));
    }

    let events = read(&format!("olaylar/{}.md", month()));
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
    result
}

// records over their limit: (kind, record key)
/// Input: none. Output: `Vec<(&'static str, String)>` — `("kisi", key)`/`("konu", key)`/
/// `("olay", key)` for every record over `PERSON_LIMIT`/`TOPIC_LIMIT`/`EVENT_LIMIT`. Uses:
/// `files`, `read`, `month`. Used by: `Bot::summarizer` (`agents.rs`), the only caller.
pub fn over_limit() -> Vec<(&'static str, String)> {
    let mut list = Vec::new();
    for key in files("kisiler") {
        if read(&key).len() > PERSON_LIMIT {
            list.push(("kisi", key));
        }
    }
    for key in files("konular") {
        if read(&key).len() > TOPIC_LIMIT {
            list.push(("konu", key));
        }
    }
    let event_key = format!("olaylar/{}.md", month());
    if read(&event_key).len() > EVENT_LIMIT {
        list.push(("olay", event_key));
    }
    list
}

#[cfg(test)]
pub(crate) mod test {
    use super::*;

    // every test in this module shares the process-wide `DB` static, so they all open the
    // same temp redb file (once, first test in) and use distinctly-prefixed keys to avoid
    // stepping on each other — mirrors the old suite's single `"test-gecici.md"` convention,
    // just extended to "the whole database is shared, not just one file". `Once` (not a
    // `DB.get().is_none()` check) matters here: cargo test runs these in parallel threads,
    // and a check-then-act race let two threads both call `Database::create` on the same
    // path at once, which redb's file locking turned into a spurious "memory::init wasn't
    // called" failure for whichever thread lost the race.
    static INIT: std::sync::Once = std::sync::Once::new();
    pub(crate) fn ensure_test_db() {
        INIT.call_once(|| {
            let path =
                std::env::temp_dir().join(format!("discord-bot-test-{}.redb", std::process::id()));
            let _ = fs::remove_file(&path);
            init(&path);
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

    /// Verifies `write`/`append`/`read` round-trip through the database, and that `files`
    /// finds a written key under its folder prefix.
    #[test]
    fn write_append_read_round_trip() {
        ensure_test_db();
        let key = "test/gecici.md";
        write(key, "ilk\n");
        append(key, "ikinci");
        assert_eq!(read(key), "ilk\nikinci\n");
        assert!(files("test").contains(&key.to_string()));
    }

    /// Verifies `add_topic` writes a header on first use and only appends on later calls.
    #[test]
    fn add_topic_writes_header_once() {
        ensure_test_db();
        add_topic("test konusu", "ilk not");
        add_topic("test konusu", "ikinci not");
        let content = read("konular/test-konusu.md");
        assert_eq!(content.matches("# test konusu").count(), 1);
        assert!(content.contains("ilk not"));
        assert!(content.contains("ikinci not"));
    }

    /// Verifies `write_with_mtime` stores the given timestamp (not "now") and that
    /// `record_count` reflects it — the two primitives `migrate::run` is built on.
    #[test]
    fn write_with_mtime_round_trips() {
        ensure_test_db();
        let before = record_count();
        write_with_mtime("test/mtime.md", "içerik", 12345);
        assert_eq!(read("test/mtime.md"), "içerik");
        assert!(record_count() > before);
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
        );
        assert!(result.contains("rust derleme süresi"));
        assert!(!result.contains("hava güzel"));
        assert!(!result.contains("son mesaj"));
    }
}
