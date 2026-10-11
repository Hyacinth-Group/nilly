use super::{bounded_json, client, Application, BotError, HashMap, PlatformKind};
use reqwest::{Client, Request};
use serde_json::Value;

const INVALID_CREDENTIAL: &str = "panel.setup.probe_credential_invalid";
const INVALID_RESPONSE: &str = "panel.setup.probe_response_invalid";
const REQUEST_FAILED: &str = "panel.setup.probe_request_failed";
const IDENTITY_MISMATCH: &str = "panel.setup.probe_identity_mismatch";

fn credential(key: &str, raw: &str) -> Result<String, BotError> {
    let value = crate::config::validate_value(key, raw)?;
    if value.is_empty()
        || value
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return Err(INVALID_CREDENTIAL.into());
    }
    crate::logging::set_secrets([raw, value.as_str()]).map_err(|_| REQUEST_FAILED)?;
    Ok(value)
}

fn value<'a>(values: &'a HashMap<String, String>, key: &str) -> &'a str {
    values.get(key).map(String::as_str).unwrap_or("")
}

async fn execute(client: &Client, request: Request) -> Result<Value, BotError> {
    let response = client.execute(request).await.map_err(|_| REQUEST_FAILED)?;
    bounded_json(response).await
}

fn discord_request(client: &Client, token: &str) -> Result<Request, BotError> {
    let token = credential("DISCORD_TOKEN", token)?;
    let mut authorization = reqwest::header::HeaderValue::from_str(&format!("Bot {token}"))
        .map_err(|_| INVALID_CREDENTIAL)?;
    authorization.set_sensitive(true);
    client
        .get("https://discord.com/api/v10/oauth2/applications/@me")
        .header(reqwest::header::AUTHORIZATION, authorization)
        .build()
        .map_err(|_| REQUEST_FAILED.into())
}

fn discord_application(body: &Value) -> Result<Application, BotError> {
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .ok_or(INVALID_RESPONSE)?;
    if !id.bytes().all(|byte| byte.is_ascii_digit())
        || !id.parse::<u64>().ok().is_some_and(|id| id != 0)
    {
        return Err(INVALID_RESPONSE.into());
    }
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(id)
        .chars()
        .take(100)
        .collect();
    Ok(Application {
        id: id.to_owned(),
        name,
        invite_url: format!("https://discord.com/oauth2/authorize?client_id={id}&permissions=67226688&scope=bot%20applications.commands"),
        platform: PlatformKind::Discord,
        privacy_mode: None,
        workspace: None,
    })
}

pub(super) async fn discord(token: &str) -> Result<Application, BotError> {
    let client = client()?;
    let request = discord_request(&client, token)?;
    discord_application(&execute(&client, request).await?)
}

fn telegram_request(client: &Client, token: &str) -> Result<Request, BotError> {
    let token = credential("TELEGRAM_TOKEN", token)?;
    let (id, secret) = token.split_once(':').ok_or(INVALID_CREDENTIAL)?;
    // The token occupies one path component, never a caller-supplied host or method.
    if !id.parse::<u64>().ok().is_some_and(|id| id != 0)
        || !id.bytes().all(|byte| byte.is_ascii_digit())
        || secret.is_empty()
        || !secret
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(INVALID_CREDENTIAL.into());
    }
    client
        .get(format!("https://api.telegram.org/bot{token}/getMe"))
        .build()
        .map_err(|_| REQUEST_FAILED.into())
}

