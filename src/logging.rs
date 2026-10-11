// Lightweight logging: `log` macros + a hand-written sink. Level comes from the
// LOG_LEVEL environment variable (default info); color is automatic on a terminal, or
// forced with LOG_COLOR=on|off. Only discord_bot's own records pass the level filter as
// configured; other crates' (serenity, reqwest, ...) internal events are shown only at
// warn/error, so they don't flood the console.

use log::{Level, LevelFilter, Log, Metadata, Record};
use serde::Serialize;
use std::collections::VecDeque;
use std::fmt::{self, Write};
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

/// Atomic log-level scale: 0 (off) through 5 (trace).
fn filter_number(f: LevelFilter) -> u8 {
    match f {
        LevelFilter::Off => 0,
        LevelFilter::Error => 1,
        LevelFilter::Warn => 2,
        LevelFilter::Info => 3,
        LevelFilter::Debug => 4,
        LevelFilter::Trace => 5,
    }
}

/// Uses the same numeric scale as `filter_number`.
fn level_number(l: Level) -> u8 {
    match l {
        Level::Error => 1,
        Level::Warn => 2,
        Level::Info => 3,
        Level::Debug => 4,
        Level::Trace => 5,
    }
}

// 0=off 1=error 2=warn 3=info 4=debug 5=trace
static LEVEL: AtomicU8 = AtomicU8::new(3);
static COLOR: AtomicBool = AtomicBool::new(false);

const CAPACITY: usize = 512;
const ENTRY_BYTES: usize = 2048;
// Reserve room for the timestamp, level and sequence in each entry.
const MESSAGE_BYTES: usize = ENTRY_BYTES - 64;
const SECRET_BYTES: usize = 16 * 1024;
const SECRET_TOTAL_BYTES: usize = 64 * 1024;
const SECRET_COUNT: usize = 256;
const RAW_BYTES: usize = MESSAGE_BYTES + SECRET_BYTES;
const REDACTED: &str = "[REDACTED]";

/// Plain text only: the browser must render `message` with textContent, not HTML.
#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    pub id: u64,
    pub timestamp: String,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct Batch {
    pub oldest: Option<u64>,
    pub newest: Option<u64>,
    /// Unavailable sequence IDs between the requested cursor and retained history.
    pub dropped: u64,
    /// The requested cursor belongs to a sequence newer than this process buffer.
    pub reset: bool,
    pub entries: Vec<Entry>,
}

#[derive(Default)]
struct Buffer {
    entries: VecDeque<Arc<Entry>>,
    secrets: Vec<String>,
    last_id: u64,
}

impl Buffer {
    fn push(&mut self, timestamp: String, level: Level, raw: &Text) -> Option<Arc<Entry>> {
        let id = self.last_id.checked_add(1)?;
        let entry = Arc::new(Entry {
            id,
            timestamp,
            level: level.to_string(),
            message: redact(&raw.value, &self.secrets, raw.truncated),
        });
        if self.entries.len() == CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back(Arc::clone(&entry));
        self.last_id = id;
        Some(entry)
    }

    fn snapshot(&self, after: Option<u64>, limit: usize) -> Batch {
        let oldest = self.entries.front().map(|entry| entry.id);
        let newest = self.entries.back().map(|entry| entry.id);
        let reset = after.is_some_and(|cursor| cursor > newest.unwrap_or(0));
        let after = if reset { None } else { after };
        let dropped = match (after, oldest) {
            (Some(cursor), Some(first)) => first.saturating_sub(cursor).saturating_sub(1),
            _ => 0,
        };
        Batch {
            oldest,
            newest,
            dropped,
            reset,
            entries: self
                .entries
                .iter()
                .filter(|entry| after.is_none_or(|cursor| entry.id > cursor))
                .take(limit.min(CAPACITY))
                .map(|entry| entry.as_ref().clone())
                .collect(),
        }
    }

