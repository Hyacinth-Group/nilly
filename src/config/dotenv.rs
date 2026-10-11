use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use super::{ConfigError, MAX_FILE_BYTES, MAX_VALUE_BYTES};

pub struct Document {
    pub text: String,
    pub exists: bool,
    pub values: HashMap<String, String>,
    pub errors: Vec<String>,
}

impl Document {
    pub fn read(path: &Path) -> Result<Self, ConfigError> {
        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Self::parse(String::new(), false)
            }
            Err(error) => return Err(error.into()),
        };
        let mut bytes = Vec::new();
        (&mut file)
            .take((MAX_FILE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(ConfigError::TooLarge);
        }
        let text = String::from_utf8(bytes).map_err(|_| ConfigError::InvalidEncoding)?;
        Self::parse(text, true)
    }

    pub fn parse(text: String, exists: bool) -> Result<Self, ConfigError> {
        if text.len() > MAX_FILE_BYTES {
            return Err(ConfigError::TooLarge);
        }
        let mut values = HashMap::new();
        let mut errors = Vec::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            match assignment(line) {
                Some((key, raw)) => match decode(raw) {
                    Ok(value) => {
                        values.insert(key.to_owned(), value);
                    }
                    Err(()) => {
                        values.insert(key.to_owned(), raw.to_owned());
                        errors.push(key.to_owned());
                    }
                },
                None => errors.push(String::new()),
            }
        }
        Ok(Self {
            text,
            exists,
            values,
            errors,
        })
    }

    pub fn revision(&self) -> String {
        let mut hash = Sha256::new();
        hash.update([u8::from(self.exists)]);
        hash.update(self.text.as_bytes());
        format!("{:x}", hash.finalize())
    }

    pub fn patched(&self, changes: &HashMap<String, String>) -> Result<Self, ConfigError> {
        let newline = if self.text.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let mut output = String::new();
        let mut seen = std::collections::HashSet::new();
        for line in self.text.split_inclusive('\n') {
            let body = line.trim_end_matches(['\r', '\n']);
            if let Some((key, raw)) = assignment(body) {
                if let Some(value) = changes.get(key) {
                    seen.insert(key.to_owned());
                    let prefix = &body[..body.len() - raw.len()];
                    output.push_str(prefix);
                    output.push_str(&encode(value)?);
                    if let Some(comment) = inline_comment(raw) {
                        output.push(' ');
                        output.push_str(comment);
                    }
                    if line.ends_with('\n') {
                        output.push_str(newline);
                    }
                    continue;
                }
            }
            output.push_str(line);
        }
        let mut additions: Vec<_> = changes
            .iter()
            .filter(|(key, _)| !seen.contains(*key))
            .collect();
        additions.sort_by(|left, right| left.0.cmp(right.0));
        for (key, value) in additions {
            if !output.is_empty() && !output.ends_with('\n') {
                output.push_str(newline);
            }
            output.push_str(key);
            output.push('=');
            output.push_str(&encode(value)?);
            output.push_str(newline);
        }
        Self::parse(output, true)
    }
}

fn assignment(line: &str) -> Option<(&str, &str)> {
    let line = line.trim_start();
    let line = line.strip_prefix("export ").map_or(line, str::trim_start);
    let (key, raw) = line.split_once('=')?;
    let key = key.trim();
    if key.is_empty()
        || !key.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_alphabetic() || (index != 0 && byte.is_ascii_digit())
        })
    {
        return None;
    }
    Some((key, raw))
}

// Quotes and escapes are parsed without consulting process environment or expanding `$`.
fn decode(raw: &str) -> Result<String, ()> {
    let raw = raw.trim();
    let mut result = String::new();
    let mut chars = raw.char_indices().peekable();
    let quote = match chars.peek() {
        Some((_, '\'')) => {
            chars.next();
            Some('\'')
        }
        Some((_, '"')) => {
            chars.next();
            Some('"')
        }
        _ => None,
    };
    let mut closed = quote.is_none();
    while let Some((index, ch)) = chars.next() {
        if Some(ch) == quote {
            closed = true;
            let remainder = raw[index + ch.len_utf8()..].trim_start();
            if !remainder.is_empty() && !remainder.starts_with('#') {
                return Err(());
            }
            break;
        }
        if quote.is_none()
            && ch == '#'
            && (index == 0 || raw[..index].ends_with(char::is_whitespace))
        {
            break;
        }
        if ch == '\\' && quote != Some('\'') {
            let (_, escaped) = chars.next().ok_or(())?;
            result.push(match escaped {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                other => other,
            });
        } else {
            result.push(ch);
        }
    }
    if !closed || result.len() > MAX_VALUE_BYTES || result.contains(['\r', '\n', '\0']) {
        return Err(());
    }
    Ok(if quote.is_none() {
        result.trim_end().to_owned()
    } else {
        result
    })
}

fn inline_comment(raw: &str) -> Option<&str> {
    let trimmed = raw.trim_start();
    let quote = trimmed.chars().next().filter(|ch| matches!(ch, '\'' | '"'));
    let start = raw.len() - trimmed.len();
    let mut escaped = false;
    for (index, ch) in raw.char_indices() {
        if quote.is_some() && index == start {
            continue;
        }
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if quote == Some(ch) {
            let remainder = raw[index + ch.len_utf8()..].trim_start();
            return remainder.starts_with('#').then_some(remainder);
        }
        if quote.is_none()
            && ch == '#'
            && (index == 0 || raw[..index].ends_with(char::is_whitespace))
        {
            return Some(&raw[index..]);
        }
    }
    None
}

pub fn encode(value: &str) -> Result<String, ConfigError> {
    if value.len() > MAX_VALUE_BYTES || value.contains(['\r', '\n', '\0']) {
        return Err(ConfigError::Field {
            key: String::new(),
            code: "panel.config.invalid_value",
        });
    }
    if value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"_-./:,".contains(&byte))
    {
        return Ok(value.to_owned());
    }
    let mut encoded = String::from("\"");
    for ch in value.chars() {
        if matches!(ch, '\\' | '"' | '$') {
            encoded.push('\\');
        }
        encoded.push(ch);
    }
    encoded.push('"');
    Ok(encoded)
}

pub fn persist(path: &Path, text: &str, expected_revision: &str) -> Result<(), ConfigError> {
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    // NamedTempFile creates mode 0600 on Unix, before any secret bytes are written.
    let mut temporary = tempfile::Builder::new()
        .prefix(".panel-config-")
        .tempfile_in(directory)?;
    temporary.write_all(text.as_bytes())?;
    temporary.as_file().sync_all()?;
    if Document::read(path)?.revision() != expected_revision {
        return Err(ConfigError::Conflict);
    }
    temporary
        .persist(path)
        .map_err(|error| ConfigError::Io(error.error))?;
    #[cfg(unix)]
    File::open(directory)?.sync_all()?;
    #[cfg(not(unix))]
    File::open(path)?.sync_all()?;
    // Replacement already committed if directory fsync fails: caller must reload before retrying.
    Ok(())
}
