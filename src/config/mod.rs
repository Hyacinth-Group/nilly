mod dotenv;
mod registry;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use dotenv::Document;
pub use registry::{FieldKind, FieldSpec, REGISTRY};

pub type ConfigResult<T> = Result<T, ConfigError>;
pub type Patch = HashMap<String, PatchValue>;
const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_VALUE_BYTES: usize = 16 * 1024;
static WRITE_GATE: Mutex<()> = Mutex::new(());
static ACTIVE: OnceLock<Config> = OnceLock::new();

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Field { key: String, code: &'static str },
    Conflict,
    TooLarge,
    InvalidEncoding,
    AlreadyActive,
    LockPoisoned,
}

impl ConfigError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io(_) => "panel.config.io_error",
            Self::Field { code, .. } => code,
            Self::Conflict => "panel.config.conflict",
            Self::TooLarge => "panel.config.too_large",
            Self::InvalidEncoding => "panel.config.invalid_encoding",
            Self::AlreadyActive => "panel.config.already_active",
            Self::LockPoisoned => "panel.config.lock_error",
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Field { key, code } => write!(formatter, "{code}: {key}"),
            // OS error text may include paths; neither paths nor raw config values are exposed.
            _ => formatter.write_str(self.code()),
        }
    }
}
impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}
impl From<std::io::Error> for ConfigError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum PatchValue {
    Keep,
    Replace { value: String },
    Clear,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Environment,
    File,
    Default,
}

// A secret's value is absent, not masked with a potentially meaningful length.
#[derive(Serialize)]
pub struct ValueView {
    pub configured: bool,
    pub source: Source,
    pub value: Option<String>,
}

#[derive(Serialize)]
pub struct FieldView {
    pub spec: &'static FieldSpec,
    pub source: Source,
    pub effective: ValueView,
    pub saved: ValueView,
    pub default: ValueView,
    pub read_only: bool,
    pub pending_restart: bool,
}

// Deliberately no Debug/Serialize: snapshots contain write-only credentials.
#[derive(Clone)]
pub struct Config {
    path: PathBuf,
    startup_env: HashMap<String, String>,
    saved: HashMap<String, String>,
    effective: HashMap<String, String>,
    active_sources: HashMap<String, Source>,
    text: String,
    exists: bool,
    parse_errors: Vec<String>,
    revision: String,
}

impl Config {
    // Missing credentials and invalid values are retained for authenticated repair.
    // Runtime startup must separately call validate_runtime before activate/Bot::setup.
    pub fn load(
        path: impl AsRef<Path>,
        startup_env: &HashMap<String, String>,
    ) -> ConfigResult<Self> {
        let path = path.as_ref().to_owned();
        let document = Document::read(&path)?;
        Ok(Self::from_document(path, startup_env, document))
    }

    fn from_document(path: PathBuf, env: &HashMap<String, String>, document: Document) -> Self {
        let startup_env: HashMap<_, _> = env
            .iter()
            .filter(|(key, _)| registry::spec(key).is_some())
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let (effective, active_sources) = resolve(&path, &startup_env, &document.values);
        let revision = document.revision();
        Self {
            path,
            startup_env,
            saved: document.values,
            effective,
            active_sources,
            text: document.text,
            exists: document.exists,
            parse_errors: document.errors,
            revision,
        }
    }

    pub fn get(&self, key: &str) -> &str {
        self.effective.get(key).map(String::as_str).unwrap_or("")
    }