    fn register(&mut self, incoming: Vec<String>) -> Result<(), &'static str> {
        let mut count = self.secrets.len();
        let mut bytes: usize = self.secrets.iter().map(String::len).sum();
        for secret in &incoming {
            if !self.secrets.contains(secret) {
                count += 1;
                bytes += secret.len();
            }
        }
        if count > SECRET_COUNT || bytes > SECRET_TOTAL_BYTES {
            return Err("secret redaction registry capacity exceeded");
        }
        for secret in incoming {
            if !self.secrets.contains(&secret) {
                self.secrets.push(secret);
            }
        }
        for entry in &mut self.entries {
            let entry = Arc::make_mut(entry);
            let truncated = entry.message.ends_with('…');
            let message = if truncated {
                entry.message.trim_end_matches('…')
            } else {
                &entry.message
            };
            entry.message = redact(message, &self.secrets, truncated);
        }
        Ok(())
    }
}

static BUFFER: LazyLock<Mutex<Buffer>> = LazyLock::new(|| Mutex::new(Buffer::default()));

fn buffer() -> MutexGuard<'static, Buffer> {
    // Operations do not call user code or log while holding this guard. Recovery keeps
    // already-redacted history available if an unrelated unwind poisoned the mutex.
    BUFFER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Register secrets before logging configuration or starting credential-bearing work.
/// Old values remain registered so in-flight calls remain protected after key changes.
/// Empty values are ignored; failure leaves the existing registry unchanged.
pub fn set_secrets<I, S>(values: I) -> Result<(), &'static str>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let incoming = secrets(values)?;
    buffer().register(incoming)
}

fn secrets<I, S>(values: I) -> Result<Vec<String>, &'static str>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut result = Vec::new();
    let mut bytes = 0;
    for value in values {
        let value = value.as_ref();
        if value.is_empty() || result.iter().any(|old| old == value) {
            continue;
        }
        if value.len() > SECRET_BYTES {
            return Err("secret exceeds redaction value limit");
        }
        bytes += value.len();
        if result.len() == SECRET_COUNT || bytes > SECRET_TOTAL_BYTES {
            return Err("secret redaction registry capacity exceeded");
        }
        result.push(value.to_owned());
    }
    Ok(result)
}

/// Exclusive cursor; None starts at retained history. Limits are capped at 512.
/// SSE callers can poll this synchronous operation without holding a guard over await.
pub fn snapshot(after: Option<u64>, limit: usize) -> Batch {
    buffer().snapshot(after, limit)
}

struct Text {
    value: String,
    limit: usize,
    truncated: bool,
}

impl Text {
    fn new(limit: usize) -> Self {
        Self {
            value: String::new(),
            limit,
            truncated: false,
        }
    }

    fn finish(mut self) -> String {
        if self.truncated {
            let mut end = self.value.len().min(self.limit - '…'.len_utf8());
            while !self.value.is_char_boundary(end) {
                end -= 1;
            }
            self.value.truncate(end);
            self.value.push('…');
        }
        self.value
    }
}

impl Write for Text {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self.truncated {
            return Ok(());
        }
        let available = self.limit - self.value.len();
        let mut end = available.min(value.len());
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        self.value.push_str(&value[..end]);
        self.truncated = end < value.len();
        Ok(())
    }
}

fn redact(text: &str, secrets: &[String], truncated: bool) -> String {
    let mut output = Text::new(MESSAGE_BYTES);
    let mut plain = 0;
    let mut hidden: Option<(usize, usize)> = None;
    // Scan even inside hidden ranges: overlapping registered values must not expose
    // another credential's suffix. One merged range avoids a match-sized allocation.
    for (start, _) in text.char_indices() {
        let suffix = &text[start..];
        let mut end = sensitive_end(text, start);
        for secret in secrets {
            let possible_end = (start + secret.len()).min(text.len());
            if end >= possible_end || hidden.is_some_and(|(_, stop)| stop >= possible_end) {
                continue;
            }
            if suffix.starts_with(secret) {
                end = end.max(start + secret.len());
            } else if truncated && secret.starts_with(suffix) {
                end = text.len();
            }
        }
        if let Some((begin, stop)) = hidden {
            if start > stop {
                let _ = output.write_str(&text[plain..begin]);
                let _ = output.write_str(REDACTED);
                plain = stop;
                hidden = None;
            }
        }
        if end > start {
            hidden = Some(match hidden {
                Some((begin, stop)) => (begin, stop.max(end)),
                None => (start, end),
            });
        }
        if hidden.is_some_and(|(_, stop)| stop == text.len()) {
            break;
        }
    }
    if let Some((begin, stop)) = hidden {
        let _ = output.write_str(&text[plain..begin]);
        let _ = output.write_str(REDACTED);
        plain = stop;
    }
    let _ = output.write_str(&text[plain..]);
    output.truncated |= truncated;
    output.finish()
}

