use super::*;
use redb::WriteTransaction;

const REVISIONS: TableDefinition<&str, u64> = TableDefinition::new("memory_revisions");
const HISTORY: TableDefinition<&str, &str> = TableDefinition::new("memory_history");
static MUTATIONS: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RecordSnapshot {
    pub key: String,
    pub content: String,
    pub revision: u64,
    pub modified: u64,
}
#[derive(Debug)]
pub enum MutationError {
    Conflict(RecordSnapshot),
    Missing,
    Storage(BotError),
}
impl From<BotError> for MutationError {
    fn from(error: BotError) -> Self {
        Self::Storage(error)
    }
}
impl std::fmt::Display for MutationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict(_) => {
                f.write_str("memory revision changed; recompute from current memory")
            }
            Self::Missing => f.write_str("memory record is missing"),
            Self::Storage(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for MutationError {}

pub(super) fn initialize(txn: &WriteTransaction) -> Result<(), BotError> {
    txn.open_table(REVISIONS)?;
    txn.open_table(HISTORY)?;
    Ok(())
}

fn snapshot(
    key: &str,
    content: &impl ReadableTable<&'static str, &'static str>,
    revisions: &impl ReadableTable<&'static str, u64>,
    modified: &impl ReadableTable<&'static str, u64>,
) -> Result<Option<RecordSnapshot>, BotError> {
    let Some(body) = content.get(key)? else {
        return Ok(None);
    };
    Ok(Some(RecordSnapshot {
        key: key.to_owned(),
        content: body.value().to_owned(),
        revision: revisions.get(key)?.map(|v| v.value()).unwrap_or(0),
        modified: modified.get(key)?.map(|v| v.value()).unwrap_or(0),
    }))
}

pub fn read_versioned(key: &str) -> Result<Option<RecordSnapshot>, BotError> {
    read_database_versioned(db_result()?, key)
}
fn read_database_versioned(
    database: &Database,
    key: &str,
) -> Result<Option<RecordSnapshot>, BotError> {
    let txn = database.begin_read()?;
    snapshot(
        key,
        &txn.open_table(CONTENT)?,
        &txn.open_table(REVISIONS)?,
        &txn.open_table(MODIFIED)?,
    )
}

pub fn browse_versioned(
    prefix: &str,
    query: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<RecordPage, BotError> {
    browse_database(db_result()?, prefix, query, cursor, limit)
}
fn browse_database(
    database: &Database,
    prefix: &str,
    query: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<RecordPage, BotError> {
    if !matches!(prefix, "kisiler/" | "konular/" | "olaylar/")
        || query.chars().count() > 200
        || !(1..=100).contains(&limit)
    {
        return Err("invalid memory browse bounds".into());
    }
    let after: Option<(u64, String)> = cursor
        .map(|value| -> Result<_, BotError> {
            if value.len() > 2048 {
                return Err("invalid memory cursor".into());
            }
            let pair: (u64, String) = serde_json::from_str(value)?;
            if !pair.1.starts_with(prefix) {
                return Err("memory cursor prefix mismatch".into());
            }
            Ok(pair)
        })
        .transpose()?;
    let query = query.to_lowercase();
    let txn = database.begin_read()?;
    let content = txn.open_table(CONTENT)?;
    let modified = txn.open_table(MODIFIED)?;
    let revisions = txn.open_table(REVISIONS)?;
    // Keep only the next bounded page, never copy the entire database's bodies.
    let mut records = Vec::with_capacity(limit + 1);
    for row in content.range(prefix..)? {
        let (key, body) = row?;
        let key = key.value();
        if !key.starts_with(prefix) {
            break;
        }
        if prefix == "kisiler/" && platform::keys::parse_person_record(key).is_none() {
            continue;
        }
        let when = modified.get(key)?.map(|v| v.value()).unwrap_or(0);
        if let Some((timestamp, last_key)) = &after {
            if when > *timestamp || (when == *timestamp && key <= last_key.as_str()) {
                continue;
            }
        }
        if !query.is_empty()
            && !key.to_lowercase().contains(&query)
            && !body.value().to_lowercase().contains(&query)
        {
            continue;
        }
        let position = records.partition_point(|item: &RecordSnapshot| {
            item.modified > when || (item.modified == when && item.key.as_str() < key)
        });
        if position > limit {
            continue;
        }
        if records.len() == limit + 1 {
            records.pop();
        }
        records.insert(
            position,
            RecordSnapshot {
                key: key.to_owned(),
                content: body.value().to_owned(),
                modified: when,
                revision: revisions.get(key)?.map(|v| v.value()).unwrap_or(0),
            },
        );
    }
    let next_cursor = if records.len() > limit {
        records.pop();
        let last = records.last().expect("nonzero page bound");
        Some(serde_json::to_string(&(last.modified, &last.key))?)
    } else {
        None
    };
    Ok(RecordPage {
        records,
        next_cursor,
    })
}

pub fn replace_versioned(
    bot: Option<&Bot>,
    key: &str,
    expected: u64,
    content: &str,
) -> Result<RecordSnapshot, MutationError> {
    coordinated(
        db_result()?,
        bot,
        key,
        Some(expected),
        key == "huy.md" && expected == 0,
        now_unix() as u64,
        |_| Ok(content.to_owned()),
    )
}
/// Generated prose may create an absent record, but never replaces a changed snapshot.
pub fn replace_generated(
    bot: &Bot,
    key: &str,
    expected: u64,
    content: &str,
) -> Result<RecordSnapshot, MutationError> {
    coordinated(
        db_result()?,
        Some(bot),
        key,
        Some(expected),
        true,
        now_unix() as u64,
        |_| Ok(content.to_owned()),
    )
}
/// Applies a delta to the latest transaction value. Call without holding any Bot state/cache lock.
pub fn update_record<F>(bot: Option<&Bot>, key: &str, update: F) -> Result<RecordSnapshot, BotError>
where
    F: FnOnce(&str) -> Result<String, BotError>,
{
    coordinated(
        db_result()?,
        bot,
        key,
        None,
        true,
        now_unix() as u64,
        update,
    )
    .map_err(|e| Box::new(e) as BotError)
}

fn coordinated<F>(
    database: &Database,
    bot: Option<&Bot>,
    key: &str,
    expected: Option<u64>,
    create: bool,
    when: u64,
    update: F,
) -> Result<RecordSnapshot, MutationError>
where
    F: FnOnce(&str) -> Result<String, BotError>,
{
    let _gate = MUTATIONS
        .lock()
        .map_err(|_| MutationError::Storage("memory mutation lock poisoned".into()))?;
    mutate_database(
        database,
        key,
        expected,
        create,
        when,
        update,
        |record, index| {
            if let Some(bot) = bot {
                publish_record(bot, record, index);
            }
        },
    )
}

fn publish_record(bot: &Bot, record: &RecordSnapshot, index: String) {
    let mut state = bot.state();
    state.index = index;
    match record.key.as_str() {
        "huy.md" => state.temperament = record.content.clone(),
        "profil.md" => state.profile = record.content.clone(),
        "duzeltmeler.md" => state.corrections = record.content.clone(),
        "kendim.md" => state.myself = record.content.clone(),
        "gundem.md" => state.agenda = crate::agenda::latest_agenda(&record.content),
        _ => {}
    }
}

fn mutate_database<F, P>(
    database: &Database,
    key: &str,
    expected: Option<u64>,
    create: bool,
    when: u64,
    update: F,
    publish: P,
) -> Result<RecordSnapshot, MutationError>
where
    F: FnOnce(&str) -> Result<String, BotError>,
    P: FnOnce(&RecordSnapshot, String),
{
    mutate_database_commit(
        database,
        MutationSpec {
            key,
            expected,
            create,
            when,
        },
        update,
        publish,
        |txn| txn.commit().map_err(|error| error.into()),
    )
}

struct MutationSpec<'a> {
    key: &'a str,
    expected: Option<u64>,
    create: bool,
    when: u64,
}

fn mutate_database_commit<F, P, C>(
    database: &Database,
    spec: MutationSpec<'_>,
    update: F,
    publish: P,
    commit: C,
) -> Result<RecordSnapshot, MutationError>
where
    F: FnOnce(&str) -> Result<String, BotError>,
    P: FnOnce(&RecordSnapshot, String),
    C: FnOnce(WriteTransaction) -> Result<(), BotError>,
{
    let MutationSpec {
        key,
        expected,
        create,
        when,
    } = spec;
    let work = || -> Result<_, MutationError> {
        let txn = database
            .begin_write()
            .map_err(|e| MutationError::Storage(e.into()))?;
        initialize(&txn)?;
        let previous = snapshot(
            key,
            &txn.open_table(CONTENT)
                .map_err(|e| MutationError::Storage(e.into()))?,
            &txn.open_table(REVISIONS)
                .map_err(|e| MutationError::Storage(e.into()))?,
            &txn.open_table(MODIFIED)
                .map_err(|e| MutationError::Storage(e.into()))?,
        )?;
        match (&previous, expected) {
            (Some(current), Some(revision)) if current.revision != revision => {
                return Err(MutationError::Conflict(current.clone()))
            }
            (None, _) if !create => return Err(MutationError::Missing),
            (None, Some(revision)) if revision != 0 => return Err(MutationError::Missing),
            _ => {}
        }
        let content = update(previous.as_ref().map(|v| v.content.as_str()).unwrap_or(""))?;
        let revision = previous
            .as_ref()
            .map(|v| v.revision)
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| MutationError::Storage("memory revision exhausted".into()))?;
        if let Some(previous) = &previous {
            // Includes timestamps and exact displaced bytes; archive filesystem failures cannot lose them.
            let history_key = serde_json::to_string(&(key, previous.revision))
                .map_err(|e| MutationError::Storage(e.into()))?;
            let history =
                serde_json::to_string(previous).map_err(|e| MutationError::Storage(e.into()))?;
            txn.open_table(HISTORY)
                .map_err(|e| MutationError::Storage(e.into()))?
                .insert(history_key.as_str(), history.as_str())
                .map_err(|e| MutationError::Storage(e.into()))?;
        }
        txn.open_table(CONTENT)
            .map_err(|e| MutationError::Storage(e.into()))?
            .insert(key, content.as_str())
            .map_err(|e| MutationError::Storage(e.into()))?;
        txn.open_table(MODIFIED)
            .map_err(|e| MutationError::Storage(e.into()))?
            .insert(key, when)
            .map_err(|e| MutationError::Storage(e.into()))?;
        txn.open_table(REVISIONS)
            .map_err(|e| MutationError::Storage(e.into()))?
            .insert(key, revision)
            .map_err(|e| MutationError::Storage(e.into()))?;
        let index = build_index_transaction(&txn)?;
        txn.open_table(CONTENT)
            .map_err(|e| MutationError::Storage(e.into()))?
            .insert("INDEX.md", index.as_str())
            .map_err(|e| MutationError::Storage(e.into()))?;
        txn.open_table(MODIFIED)
            .map_err(|e| MutationError::Storage(e.into()))?
            .insert("INDEX.md", when)
            .map_err(|e| MutationError::Storage(e.into()))?;
        commit(txn)?;
        Ok((
            RecordSnapshot {
                key: key.to_owned(),
                content,
                revision,
                modified: when,
            },
            index,
        ))
    };
    let (record, index) = work()?;
    publish(&record, index);
    Ok(record)
}

pub fn refresh_index_for(bot: Option<&Bot>) -> Result<String, BotError> {
    let _gate = MUTATIONS
        .lock()
        .map_err(|_| "memory mutation lock poisoned")?;
    let index = refresh_index_database(db_result()?)?;
    if let Some(bot) = bot {
        bot.state().index = index.clone();
    }
    Ok(index)
}

pub(super) fn write_coordinated(
    database: &Database,
    key: &str,
    content: &str,
    modified: u64,
) -> Result<(), BotError> {
    coordinated(database, None, key, None, true, modified, |_| {
        Ok(content.to_owned())
    })
    .map(|_| ())
    .map_err(|e| Box::new(e) as BotError)
}

/// Runtime controls share commit-success-only publication but are not editable prose.
pub fn set_model_override(bot: &Bot, model: &str, save: bool) -> Result<(), BotError> {
    persist_control(
        bot,
        "model.md",
        if save { Some(model) } else { None },
        |state| {
            state.model = model.to_owned();
        },
    )
}
pub fn set_thinking_mode(bot: &Bot, mode: ThinkingMode) -> Result<(), BotError> {
    persist_control(bot, "dusunme.md", Some(mode.file_value()), |state| {
        state.thinking_mode = mode;
    })
}
pub fn set_debug_mode(bot: &Bot, enabled: Option<bool>) -> Result<bool, BotError> {
    set_debug_commit(db_result()?, bot, enabled, |txn| {
        txn.commit().map_err(|e| e.into())
    })
}
fn set_debug_commit<C>(
    database: &Database,
    bot: &Bot,
    enabled: Option<bool>,
    commit: C,
) -> Result<bool, BotError>
where
    C: FnOnce(WriteTransaction) -> Result<(), BotError>,
{
    let _gate = MUTATIONS
        .lock()
        .map_err(|_| "memory mutation lock poisoned")?;
    let enabled = enabled.unwrap_or_else(|| !bot.state().debug);
    persist_control_commit(
        database,
        bot,
        "debug.md",
        Some(if enabled { "acik" } else { "kapali" }),
        |state| {
            state.debug = enabled;
        },
        commit,
    )?;
    Ok(enabled)
}
fn persist_control<F>(bot: &Bot, key: &str, value: Option<&str>, publish: F) -> Result<(), BotError>
where
    F: FnOnce(&mut State),
{
    let _gate = MUTATIONS
        .lock()
        .map_err(|_| "memory mutation lock poisoned")?;
    persist_control_commit(db_result()?, bot, key, value, publish, |txn| {
        txn.commit().map_err(|e| e.into())
    })
}

fn persist_control_commit<F, C>(
    database: &Database,
    bot: &Bot,
    key: &str,
    value: Option<&str>,
    publish: F,
    commit: C,
) -> Result<(), BotError>
where
    F: FnOnce(&mut State),
    C: FnOnce(WriteTransaction) -> Result<(), BotError>,
{
    let txn = database.begin_write()?;
    {
        let mut content = txn.open_table(CONTENT)?;
        let mut modified = txn.open_table(MODIFIED)?;
        if let Some(value) = value {
            content.insert(key, value)?;
            modified.insert(key, now_unix() as u64)?;
        } else {
            content.remove(key)?;
            modified.remove(key)?;
        }
    }
    commit(txn)?;
    publish(&mut bot.state());
    Ok(())
}

#[cfg(test)]
pub(crate) fn historical_snapshot(key: &str, revision: u64) -> RecordSnapshot {
    let transaction = db().begin_read().expect("read test history");
    let history = transaction.open_table(HISTORY).expect("open test history");
    let history_key = serde_json::to_string(&(key, revision)).expect("encode test history key");
    let record = history
        .get(history_key.as_str())
        .expect("read test history row")
        .expect("requested test revision exists");
    serde_json::from_str(record.value()).expect("decode preserved snapshot")
}

#[cfg(test)]
#[path = "revisions_tests.rs"]
mod tests;
