use serde::Serialize;
use std::net::IpAddr;
use std::str::FromStr;

use crate::platform::types::UserKey;

use super::{ConfigError, MAX_VALUE_BYTES};

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Secret,
    Provider,
    Model,
    Url,
    Id,
    UserKey,
    IdList,
    TelegramRooms,
    SlackChannel,
    SlackChannels,
    SlackHistoryTier,
    MatrixHomeserver,
    MatrixUser,
    MatrixRooms,
    TimezoneOffset,
    Language,
    Bool,
    LogLevel,
    LogColor,
    Bind,
    Port,
    Path,
    ProxyList,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct FieldSpec {
    pub key: &'static str,
    pub kind: FieldKind,
    pub default: &'static str,
    pub secret: bool,
    pub label_key: &'static str,
    pub help_key: &'static str,
    pub dependency: Option<&'static str>,
    pub application: &'static str,
    pub startup_only: bool,
}

macro_rules! field {
    ($key:literal, $kind:ident, $default:literal) => {
        FieldSpec {
            key: $key,
            kind: FieldKind::$kind,
            default: $default,
            secret: matches!(FieldKind::$kind, FieldKind::Secret),
            label_key: concat!("panel.settings.", $key, ".label"),
            help_key: concat!("panel.settings.", $key, ".help"),
            dependency: None,
            application: "restart",
            startup_only: matches!(FieldKind::$kind, FieldKind::Path),
        }
    };
}

pub static REGISTRY: &[FieldSpec] = &[
    field!("DISCORD_TOKEN", Secret, ""),
    field!("TELEGRAM_TOKEN", Secret, ""),
    field!("TELEGRAM_ROOMS", TelegramRooms, ""),
    field!("SLACK_APP_TOKEN", Secret, ""),
    field!("SLACK_BOT_TOKEN", Secret, ""),
    field!("SLACK_CHANNELS", SlackChannels, ""),
    field!("SLACK_HISTORY_TIER", SlackHistoryTier, "internal"),
    field!("SLACK_TEST_CHANNEL", SlackChannel, ""),
    field!("MATRIX_HOMESERVER", MatrixHomeserver, ""),
    field!("MATRIX_USER", MatrixUser, ""),
    field!("MATRIX_ACCESS_TOKEN", Secret, ""),
    field!("MATRIX_PASSWORD", Secret, ""),
    field!("MATRIX_ROOMS", MatrixRooms, ""),
    field!("OWNER_KEY", UserKey, ""),
    field!("OWNER_ID", Id, ""),
    field!("TZ_OFFSET", TimezoneOffset, "0"),
    FieldSpec {
        dependency: Some("PROVIDER=openrouter"),
        ..field!("OPENROUTER_KEY", Secret, "")
    },
    FieldSpec {
        dependency: Some("PROVIDER=mistral"),
        ..field!("MISTRAL_KEY", Secret, "")
    },
    field!("PROVIDER", Provider, "openrouter"),
    field!("MODEL", Model, "openai/gpt-4o-mini"),
    field!(
        "API_URL",
        Url,
        "https://openrouter.ai/api/v1/chat/completions"
    ),
    field!("FIRECRAWL_KEY", Secret, ""),
    field!("NEWS_CHANNEL", Id, ""),
    field!(
        "NEWS_RSS",
        Url,
        "https://feeds.bbci.co.uk/news/world/rss.xml"
    ),
    field!("GUILD_ID", Id, ""),
    field!("CHANNELS", IdList, ""),
    field!("DEBUG_CHANNEL", Id, ""),
    field!("IMAGE_ANALYSIS", Bool, "on"),
    field!("STEALTH", Bool, "off"),
    field!("BOT_LANG", Language, "en"),
    field!("LOG_LEVEL", LogLevel, "info"),
    field!("LOG_COLOR", LogColor, "auto"),
    field!("PANEL_ENABLED", Bool, "on"),
    field!("PANEL_BIND", Bind, "127.0.0.1"),
    field!("PANEL_PORT", Port, "8787"),
    field!("PANEL_CONFIG_PATH", Path, ".env"),
    field!("PANEL_PUBLIC_URL", Url, ""),
    field!("PANEL_TRUSTED_PROXIES", ProxyList, ""),
];

pub fn spec(key: &str) -> Option<&'static FieldSpec> {
    REGISTRY.iter().find(|field| field.key == key)
}

fn invalid(field: &FieldSpec, code: &'static str) -> ConfigError {
    ConfigError::Field {
        key: field.key.to_owned(),
        code,
    }
}

