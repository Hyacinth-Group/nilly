//! Plaintext Matrix rooms only: no crypto store or deceptive plaintext fallback.
pub(crate) mod events;
pub(crate) mod render;
pub(crate) mod runner;
mod transport;

use crate::platform::{capabilities::Capabilities, types::*, PlatformError, PlatformResult};
use matrix_sdk::Client;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

#[derive(Clone)]
pub struct MatrixAdapter {
    client: Client,
    connection: ConnectionId,
    account: UserRef,
    capabilities: Arc<RwLock<HashMap<RoomKey, Capabilities>>>,
    media_http: reqwest::Client,
    image_analysis: bool,
}
impl MatrixAdapter {
    pub(crate) fn new(
        client: Client,
        connection: ConnectionId,
        image_analysis: bool,
    ) -> PlatformResult<Self> {
        let user = client.user_id().ok_or(PlatformError::Disconnected)?;
        let mut account = events::user_ref(user.as_str(), None)?;
        account.automation = Automation::Bot;
        Ok(Self {
            client,
            connection,
            account,
            capabilities: Arc::new(RwLock::new(HashMap::new())),
            media_http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| PlatformError::Transport(e.to_string()))?,
            image_analysis,
        })
    }
}
pub struct MatrixConfig {
    pub homeserver: String,
    pub user: String,
    pub access_token: Option<String>,
    pub password: Option<String>,
    pub rooms: Vec<String>,
}
impl MatrixConfig {
    pub(crate) fn from_settings() -> Result<Option<Self>, crate::BotError> {
        let Some(homeserver) = crate::config::runtime_value("MATRIX_HOMESERVER")
            .ok()
            .filter(|v| !v.trim().is_empty())
        else {
            if [
                "MATRIX_USER",
                "MATRIX_ACCESS_TOKEN",
                "MATRIX_PASSWORD",
                "MATRIX_ROOMS",
            ]
            .iter()
            .any(|key| crate::config::runtime_value(key).is_ok_and(|v| !v.trim().is_empty()))
            {
                return Err("MATRIX_HOMESERVER is missing".into());
            }
            return Ok(None);
        };
        let user = crate::setting("MATRIX_USER")?;
        matrix_sdk::ruma::UserId::parse(&user)?;
        let access_token = crate::setting("MATRIX_ACCESS_TOKEN").ok();
        let password = crate::setting("MATRIX_PASSWORD").ok();
        if access_token.is_none() && password.is_none() {
            return Err("MATRIX_ACCESS_TOKEN or MATRIX_PASSWORD is required".into());
        }
        let rooms = crate::config::runtime_value("MATRIX_ROOMS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        for room in &rooms {
            matrix_sdk::ruma::RoomId::parse(room)?;
        }
        let url = reqwest::Url::parse(homeserver.trim())?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "MATRIX_HOMESERVER must be an HTTP(S) URL without credentials, query or fragment"
                    .into(),
            );
        }
        Ok(Some(Self {
            homeserver: homeserver.trim().into(),
            user,
            access_token,
            password,
            rooms,
        }))
    }
}
