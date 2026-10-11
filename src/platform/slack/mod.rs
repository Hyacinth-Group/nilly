//! Slack's Web API and outbound Socket Mode session; no SDK objects enter the brain.
pub mod events;
pub mod render;
pub(crate) mod runner;
mod transport;

use super::{types::*, PlatformError, PlatformResult};
use parking_lot::Mutex;
use serde_json::json;
use std::{collections::HashMap, time::Instant};

pub struct SlackAdapter {
    pub(crate) connection: ConnectionId,
    pub(crate) account: UserRef,
    pub(crate) team: String,
    pub(crate) client: reqwest::Client,
    pub(crate) bot_token: String,
    pub(crate) rooms: Mutex<HashMap<RoomKey, RoomRef>>,
    pub(crate) users: Mutex<HashMap<UserKey, UserRef>>,
    pub(crate) scopes: Mutex<std::collections::HashSet<String>>,
    pub(crate) history_gate: tokio::sync::Mutex<Instant>,
    pub(crate) history_spacing: std::time::Duration,
    pub(crate) history_limit: usize,
}
impl SlackAdapter {
    pub(crate) async fn authenticate(
        connection: ConnectionId,
        bot_token: String,
    ) -> PlatformResult<Self> {
        let restricted = std::env::var("SLACK_HISTORY_TIER").as_deref() == Ok("restricted");
        let mut adapter = Self {
            connection,
            account: events::user("pending", "pending")?,
            team: String::new(),
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|_| {
                    PlatformError::Transport("Slack HTTP client initialization failed".into())
                })?,
            bot_token,
            rooms: Mutex::new(HashMap::new()),
            users: Mutex::new(HashMap::new()),
            history_gate: tokio::sync::Mutex::new(Instant::now()),
            scopes: Mutex::new(std::collections::HashSet::new()),
            history_spacing: std::time::Duration::from_secs(if restricted { 60 } else { 1 }),
            history_limit: if restricted { 15 } else { 100 },
        };
        let auth = adapter.api("auth.test", json!({})).await?;
        adapter.team = auth["team_id"]
            .as_str()
            .ok_or_else(|| PlatformError::Transport("Slack auth.test omitted team_id".into()))?
            .into();
        let user_id = auth["user_id"]
            .as_str()
            .ok_or_else(|| PlatformError::Transport("Slack auth.test omitted user_id".into()))?;
        adapter.account = events::user(&adapter.team, user_id)?;
        adapter.account.automation = Automation::Bot;
        adapter.account.display_name = auth["user"].as_str().unwrap_or(user_id).into();
        adapter.account.username = Some(adapter.account.display_name.clone());
        Ok(adapter)
    }

    pub(crate) async fn resolve_user(&self, key: &UserKey) -> PlatformResult<UserRef> {
        if let Some(user) = self.users.lock().get(key).cloned() {
            return Ok(user);
        }
        let user_id = transport::native_user(&self.team, key)?;
        let data = self.api("users.info", json!({"user":user_id})).await?;
        let info = &data["user"];
        let mut user = events::user(&self.team, &user_id)?;
        user.username = info["name"].as_str().map(str::to_owned);
        user.display_name = info["profile"]["display_name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .or_else(|| info["real_name"].as_str())
            .or(user.username.as_deref())
            .unwrap_or(&user_id)
            .into();
        user.automation = if info["is_bot"].as_bool().unwrap_or(false)
            || info["is_app_user"].as_bool().unwrap_or(false)
        {
            Automation::Bot
        } else {
            Automation::Human
        };
        let mut users = self.users.lock();
        if users.len() >= 4096 {
            users.clear();
        }
        users.insert(key.clone(), user.clone());
        Ok(user)
    }
}