pub fn normalize(field: &FieldSpec, raw: &str) -> Result<String, ConfigError> {
    if raw.len() > MAX_VALUE_BYTES || raw.contains(['\r', '\n', '\0']) {
        return Err(invalid(field, "panel.config.invalid_value"));
    }
    let value = raw.trim();
    if value.is_empty() {
        return Ok(String::new());
    }
    let normalized = match field.kind {
        FieldKind::Bool | FieldKind::LogColor => match value.to_lowercase().as_str() {
            "on" | "true" | "1" | "yes" | "acik" | "açık" | "evet" => "on",
            "off" | "false" | "0" | "no" | "kapali" | "kapalı" | "hayir" | "hayır" => "off",
            "auto" if matches!(field.kind, FieldKind::LogColor) => "auto",
            _ => return Err(invalid(field, "panel.config.invalid_choice")),
        }
        .to_owned(),
        FieldKind::Provider => choice(field, value, &["openrouter", "mistral"])?,
        FieldKind::Language => choice(field, value, &["en", "tr"])?,
        FieldKind::SlackHistoryTier => choice(field, value, &["internal", "restricted"])?,
        FieldKind::LogLevel => {
            if matches!(value.to_lowercase().as_str(), "kapali" | "kapalı") {
                "off".to_owned()
            } else {
                choice(
                    field,
                    value,
                    &["off", "error", "warn", "info", "debug", "trace"],
                )?
            }
        }
        FieldKind::Id => id(field, value)?.to_string(),
        FieldKind::UserKey => UserKey::from_str(value)
            .map_err(|_| invalid(field, "panel.config.invalid_user_key"))?
            .to_string(),
        FieldKind::IdList => {
            let mut ids = Vec::new();
            for part in value.split(',') {
                let parsed = id(field, part.trim())?;
                if !ids.contains(&parsed) {
                    ids.push(parsed);
                }
                if ids.len() > 1024 {
                    return Err(invalid(field, "panel.config.too_many_ids"));
                }
            }
            ids.iter().map(u64::to_string).collect::<Vec<_>>().join(",")
        }
        FieldKind::TelegramRooms => {
            let rooms = crate::platform::telegram::runner::parse_rooms(value)
                .map_err(|_| invalid(field, "panel.config.invalid_telegram_rooms"))?;
            let mut rooms: Vec<_> = rooms.into_iter().map(|room| room.to_string()).collect();
            rooms.sort_unstable();
            rooms.join(",")
        }
        FieldKind::SlackChannel => slack_channel(field, value)?.to_owned(),
        FieldKind::SlackChannels => {
            let mut channels = Vec::new();
            for part in value.split(',') {
                let channel = slack_channel(field, part.trim())?;
                if !channels.contains(&channel) {
                    channels.push(channel);
                }
                if channels.len() > 1024 {
                    return Err(invalid(field, "panel.config.too_many_ids"));
                }
            }
            channels.join(",")
        }
        FieldKind::MatrixHomeserver => {
            let url = reqwest::Url::parse(value)
                .map_err(|_| invalid(field, "panel.config.invalid_matrix_homeserver"))?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(invalid(field, "panel.config.invalid_matrix_homeserver"));
            }
            value.to_owned()
        }
        FieldKind::MatrixUser => matrix_user(field, value)?,
        FieldKind::MatrixRooms => matrix_rooms(field, value)?,
        FieldKind::TimezoneOffset => {
            let hours: f64 = value
                .parse()
                .map_err(|_| invalid(field, "panel.config.invalid_timezone"))?;
            if !hours.is_finite() || !(-12.0..=14.0).contains(&hours) {
                return Err(invalid(field, "panel.config.invalid_timezone"));
            }
            hours.to_string()
        }
        FieldKind::Url => {
            validate_url(field, value)?;
            value.to_owned()
        }
        FieldKind::Bind => value
            .parse::<IpAddr>()
            .map_err(|_| invalid(field, "panel.config.invalid_address"))?
            .to_string(),
        FieldKind::Port => {
            let port = value
                .parse::<u16>()
                .map_err(|_| invalid(field, "panel.config.invalid_port"))?;
            if port == 0 {
                return Err(invalid(field, "panel.config.invalid_port"));
            }
            port.to_string()
        }
        FieldKind::ProxyList => {
            let mut addresses = Vec::new();
            for part in value.split(',') {
                let address = part
                    .trim()
                    .parse::<IpAddr>()
                    .map_err(|_| invalid(field, "panel.config.invalid_address"))?
                    .to_string();
                if !addresses.contains(&address) {
                    addresses.push(address);
                }
                if addresses.len() > 64 {
                    return Err(invalid(field, "panel.config.too_many_proxies"));
                }
            }
            addresses.join(",")
        }
        FieldKind::Model => {
            if value.chars().any(char::is_whitespace) {
                return Err(invalid(field, "panel.config.invalid_model"));
            }
            value.to_owned()
        }
        FieldKind::Secret | FieldKind::Path => value.to_owned(),
    };
    Ok(normalized)
}

