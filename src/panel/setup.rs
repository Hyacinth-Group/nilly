use crate::BotError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct Draft {
    pub values: HashMap<String, String>,
    pub application: Option<Application>,
    pub provider_verified: bool,
    pub expires: Instant,
    pub revision: String,
}

impl Draft {
    pub fn new(config: &crate::config::Config) -> Self {
        let values = [
            "DISCORD_TOKEN",
            "PROVIDER",
            "OPENROUTER_KEY",
            "MISTRAL_KEY",
            "MODEL",
            "BOT_LANG",
            "TZ_OFFSET",
            "OWNER_ID",
            "GUILD_ID",
            "CHANNELS",
            "NEWS_CHANNEL",
            "NEWS_RSS",
        ]
        .into_iter()
        .map(|key| (key.to_string(), config.get(key).to_string()))
        .collect();
        Self {
            values,
            application: None,
            provider_verified: false,
            expires: Instant::now() + Duration::from_secs(900),
            revision: config.revision().to_owned(),
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Application {
    pub id: String,
    pub name: String,
    pub invite_url: String,
}

fn client() -> Result<reqwest::Client, BotError> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()?)
}

async fn bounded_json(mut response: reqwest::Response) -> Result<serde_json::Value, BotError> {
    if !response.status().is_success() {
        return Err("credential test failed".into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len() + chunk.len() > 256 * 1024 {
            return Err("credential response too large".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&body)?)
}

pub async fn test_platform(token: &str) -> Result<Application, BotError> {
    crate::config::validate_value("DISCORD_TOKEN", token)?;
    let response = client()?
        .get("https://discord.com/api/v10/oauth2/applications/@me")
        .header(reqwest::header::AUTHORIZATION, format!("Bot {token}"))
        .send()
        .await?;
    let body = bounded_json(response).await?;
    let id = body
        .get("id")
        .and_then(serde_json::Value::as_str)
        .ok_or("application identity not reported")?;
    let parsed: u64 = id.parse()?;
    if parsed == 0 {
        return Err("invalid application identity".into());
    }
    let name = body
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(id)
        .chars()
        .take(100)
        .collect();
    Ok(Application { id: id.to_string(), name, invite_url: format!("https://discord.com/oauth2/authorize?client_id={id}&permissions=67226688&scope=bot%20applications.commands") })
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
    let body = bounded_json(client.get(url).bearer_auth(key).send().await?).await?;
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