    // Used by the CLI wizard; arbitrary unknown settings are not imported through this API.
    pub fn patch_from_dotenv(content: &str) -> ConfigResult<Patch> {
        let document = Document::parse(content.to_owned(), true)?;
        if let Some(key) = document.errors.first() {
            return Err(field_error(key, "panel.config.invalid_dotenv"));
        }
        document
            .values
            .into_iter()
            .map(|(key, value)| {
                validate_value(&key, &value)?;
                Ok((key, PatchValue::Replace { value }))
            })
            .collect()
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn revision(&self) -> &str {
        &self.revision
    }
    pub fn reload(&self) -> ConfigResult<Self> {
        Self::load(&self.path, &self.startup_env)
    }

    // Refresh saved state after external edits without misreporting it as active runtime state.
    pub fn refresh_saved(&mut self) -> ConfigResult<()> {
        let document = Document::read(&self.path)?;
        self.revision = document.revision();
        self.saved = document.values;
        self.text = document.text;
        self.exists = document.exists;
        self.parse_errors = document.errors;
        Ok(())
    }

    pub fn selected_provider(&self) -> &str {
        self.get("PROVIDER")
    }

    pub fn secret_values(&self) -> Vec<&str> {
        REGISTRY
            .iter()
            .filter(|field| field.secret)
            .flat_map(|field| {
                [
                    self.get(field.key),
                    self.saved.get(field.key).map(String::as_str).unwrap_or(""),
                ]
            })
            .filter(|value| !value.is_empty())
            .collect()
    }

    pub fn unknown_keys(&self) -> Vec<&str> {
        let mut keys: Vec<_> = self
            .saved
            .keys()
            .filter(|key| registry::spec(key).is_none())
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        keys
    }

    pub fn fields(&self) -> Vec<FieldView> {
        let (next, _) = resolve(&self.path, &self.startup_env, &self.saved);
        REGISTRY
            .iter()
            .map(|field| {
                let source = self.active_sources[field.key];
                let saved = self.saved.get(field.key).map(String::as_str).unwrap_or("");
                let default = default_value(field, &next);
                FieldView {
                    spec: field,
                    source,
                    effective: value_view(field, self.get(field.key), source),
                    saved: value_view(
                        field,
                        saved,
                        if self.saved.contains_key(field.key) {
                            Source::File
                        } else {
                            Source::Default
                        },
                    ),
                    default: value_view(field, default, Source::Default),
                    read_only: field.startup_only || self.startup_env.contains_key(field.key),
                    pending_restart: self.get(field.key) != next[field.key],
                }
            })
            .collect()
    }

    pub fn validate_runtime(&self) -> ConfigResult<()> {
        if let Some(key) = self.parse_errors.first() {
            return Err(ConfigError::Field {
                key: key.clone(),
                code: "panel.config.invalid_dotenv",
            });
        }
        for field in REGISTRY {
            registry::normalize(field, self.get(field.key))?;
        }
        if self.get("DISCORD_TOKEN").is_empty() && self.get("TELEGRAM_TOKEN").is_empty() {
            return Err(field_error(
                "DISCORD_TOKEN",
                "panel.config.platform_required",
            ));
        }
        self.validate_provider()
    }

    // Terminal chat does not require a platform credential.
    pub fn validate_provider(&self) -> ConfigResult<()> {
        let key = match self.get("PROVIDER") {
            "openrouter" => "OPENROUTER_KEY",
            "mistral" => "MISTRAL_KEY",
            _ => return Err(field_error("PROVIDER", "panel.config.invalid_choice")),
        };
        if self.get(key).is_empty() {
            return Err(field_error(key, "panel.config.required"));
        }
        Ok(())
    }

    pub fn validate_patch(&self, patch: &Patch) -> ConfigResult<Self> {
        let candidate = self.candidate(patch)?;
        candidate.validate_runtime()?;
        Ok(candidate)
    }

    fn candidate(&self, patch: &Patch) -> ConfigResult<Self> {
        if patch.len() > REGISTRY.len() {
            return Err(ConfigError::TooLarge);
        }
        let mut changes = HashMap::new();
        for (key, operation) in patch {
            let field = registry::spec(key)
                .ok_or_else(|| field_error(key, "panel.config.unknown_field"))?;
            if matches!(operation, PatchValue::Keep) {
                continue;
            }
            if field.startup_only || self.startup_env.contains_key(key) {
                return Err(field_error(key, "panel.config.read_only"));
            }
            let value = match operation {
                PatchValue::Keep => continue,
                PatchValue::Replace { value } => {
                    let value = registry::normalize(field, value)?;
                    if field.secret && value.is_empty() {
                        continue;
                    }
                    value
                }
                PatchValue::Clear => String::new(),
            };
            changes.insert(key.clone(), value);
        }
        let document = Document::parse(self.text.clone(), self.exists)?.patched(&changes)?;
        Ok(Self::from_document(
            self.path.clone(),
            &self.startup_env,
            document,
        ))
    }

    // Synchronous disk work: HTTP callers run this in their bounded blocking executor.
    // An external edit changes the content hash even if no known setting changed.
    pub fn save_patch(&mut self, expected_revision: &str, patch: &Patch) -> ConfigResult<()> {
        let _gate = WRITE_GATE.lock().map_err(|_| ConfigError::LockPoisoned)?;
        if expected_revision != self.revision
            || Document::read(&self.path)?.revision() != expected_revision
        {
            return Err(ConfigError::Conflict);
        }
        let candidate = self.validate_patch(patch)?;
        dotenv::persist(&self.path, &candidate.text, expected_revision)?;
        self.saved = candidate.saved;
        self.text = candidate.text;
        self.exists = true;
        self.parse_errors = candidate.parse_errors;
        self.revision = candidate.revision;
        Ok(())
    }
}

pub fn validate_value(key: &str, value: &str) -> ConfigResult<String> {
    let field =
        registry::spec(key).ok_or_else(|| field_error(key, "panel.config.unknown_field"))?;
    registry::normalize(field, value)
}

pub fn activate(config: Config) -> ConfigResult<()> {
    config.validate_runtime()?;
    ACTIVE.set(config).map_err(|_| ConfigError::AlreadyActive)
}

pub fn runtime_value(name: &str) -> Result<String, std::env::VarError> {
    match ACTIVE.get() {
        Some(config) if registry::spec(name).is_some() => Ok(config.get(name).to_owned()),
        _ => std::env::var(name),
    }
}

pub fn encode_value(key: &str, value: &str) -> ConfigResult<String> {
    dotenv::encode(&validate_value(key, value)?)
}

fn field_error(key: &str, code: &'static str) -> ConfigError {
    ConfigError::Field {
        key: key.to_owned(),
        code,
    }
}
fn value_view(field: &FieldSpec, value: &str, source: Source) -> ValueView {
    ValueView {
        configured: !value.is_empty(),
        source,
        value: (!field.secret).then(|| value.to_owned()),
    }
}

fn resolve(
    path: &Path,
    env: &HashMap<String, String>,
    saved: &HashMap<String, String>,
) -> (HashMap<String, String>, HashMap<String, Source>) {
    let mut values = HashMap::new();
    let mut sources = HashMap::new();
    for field in REGISTRY {
        let (raw, source) = if let Some(value) = env.get(field.key) {
            (value.as_str(), Source::Environment)
        } else if field.startup_only {
            (path.to_str().unwrap_or(".env"), Source::Default)
        } else if let Some(value) = saved.get(field.key) {
            (value.as_str(), Source::File)
        } else {
            ("", Source::Default)
        };
        let value = registry::normalize(field, raw).unwrap_or_else(|_| raw.to_owned());
        values.insert(field.key.to_owned(), value);
        sources.insert(field.key.to_owned(), source);
    }
    if values["PROVIDER"].is_empty() {
        let provider = if values["OPENROUTER_KEY"].is_empty() && !values["MISTRAL_KEY"].is_empty() {
            "mistral"
        } else {
            "openrouter"
        };
        values.insert("PROVIDER".to_owned(), provider.to_owned());
    }
    for field in REGISTRY {
        if values[field.key].is_empty() && !field.secret {
            let default = default_value(field, &values).to_owned();
            values.insert(field.key.to_owned(), default);
        }
    }
    (values, sources)
}

fn default_value<'a>(field: &'a FieldSpec, values: &HashMap<String, String>) -> &'a str {
    match field.key {
        "MODEL"
            if values
                .get("PROVIDER")
                .is_some_and(|value| value == "mistral") =>
        {
            "mistral-medium-latest"
        }
        "API_URL"
            if values
                .get("PROVIDER")
                .is_some_and(|value| value == "mistral") =>
        {
            "https://api.mistral.ai/v1/chat/completions"
        }
        "NEWS_RSS" if values.get("BOT_LANG").is_some_and(|value| value == "tr") => {
            "https://www.sozcu.com.tr/rss/news.xml"
        }
        _ => field.default,
    }
}
