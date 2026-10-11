mod probes;
#[cfg(test)]
mod tests;

use crate::platform::types::PlatformKind;
use crate::BotError;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct Draft {
    pub values: HashMap<String, String>,
    pub application: Option<Application>,
    pub selected_platform: PlatformKind,
    pub platform_attempt: u64,
    pub platform_nonce: [u8; 16],
    pub provider_verified: bool,
    pub fit_completed: bool,
    pub expires: Instant,
    pub revision: String,
}

impl Draft {
    pub fn new(config: &crate::config::Config) -> Self {
        let mut platform_nonce = [0; 16];
        OsRng.fill_bytes(&mut platform_nonce);
        let values = [
            "DISCORD_TOKEN",
            "TELEGRAM_TOKEN",
            "TELEGRAM_ROOMS",
            "SLACK_APP_TOKEN",
            "SLACK_BOT_TOKEN",
            "SLACK_CHANNELS",
            "SLACK_HISTORY_TIER",
            "SLACK_TEST_CHANNEL",
            "MATRIX_HOMESERVER",
            "MATRIX_USER",
            "MATRIX_ACCESS_TOKEN",
            "MATRIX_PASSWORD",
            "MATRIX_ROOMS",
            "OWNER_KEY",
            "PROVIDER",
            "OPENROUTER_KEY",
            "MISTRAL_KEY",
            "MODEL",
            "BOT_LANG",
            "TZ_OFFSET",
            "OWNER_ID",
            "GUILD_ID",
            "CHANNELS",
            "DEBUG_CHANNEL",
            "NEWS_CHANNEL",
            "NEWS_RSS",
        ]
        .into_iter()
        .map(|key| (key.to_string(), config.get(key).to_string()))
        .collect();
        Self {
            values,
            application: None,
            selected_platform: if !config.get("DISCORD_TOKEN").is_empty() {
                PlatformKind::Discord
            } else if !config.get("TELEGRAM_TOKEN").is_empty() {
                PlatformKind::Telegram
            } else if !config.get("SLACK_APP_TOKEN").is_empty()
                || !config.get("SLACK_BOT_TOKEN").is_empty()
            {
                PlatformKind::Slack
            } else if !config.get("MATRIX_HOMESERVER").is_empty()
                || !config.get("MATRIX_USER").is_empty()
                || !config.get("MATRIX_ACCESS_TOKEN").is_empty()
                || !config.get("MATRIX_PASSWORD").is_empty()
            {
                PlatformKind::Matrix
            } else {
                PlatformKind::Telegram
            },
            platform_attempt: 0,
            platform_nonce,
            provider_verified: false,
            fit_completed: false,
            expires: Instant::now() + Duration::from_secs(900),
            revision: config.revision().to_owned(),
        }
    }

    pub fn step(&self) -> usize {
        if !self
            .application
            .as_ref()
            .is_some_and(|application| application.platform == self.selected_platform)
        {
            2
        } else if !self.provider_verified {
            3
        } else if !self.fit_completed {
            4
        } else {
            5
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Application {
    pub id: String,
    pub name: String,
    pub invite_url: String,
    pub platform: PlatformKind,
    pub privacy_mode: Option<bool>,
    pub workspace: Option<String>,
}

fn client() -> Result<reqwest::Client, BotError> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| "panel.setup.probe_request_failed")?)
}

async fn bounded_json(mut response: reqwest::Response) -> Result<serde_json::Value, BotError> {
    if !response.status().is_success() {
        return Err("panel.setup.probe_rejected".into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "panel.setup.probe_request_failed")?
    {
        if body.len() + chunk.len() > 256 * 1024 {
            return Err("panel.setup.probe_response_too_large".into());
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| "panel.setup.probe_response_invalid".into())
}

pub fn parse_platform(value: &str) -> Result<PlatformKind, BotError> {
    match value {
        "discord" => Ok(PlatformKind::Discord),
        "telegram" => Ok(PlatformKind::Telegram),
        "slack" => Ok(PlatformKind::Slack),
        "matrix" => Ok(PlatformKind::Matrix),
        _ => Err("panel.setup.probe_platform_invalid".into()),
    }
}

pub async fn test_chat_platform(
    platform: PlatformKind,
    values: &HashMap<String, String>,
) -> Result<Application, BotError> {
    probes::test(platform, values).await
}

pub async fn test_provider(provider: &str, key: &str, model: &str) -> Result<(), BotError> {
    crate::config::validate_value("PROVIDER", provider)?;
    crate::config::validate_value("MODEL", model)?;
    let client = client()?;
    let url = match provider {
        "openrouter" => {
            crate::config::validate_value("OPENROUTER_KEY", key)?;
            "https://openrouter.ai/api/v1/auth/key"
        }
        "mistral" => {
            crate::config::validate_value("MISTRAL_KEY", key)?;
            "https://api.mistral.ai/v1/models"
        }
        _ => return Err("unsupported provider".into()),
    };
    if key.is_empty() {
        return Err("missing provider key".into());
    }
    crate::logging::set_secrets([key]).map_err(|_| "panel.setup.probe_request_failed")?;
    let body = bounded_json(
        client
            .get(url)
            .bearer_auth(key)
            .send()
            .await
            .map_err(|_| "panel.setup.probe_request_failed")?,
    )
    .await?;
    if provider == "mistral" && !model.is_empty() {
        let present = body
            .get("data")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|models| {
                models
                    .iter()
                    .any(|entry| entry.get("id").and_then(serde_json::Value::as_str) == Some(model))
            });
        if !present {
            return Err("model unavailable".into());
        }
    }
    Ok(())
}