fn choice(field: &FieldSpec, value: &str, choices: &[&str]) -> Result<String, ConfigError> {
    let value = value.to_lowercase();
    if choices.contains(&value.as_str()) {
        Ok(value)
    } else {
        Err(invalid(field, "panel.config.invalid_choice"))
    }
}

fn slack_channel<'a>(field: &FieldSpec, value: &'a str) -> Result<&'a str, ConfigError> {
    if crate::platform::slack::valid_channel_id(value) {
        Ok(value)
    } else {
        Err(invalid(field, "panel.config.invalid_slack_channel"))
    }
}

#[cfg(feature = "matrix")]
fn matrix_user(field: &FieldSpec, value: &str) -> Result<String, ConfigError> {
    if value
        .chars()
        .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return Err(invalid(field, "panel.config.invalid_matrix_user"));
    }
    let user = matrix_sdk::ruma::UserId::parse(value)
        .map_err(|_| invalid(field, "panel.config.invalid_matrix_user"))?;
    if user.localpart().is_empty() {
        return Err(invalid(field, "panel.config.invalid_matrix_user"));
    }
    Ok(user.to_string())
}

#[cfg(feature = "matrix")]
fn matrix_rooms(field: &FieldSpec, value: &str) -> Result<String, ConfigError> {
    let mut rooms = Vec::new();
    for part in value.split(',') {
        let part = part.trim();
        // Preserve lexical/empty-component checks while the SDK validates native IDs.
        // Modern opaque IDs have no server component.
        if part.chars().any(|ch| ch.is_whitespace() || ch.is_control())
            || part.len() <= 1
            || part
                .split_once(':')
                .is_some_and(|(local, server)| local.len() <= 1 || server.is_empty())
        {
            return Err(invalid(field, "panel.config.invalid_matrix_rooms"));
        }
        let room = matrix_sdk::ruma::RoomId::parse(part)
            .map_err(|_| invalid(field, "panel.config.invalid_matrix_rooms"))?;
        if !rooms.contains(&room) {
            rooms.push(room);
        }
        if rooms.len() > 1024 {
            return Err(invalid(field, "panel.config.too_many_ids"));
        }
    }
    Ok(rooms
        .iter()
        .map(|room| room.as_str())
        .collect::<Vec<_>>()
        .join(","))
}

#[cfg(not(feature = "matrix"))]
fn matrix_user(field: &FieldSpec, _value: &str) -> Result<String, ConfigError> {
    Err(invalid(field, "panel.config.matrix_feature_required"))
}

#[cfg(not(feature = "matrix"))]
fn matrix_rooms(field: &FieldSpec, _value: &str) -> Result<String, ConfigError> {
    Err(invalid(field, "panel.config.matrix_feature_required"))
}

fn id(field: &FieldSpec, value: &str) -> Result<u64, ConfigError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid(field, "panel.config.invalid_id"));
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|id| *id != 0)
        .ok_or_else(|| invalid(field, "panel.config.invalid_id"))
}

fn validate_url(field: &FieldSpec, value: &str) -> Result<(), ConfigError> {
    if value
        .chars()
        .any(|ch| ch.is_whitespace() || ch.is_control())
        || value.contains('\\')
    {
        return Err(invalid(field, "panel.config.invalid_url"));
    }
    let url = reqwest::Url::parse(value).map_err(|_| invalid(field, "panel.config.invalid_url"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid(field, "panel.config.invalid_url"));
    }
    // HTTP is allowed only for literal loopback model/RSS endpoints, not arbitrary remote services.
    if url.scheme() == "http" && field.key != "PANEL_PUBLIC_URL" {
        let local = url.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .trim_matches(['[', ']'])
                    .parse::<IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        });
        if !local {
            return Err(invalid(field, "panel.config.https_required"));
        }
    }
    if field.key == "PANEL_PUBLIC_URL" && (url.path() != "/" || url.query().is_some()) {
        return Err(invalid(field, "panel.config.invalid_origin"));
    }
    Ok(())
}