fn sensitive_end(text: &str, start: usize) -> usize {
    let suffix = &text[start..];
    let bytes = text.as_bytes();
    let boundary = start == 0
        || suffix.starts_with(['#', '?'])
        || !matches!(bytes[start - 1], b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-');
    if !boundary {
        return start;
    }
    for prefix in ["sk-", "sk_", "gsk_", "fc-", "hf_"] {
        if suffix.starts_with(prefix) {
            let end = start
                + suffix
                    .bytes()
                    .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_'))
                    .count();
            if end > start + prefix.len() {
                return end;
            }
        }
    }
    for name in [
        "authorization",
        "set-cookie",
        "cookie",
        "discord_token",
        "openrouter_key",
        "mistral_key",
        "firecrawl_key",
        "provider_key",
        "provider-key",
        "x-api-key",
        "api_key",
        "api-key",
        "setup_token",
        "setup-token",
        "#token",
        "?token",
    ] {
        let Some(label) = suffix.get(..name.len()) else {
            continue;
        };
        if !label.eq_ignore_ascii_case(name) {
            continue;
        }
        let mut value = start + name.len();
        if matches!(bytes.get(value), Some(b'"' | b'\'')) {
            value += 1;
        }
        while matches!(bytes.get(value), Some(b' ' | b'\t')) {
            value += 1;
        }
        if !matches!(bytes.get(value), Some(b':' | b'=')) {
            continue;
        }
        value += 1;
        while matches!(bytes.get(value), Some(b' ' | b'\t')) {
            value += 1;
        }
        if let Some(quote @ (b'"' | b'\'')) = bytes.get(value) {
            value += 1;
            let mut escaped = false;
            for (offset, byte) in bytes[value..].iter().enumerate() {
                if *byte == *quote && !escaped {
                    return value + offset;
                }
                escaped = *byte == b'\\' && !escaped;
            }
            return text.len();
        }
        let header = matches!(name, "authorization" | "cookie" | "set-cookie");
        return value
            + text[value..]
                .char_indices()
                .find(|(_, ch)| {
                    matches!(*ch, '\r' | '\n')
                        || (!header
                            && (ch.is_whitespace()
                                || matches!(*ch, '"' | '\'' | ',' | ';' | '&' | '}')))
                })
                .map_or(text.len() - value, |(offset, _)| offset);
    }
    start
}

fn timestamp() -> String {
    let seconds = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => i128::from(duration.as_secs()),
        Err(error) => {
            let duration = error.duration();
            -i128::from(duration.as_secs()) - i128::from(duration.subsec_nanos() != 0)
        }
    };
    utc(seconds)
}

fn utc(seconds: i128) -> String {
    // Gregorian civil-date conversion from UTC epoch days, independent of bot TZ.
    let days = seconds.div_euclid(86400) + 719468;
    let era = days.div_euclid(146097);
    let day = days - era * 146097;
    let year = (day - day / 1460 + day / 36524 - day / 146096) / 365;
    let day_of_year = day - (365 * year + year / 4 - year / 100);
    let month = (5 * day_of_year + 2) / 153;
    let date = day_of_year - (153 * month + 2) / 5 + 1;
    let month = month + if month < 10 { 3 } else { -9 };
    let year = year + era * 400 + i128::from(month <= 2);
    let clock = seconds.rem_euclid(86400);
    format!(
        "{year:04}-{month:02}-{date:02}T{:02}:{:02}:{:02}Z",
        clock / 3600,
        clock / 60 % 60,
        clock % 60
    )
}