fn telegram_application(body: &Value) -> Result<Application, BotError> {
    if body.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(INVALID_RESPONSE.into());
    }
    let result = body.get("result").ok_or(INVALID_RESPONSE)?;
    let id = result
        .get("id")
        .and_then(Value::as_u64)
        .filter(|id| *id != 0)
        .ok_or(INVALID_RESPONSE)?;
    if result.get("is_bot").and_then(Value::as_bool) != Some(true) {
        return Err(INVALID_RESPONSE.into());
    }
    let username = result
        .get("username")
        .and_then(Value::as_str)
        .ok_or(INVALID_RESPONSE)?;
    if !(5..=32).contains(&username.len())
        || !username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        || !username.to_ascii_lowercase().ends_with("bot")
    {
        return Err(INVALID_RESPONSE.into());
    }
    let name = result
        .get("first_name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .ok_or(INVALID_RESPONSE)?;
    let privacy_mode = match result.get("can_read_all_group_messages") {
        None => None,
        Some(Value::Bool(read_all)) => Some(!read_all),
        Some(_) => return Err(INVALID_RESPONSE.into()),
    };
    Ok(Application {
        id: id.to_string(),
        name: name.chars().take(100).collect(),
        invite_url: format!("https://t.me/{username}?startgroup=true"),
        platform: PlatformKind::Telegram,
        privacy_mode,
        workspace: None,
    })
}

fn slack_request(client: &Client, key: &str, token: &str) -> Result<Request, BotError> {
    let token = credential(key, token)?;
    let (prefix, endpoint) = match key {
        "SLACK_BOT_TOKEN" => ("xoxb-", "https://slack.com/api/auth.test"),
        "SLACK_APP_TOKEN" => ("xapp-", "https://slack.com/api/apps.connections.open"),
        _ => return Err(INVALID_CREDENTIAL.into()),
    };
    if !token
        .strip_prefix(prefix)
        .is_some_and(|suffix| !suffix.is_empty())
    {
        return Err(INVALID_CREDENTIAL.into());
    }
    client
        .post(endpoint)
        .bearer_auth(token)
        .build()
        .map_err(|_| REQUEST_FAILED.into())
}

fn slack_id<'a>(body: &'a Value, key: &str, prefix: &[u8]) -> Result<&'a str, BotError> {
    let id = body
        .get(key)
        .and_then(Value::as_str)
        .ok_or(INVALID_RESPONSE)?;
    if id.len() < 2
        || id.len() > 100
        || !prefix.contains(&id.as_bytes()[0])
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    {
        return Err(INVALID_RESPONSE.into());
    }
    Ok(id)
}

fn slack_application(auth: &Value, connection: &Value) -> Result<Application, BotError> {
    if auth.get("ok").and_then(Value::as_bool) != Some(true)
        || connection.get("ok").and_then(Value::as_bool) != Some(true)
    {
        return Err(INVALID_RESPONSE.into());
    }
    let user = slack_id(auth, "user_id", b"UW")?;
    let bot = slack_id(auth, "bot_id", b"B")?;
    let team = slack_id(auth, "team_id", b"T")?;
    // apps.connections.open normally only reports a short-lived socket URL.
    // Validate any supplied identity fields, but do not claim cross-token pairing:
    // the runtime checks the socket hello's team_id before consuming events.
    for (key, expected) in [("team_id", team), ("user_id", user), ("bot_id", bot)] {
        if let Some(reported) = connection.get(key) {
            if reported.as_str() != Some(expected) {
                return Err(IDENTITY_MISMATCH.into());
            }
        }
    }
    let socket = connection
        .get("url")
        .and_then(Value::as_str)
        .ok_or(INVALID_RESPONSE)?;
    let socket = reqwest::Url::parse(socket).map_err(|_| INVALID_RESPONSE)?;
    if socket.scheme() != "wss"
        || !socket
            .host_str()
            .is_some_and(|host| host.ends_with(".slack.com"))
        || !socket.username().is_empty()
        || socket.password().is_some()
        || socket.fragment().is_some()
    {
        return Err(INVALID_RESPONSE.into());
    }
    let name = auth
        .get("user")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(user)
        .chars()
        .take(100)
        .collect();
    Ok(Application {
        id: user.to_owned(),
        name,
        invite_url: String::new(),
        platform: PlatformKind::Slack,
        privacy_mode: None,
        workspace: Some(team.to_owned()),
    })
}

