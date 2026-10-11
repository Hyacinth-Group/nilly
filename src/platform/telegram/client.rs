//! Minimal Bot API client. Errors deliberately omit reqwest URLs (which contain secrets).
use crate::platform::{PlatformError, PlatformResult};
use reqwest::{multipart, Client, Response};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::time::Duration;

pub(crate) struct TelegramClient {
    http: Client,
    token: String,
}
impl TelegramClient {
    pub(crate) fn new(token: String) -> PlatformResult<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(45))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| {
                PlatformError::Transport("Telegram HTTP client initialization failed".into())
            })?;
        Ok(Self { http, token })
    }
    fn endpoint(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{method}", self.token)
    }
    pub(crate) async fn call<T: DeserializeOwned>(
        &self,
        method: &str,
        body: &Value,
    ) -> PlatformResult<T> {
        let response = self
            .http
            .post(self.endpoint(method))
            .json(body)
            .send()
            .await
            .map_err(|_| {
                PlatformError::Transport(format!("Telegram {method}: network request failed"))
            })?;
        self.decode(response, method).await
    }
    pub(crate) async fn upload<T: DeserializeOwned>(
        &self,
        method: &str,
        form: multipart::Form,
    ) -> PlatformResult<T> {
        let response = self
            .http
            .post(self.endpoint(method))
            .multipart(form)
            .send()
            .await
            .map_err(|_| PlatformError::Transport(format!("Telegram {method}: upload failed")))?;
        self.decode(response, method).await
    }
    async fn decode<T: DeserializeOwned>(
        &self,
        response: Response,
        method: &str,
    ) -> PlatformResult<T> {
        let status = response.status();
        if status.as_u16() == 401 {
            return Err(PlatformError::Transport(
                "Telegram authentication failed (HTTP 401 Unauthorized)".into(),
            ));
        }
        let value: Value = response.json().await.map_err(|_| {
            PlatformError::Transport(format!("Telegram {method}: invalid API response"))
        })?;
        if value.get("ok").and_then(Value::as_bool) != Some(true) {
            let code = value
                .get("error_code")
                .and_then(Value::as_u64)
                .unwrap_or(status.as_u16().into());
            return Err(match code {
                401 => PlatformError::Transport(
                    "Telegram authentication failed (HTTP 401 Unauthorized)".into(),
                ),
                403 => PlatformError::PermissionDenied,
                404 => PlatformError::NotFound,
                429 => PlatformError::RateLimited {
                    retry_after: value
                        .pointer("/parameters/retry_after")
                        .and_then(Value::as_u64)
                        .map(Duration::from_secs),
                },
                // Descriptions are untrusted and could echo a token-bearing parameter.
                _ => PlatformError::Transport(format!("Telegram {method}: API error {code}")),
            });
        }
        serde_json::from_value(value.get("result").cloned().unwrap_or(Value::Null))
            .map_err(|_| PlatformError::Transport(format!("Telegram {method}: invalid result")))
    }
    pub(crate) async fn download(&self, path: &str, limit: usize) -> PlatformResult<Vec<u8>> {
        if path.is_empty()
            || path.starts_with('/')
            || path.split('/').any(|part| part == "..")
            || path.contains(['?', '#', ':', '\\'])
        {
            return Err(PlatformError::Transport(
                "Telegram returned an unsafe file path".into(),
            ));
        }
        let url = format!("https://api.telegram.org/file/bot{}/{}", self.token, path);
        let mut response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|_| PlatformError::Transport("Telegram media download failed".into()))?;
        if !response.status().is_success() {
            return Err(PlatformError::Transport(format!(
                "Telegram media HTTP {}",
                response.status().as_u16()
            )));
        }
        if response
            .content_length()
            .is_some_and(|len| len > limit as u64)
        {
            return Err(PlatformError::Transport(
                "Telegram image exceeds download limit".into(),
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| PlatformError::Transport("Telegram media read failed".into()))?
        {
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(PlatformError::Transport(
                    "Telegram image exceeds download limit".into(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires TELEGRAM_TOKEN and contacts the real Telegram Bot API"]
    async fn telegram_live_get_me() {
        let token = std::env::var("TELEGRAM_TOKEN").expect("set TELEGRAM_TOKEN");
        let client = TelegramClient::new(token).unwrap();
        let user: super::super::events::User =
            client.call("getMe", &serde_json::json!({})).await.unwrap();
        assert!(user.is_bot);
        assert!(user.id > 0);
    }
}