const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";

/// Parses the configured level without initializing bot language or timezone.
fn parse_level(value: &str) -> LevelFilter {
    for (name, level) in [
        ("error", LevelFilter::Error),
        ("warn", LevelFilter::Warn),
        ("debug", LevelFilter::Debug),
        ("trace", LevelFilter::Trace),
        ("kapali", LevelFilter::Off),
        ("off", LevelFilter::Off),
    ] {
        if value.eq_ignore_ascii_case(name) {
            return level;
        }
    }
    LevelFilter::Info
}

fn level_color(l: Level) -> &'static str {
    match l {
        Level::Error => "\x1b[1;31m", // red, bold
        Level::Warn => "\x1b[33m",    // yellow
        Level::Info => "\x1b[32m",    // green
        Level::Debug | Level::Trace => DIM,
    }
}

fn is_our_target(target: &str) -> bool {
    target.starts_with("discord_bot")
}

/// The existing global `log::Log` sink. Level/color are atomics; bounded history and
/// registered secrets share a synchronous guard, released before console output.
struct Sink;

impl Log for Sink {
    /// Applies the configured level to this crate; dependency logs are warn/error only.
    fn enabled(&self, metadata: &Metadata) -> bool {
        let passes = level_number(metadata.level()) <= LEVEL.load(Ordering::Relaxed);
        if is_our_target(metadata.target()) {
            passes
        } else {
            // serenity/reqwest/... internal events: only warnings and errors show
            passes && metadata.level() <= Level::Warn
        }
    }

    /// Captures bounded formatted text, redacts once for both stdout and the panel.
    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let mut raw = Text::new(RAW_BYTES);
        if fmt::write(&mut raw, *record.args()).is_err() {
            eprintln!("log formatting failed");
            return;
        }
        let entry = {
            let mut buffer = buffer();
            buffer.push(timestamp(), record.level(), &raw)
        };
        let Some(entry) = entry else {
            eprintln!("log sequence exhausted");
            return;
        };
        if COLOR.load(Ordering::Relaxed) {
            let code = level_color(record.level());
            println!(
                "{DIM}{}{RESET} {code}{:<5}{RESET} {code}{}{RESET}",
                entry.timestamp, entry.level, entry.message
            );
        } else {
            println!("{} {:<5} {}", entry.timestamp, entry.level, entry.message);
        }
    }

    fn flush(&self) {}
}

static SINK: Sink = Sink;

/// Headless startup shares the immutable typed snapshot, with an environment fallback.
pub fn init() {
    let level = crate::config::runtime_value("LOG_LEVEL").unwrap_or_default();
    let color = crate::config::runtime_value("LOG_COLOR").unwrap_or_default();
    init_with_settings(&level, &color);
}