#[cfg(feature = "matrix")]
fn matrix_request(client: &Client, values: &HashMap<String, String>) -> Result<Request, BotError> {
    let homeserver =
        crate::config::validate_value("MATRIX_HOMESERVER", value(values, "MATRIX_HOMESERVER"))?;
    let user = crate::config::validate_value("MATRIX_USER", value(values, "MATRIX_USER"))?;
    if homeserver.is_empty() || user.is_empty() {
        return Err(INVALID_CREDENTIAL.into());
    }
    let token = credential("MATRIX_ACCESS_TOKEN", value(values, "MATRIX_ACCESS_TOKEN"))?;
    let mut url = reqwest::Url::parse(&homeserver).map_err(|_| INVALID_CREDENTIAL)?;
    url.path_segments_mut()
        .map_err(|_| INVALID_CREDENTIAL)?
        .pop_if_empty()
        .extend(["_matrix", "client", "v3", "account", "whoami"]);
    client
        .get(url)
        .bearer_auth(token)
        .build()
        .map_err(|_| REQUEST_FAILED.into())
}

#[cfg(feature = "matrix")]
fn matrix_application(body: &Value, expected: &str) -> Result<Application, BotError> {
    let reported = body
        .get("user_id")
        .and_then(Value::as_str)
        .ok_or(INVALID_RESPONSE)?;
    let user =
        crate::config::validate_value("MATRIX_USER", reported).map_err(|_| INVALID_RESPONSE)?;
    if reported != user {
        return Err(INVALID_RESPONSE.into());
    }
    let expected = crate::config::validate_value("MATRIX_USER", expected)?;
    if user.is_empty() || user != expected {
        return Err(IDENTITY_MISMATCH.into());
    }
    Ok(Application {
        id: user.clone(),
        name: user,
        invite_url: String::new(),
        platform: PlatformKind::Matrix,
        privacy_mode: None,
        workspace: None,
    })
}

pub(super) async fn test(
    platform: PlatformKind,
    values: &HashMap<String, String>,
) -> Result<Application, BotError> {
    if platform == PlatformKind::Discord {
        return discord(value(values, "DISCORD_TOKEN")).await;
    }
    if platform == PlatformKind::Terminal {
        return Err("panel.setup.probe_platform_invalid".into());
    }
    #[cfg(not(feature = "matrix"))]
    if platform == PlatformKind::Matrix {
        return Err("panel.config.matrix_feature_required".into());
    }
    let client = client()?;
    match platform {
        PlatformKind::Telegram => {
            let request = telegram_request(&client, value(values, "TELEGRAM_TOKEN"))?;
            telegram_application(&execute(&client, request).await?)
        }
        PlatformKind::Slack => {
            // Construct both before sending either, so missing/wrong token types fail locally.
            let auth = slack_request(&client, "SLACK_BOT_TOKEN", value(values, "SLACK_BOT_TOKEN"))?;
            let connection =
                slack_request(&client, "SLACK_APP_TOKEN", value(values, "SLACK_APP_TOKEN"))?;
            let auth = execute(&client, auth).await?;
            if auth.get("ok").and_then(Value::as_bool) != Some(true) {
                return Err(INVALID_RESPONSE.into());
            }
            // Validate the authenticated bot before allocating a short-lived socket URL.
            slack_id(&auth, "user_id", b"UW")?;
            slack_id(&auth, "bot_id", b"B")?;
            slack_id(&auth, "team_id", b"T")?;
            let connection = execute(&client, connection).await?;
            slack_application(&auth, &connection)
        }
        #[cfg(feature = "matrix")]
        PlatformKind::Matrix => {
            let request = matrix_request(&client, values)?;
            matrix_application(
                &execute(&client, request).await?,
                value(values, "MATRIX_USER"),
            )
        }
        _ => Err("panel.setup.probe_platform_invalid".into()),
    }
}

#[cfg(test)]
mod tests;