/// Installs the existing sink using explicit settings, without mutating process env.
pub fn init_with_settings(level: &str, color: &str) {
    let level = parse_level(level);
    LEVEL.store(filter_number(level), Ordering::Relaxed);
    log::set_max_level(level);
    let color = if color.eq_ignore_ascii_case("on") || color.eq_ignore_ascii_case("acik") {
        true
    } else if color.eq_ignore_ascii_case("off") || color.eq_ignore_ascii_case("kapali") {
        false
    } else {
        std::io::stdout().is_terminal()
    };
    COLOR.store(color, Ordering::Relaxed);
    let _ = log::set_logger(&SINK);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(value: &str) -> Text {
        let mut text = Text::new(RAW_BYTES);
        text.write_str(value).unwrap();
        text
    }

    fn push(buffer: &mut Buffer, value: &str) -> Arc<Entry> {
        buffer
            .push("2026-10-11T12:34:56Z".into(), Level::Info, &raw(value))
            .unwrap()
    }

    #[test]
    fn registered_secrets_and_overlapping_values_are_redacted() {
        let values = secrets(["top-secret", "secret-tail", "秘密", ""]).unwrap();
        assert_eq!(
            redact("top-secret-tail and 秘密", &values, false),
            "[REDACTED] and [REDACTED]"
        );
    }

    #[test]
    fn headers_provider_keys_and_setup_tokens_are_redacted() {
        let text = concat!(
            "Authorization: Bearer auth-secret\n",
            "COOKIE: session=cookie-secret; csrf=csrf-secret\n",
            "Set-Cookie: session=another-secret; HttpOnly\n",
            r#"{"openrouter_key":"router-secret","x-api-key":"api-secret"}"#,
            "\nprovider-key=provider-secret setup_token=setup-secret\n",
            "http://localhost:8080/setup#token=fragment-secret\n",
            "http://localhost:8080/setup?token=query-secret\n",
            "sk-or-v1-unregistered fc-unregistered gsk_unregistered hf_unregistered"
        );
        let safe = redact(text, &[], false);
        for value in [
            "auth-secret",
            "cookie-secret",
            "csrf-secret",
            "another-secret",
            "router-secret",
            "api-secret",
            "provider-secret",
            "setup-secret",
            "fragment-secret",
            "query-secret",
            "unregistered",
        ] {
            assert!(!safe.contains(value), "{value} survived: {safe}");
        }
        assert_eq!(safe.lines().count(), text.lines().count());
    }

    #[test]
    fn quoted_and_multiline_values_are_redacted() {
        let text = "MISTRAL_KEY='first\nsecond' FIRECRAWL_KEY=\"escaped\\\"secret\"";
        let safe = redact(text, &[], false);
        for value in ["first", "second", "escaped", "secret"] {
            assert!(!safe.contains(value), "{safe}");
        }
    }

    #[test]
    fn secret_registration_redacts_history_and_retains_old_keys() {
        let mut buffer = Buffer::default();
        push(&mut buffer, "before: opaque-old");
        buffer.register(secrets(["opaque-old"]).unwrap()).unwrap();
        buffer.register(secrets(["opaque-new"]).unwrap()).unwrap();
        push(&mut buffer, "after: opaque-old opaque-new");
        let batch = buffer.snapshot(None, CAPACITY);
        assert_eq!(batch.entries[0].message, "before: [REDACTED]");
        assert_eq!(batch.entries[1].message, "after: [REDACTED] [REDACTED]");
    }

    #[test]
    fn secret_registry_is_bounded_and_failure_is_atomic() {
        assert!(secrets([&"a".repeat(SECRET_BYTES + 1)]).is_err());
        assert!(secrets((0..=SECRET_COUNT).map(|i| format!("value-{i}"))).is_err());
        assert!(secrets((0..5).map(|i| format!("{i}{}", "a".repeat(SECRET_BYTES - 1)))).is_err());
        assert_eq!(secrets(["same", "same", ""]).unwrap(), vec!["same"]);
        let mut buffer = Buffer::default();
        buffer
            .register(secrets((0..SECRET_COUNT).map(|i| format!("value-{i}"))).unwrap())
            .unwrap();
        assert!(buffer.register(secrets(["overflow"]).unwrap()).is_err());
        assert_eq!(buffer.secrets.len(), SECRET_COUNT);
        assert!(!buffer.secrets.iter().any(|secret| secret == "overflow"));
    }

    #[test]
    fn ring_sequences_limits_and_lag_are_bounded() {
        let mut buffer = Buffer::default();
        for id in 1..=CAPACITY + 10 {
            assert_eq!(push(&mut buffer, "line").id, id as u64);
        }
        let batch = buffer.snapshot(Some(2), usize::MAX);
        assert_eq!(batch.oldest, Some(11));
        assert_eq!(batch.newest, Some(522));
        assert_eq!(batch.dropped, 8);
        assert_eq!(batch.entries.len(), CAPACITY);
        assert_eq!(batch.entries[0].id, 11);
        assert_eq!(buffer.snapshot(Some(520), 1).entries[0].id, 521);
        assert!(buffer.snapshot(Some(522), 100).entries.is_empty());
        let restarted = buffer.snapshot(Some(u64::MAX), 100);
        assert!(restarted.reset);
        assert_eq!(restarted.entries.len(), 100);
        assert_eq!(restarted.entries[0].id, 11);
        assert_eq!(buffer.snapshot(None, 0).dropped, 0);
        assert!(buffer.snapshot(None, 0).entries.is_empty());
    }

    #[test]
    fn empty_snapshot_and_sequence_exhaustion_are_explicit() {
        let mut buffer = Buffer::default();
        let batch = buffer.snapshot(None, 1);
        assert_eq!(batch.oldest, None);
        assert_eq!(batch.newest, None);
        assert_eq!(batch.dropped, 0);
        buffer.last_id = u64::MAX;
        assert!(buffer
            .push(timestamp(), Level::Error, &raw("line"))
            .is_none());
        assert!(buffer.entries.is_empty());
    }

    #[test]
    fn messages_remain_utf8_and_bounded_before_and_after_redaction() {
        let mut buffer = Buffer::default();
        let text = "💬".repeat(RAW_BYTES);
        let captured = raw(&text);
        assert!(captured.truncated);
        assert!(captured.value.len() <= RAW_BYTES);
        let entry = push(&mut buffer, &text);
        assert!(entry.message.len() <= MESSAGE_BYTES);
        assert!(entry.message.ends_with('…'));
        assert!(
            entry.message.len() + entry.timestamp.len() + entry.level.len() + 20 <= ENTRY_BYTES
        );
        buffer.register(secrets(["💬"]).unwrap()).unwrap();
        assert!(!buffer.snapshot(None, 1).entries[0].message.contains('💬'));
    }

    #[test]
    fn clipping_cannot_reveal_registered_secret_prefixes() {
        let secret = "s".repeat(SECRET_BYTES);
        let text = format!("{}{}", "x".repeat(MESSAGE_BYTES - 3), secret);
        let captured = raw(&text);
        let safe = redact(
            &captured.value,
            std::slice::from_ref(&secret),
            captured.truncated,
        );
        assert!(!safe.contains('s'));
        // A shrinking redaction can expose the end of the capture; redact a partial
        // credential there too, rather than publishing its prefix.
        let text = format!("{secret}{}", secret);
        let captured = raw(&text);
        assert!(captured.truncated);
        let safe = redact(&captured.value, &[secret], captured.truncated);
        assert!(!safe.contains('s'));
    }

    #[test]
    fn xss_text_is_preserved_as_text_in_the_serialized_dto() {
        let text = "<script>alert('x')</script> & \"<img src=x onerror=alert(1)>\"";
        let mut buffer = Buffer::default();
        let entry = push(&mut buffer, text);
        assert_eq!(entry.message, text);
        let json = serde_json::to_value(entry.as_ref()).unwrap();
        assert_eq!(json["id"], 1);
        assert_eq!(json["timestamp"], "2026-10-11T12:34:56Z");
        assert_eq!(json["level"], "INFO");
        assert_eq!(json["message"], text);
        assert_eq!(json.as_object().unwrap().len(), 4);
    }

    #[test]
    fn bootstrap_timestamp_is_utc() {
        let value = timestamp();
        assert_eq!(value.len(), 20);
        assert!(value.ends_with('Z'));
        assert_eq!(&value[10..11], "T");
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(-1), "1969-12-31T23:59:59Z");
        assert_eq!(utc(951782400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn restarted_log_cursor_recovers_even_before_the_first_new_line() {
        let mut buffer = Buffer::default();
        let batch = buffer.snapshot(Some(999), 64);
        assert!(batch.reset);
        assert_eq!(batch.dropped, 0);
        assert!(batch.entries.is_empty());
        push(&mut buffer, "new process");
        let batch = buffer.snapshot(Some(999), 64);
        assert!(batch.reset);
        assert_eq!(batch.entries[0].id, 1);
        assert!(!buffer.snapshot(Some(1), 64).reset);
    }
}
