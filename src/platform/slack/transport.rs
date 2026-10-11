//! Permission-sensitive Slack Web API transport; never retain authenticated URLs.
use super::{events, render, SlackAdapter};
use crate::platform::{
    capabilities::*, outgoing::*, types::*, Platform, PlatformError, PlatformResult,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::{Response, StatusCode, Url};
use serde_json::{json, Value};
use std::{
    borrow::Cow,
    collections::HashSet,
    time::{Duration, Instant},
};
use tokio::io::AsyncReadExt;

const MEDIA_LIMIT: usize = 10 * 1024 * 1024;
const API_LIMIT: usize = 2 * 1024 * 1024;

fn transport(message: &'static str) -> PlatformError {
    PlatformError::Transport(message.into())
}

fn retry_after(response: &Response) -> Option<Duration> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
}

fn http_status(response: &Response) -> PlatformResult<()> {
    match response.status() {
        StatusCode::UNAUTHORIZED => Err(transport("Slack authentication failed")),
        StatusCode::FORBIDDEN => Err(PlatformError::PermissionDenied),
        StatusCode::NOT_FOUND => Err(PlatformError::NotFound),
        StatusCode::TOO_MANY_REQUESTS => Err(PlatformError::RateLimited {
            retry_after: retry_after(response),
        }),
        status if status.is_success() => Ok(()),
        _ => Err(transport("Slack HTTP request failed")),
    }
}

async fn bounded_bytes(mut response: Response, limit: usize) -> PlatformResult<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err(transport("Slack response exceeds size limit"));
    }
    let capacity = response.content_length().unwrap_or(0).min(limit as u64) as usize;
    let mut data = Vec::with_capacity(capacity);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| transport("Slack response read failed"))?
    {
        if chunk.len() > limit - data.len() {
            return Err(transport("Slack response exceeds size limit"));
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

fn slack_error(value: &Value) -> PlatformError {
    match value["error"].as_str().unwrap_or("") {
        "invalid_auth" => transport("Slack API: invalid_auth"),
        "not_authed" => transport("Slack API: not_authed"),
        "token_revoked" => transport("Slack API: token_revoked"),
        "token_expired" => transport("Slack API: token_expired"),
        "account_inactive" => transport("Slack API: account_inactive"),
        "missing_scope"
        | "no_permission"
        | "not_in_channel"
        | "restricted_action"
        | "is_archived"
        | "cant_update_message"
        | "cant_delete_message" => PlatformError::PermissionDenied,
        "channel_not_found" | "message_not_found" | "file_not_found" | "user_not_found" => {
            PlatformError::NotFound
        }
        "ratelimited" => PlatformError::RateLimited {
            retry_after: value["retry_after"].as_u64().map(Duration::from_secs),
        },
        _ => transport("Slack API rejected request"),
    }
}

pub(super) fn native_id(value: &str, prefixes: &[u8]) -> bool {
    value.len() >= 2
        && value.len() <= 64
        && prefixes.contains(&value.as_bytes()[0])
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

pub(super) fn native_user(team: &str, key: &UserKey) -> PlatformResult<String> {
    let mut parts = key.components();
    let workspace = parts.next().ok_or(KeyError)??;
    let user = parts.next().ok_or(KeyError)??;
    if key.platform() != PlatformKind::Slack
        || workspace != team
        || !native_id(&user, b"UW")
        || parts.next().is_some()
    {
        return Err(KeyError.into());
    }
    Ok(user)
}

fn timestamp(value: &str) -> bool {
    let Some((seconds, fraction)) = value.split_once('.') else {
        return false;
    };
    !seconds.is_empty()
        && seconds.len() <= 12
        && seconds.bytes().all(|byte| byte.is_ascii_digit())
        && fraction.len() == 6
        && fraction.bytes().all(|byte| byte.is_ascii_digit())
}

fn native_message(team: &str, channel: &str, key: &MessageKey) -> PlatformResult<String> {
    let mut parts = key.components();
    let workspace = parts.next().ok_or(KeyError)??;
    let native_channel = parts.next().ok_or(KeyError)??;
    let ts = parts.next().ok_or(KeyError)??;
    if key.platform() != PlatformKind::Slack
        || workspace != team
        || native_channel != channel
        || !timestamp(&ts)
        || parts.next().is_some()
    {
        return Err(KeyError.into());
    }
    Ok(ts)
}

fn safe_file_url(value: &str, upload: bool) -> PlatformResult<Url> {
    let url = Url::parse(value).map_err(|_| transport("Slack file URL is invalid"))?;
    let path = if upload { "/upload/" } else { "/files-pri/" };
    if url.scheme() != "https"
        || url.host_str() != Some("files.slack.com")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !url.path().starts_with(path)
    {
        return Err(transport("Slack file URL is not allowed"));
    }
    Ok(url)
}

struct PreparedFile {
    id: String,
    title: String,
}

impl SlackAdapter {
    pub(crate) async fn api(&self, method: &str, mut body: Value) -> PlatformResult<Value> {
        if !method
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.')
            || method.is_empty()
        {
            return Err(transport("Slack API method is invalid"));
        }
        // Form transport is accepted by every Web API method, including file
        // methods that do not accept JSON bodies. Nested arguments (notably the
        // external-upload files list) are JSON-valued form fields.
        let arguments = body
            .as_object_mut()
            .ok_or_else(|| transport("Slack API arguments are invalid"))?;
        for value in arguments.values_mut() {
            if value.is_array() || value.is_object() {
                *value = Value::String(
                    serde_json::to_string(value)
                        .map_err(|_| transport("Slack API arguments are invalid"))?,
                );
            }
        }
        let response = self
            .client
            .post(format!("https://slack.com/api/{method}"))
            .bearer_auth(&self.bot_token)
            .form(&body)
            .send()
            .await
            .map_err(|_| transport("Slack API connection failed"))?;
        if let Err(error) = http_status(&response) {
            self.defer_history(method, &error).await;
            return Err(error);
        }
        let granted = response
            .headers()
            .get("x-oauth-scopes")
            .and_then(|header| header.to_str().ok())
            .map(|header| {
                header
                    .split(',')
                    .map(str::trim)
                    .filter(|scope| !scope.is_empty())
                    .map(str::to_owned)
                    .collect::<HashSet<_>>()
            });
        let retry = retry_after(&response);
        let value: Value = serde_json::from_slice(&bounded_bytes(response, API_LIMIT).await?)
            .map_err(|_| transport("Slack API response is invalid"))?;
        if value["ok"].as_bool() != Some(true) {
            let mut error = slack_error(&value);
            if let PlatformError::RateLimited { retry_after } = &mut error {
                *retry_after = retry.or(*retry_after);
            }
            self.defer_history(method, &error).await;
            return Err(error);
        }
        if let Some(granted) = granted {
            *self.scopes.lock() = granted;
        }
        Ok(value)
    }

    fn channel(&self, room: &RoomRef) -> PlatformResult<String> {
        let mut parts = room.id.components();
        let workspace = parts.next().ok_or(KeyError)??;
        let channel = parts.next().ok_or(KeyError)??;
        if room.id.platform() != PlatformKind::Slack
            || workspace != self.team
            || !native_id(&channel, b"CGD")
            || parts.next().is_some()
        {
            return Err(KeyError.into());
        }
        if let Some(space) = &room.space {
            if space != &SpaceKey::new(PlatformKind::Slack, &[&self.team])? {
                return Err(KeyError.into());
            }
        }
        if let Some(thread) = &room.thread {
            let mut parts = thread.components();
            let workspace = parts.next().ok_or(KeyError)??;
            let thread_channel = parts.next().ok_or(KeyError)??;
            let ts = parts.next().ok_or(KeyError)??;
            if thread.platform() != PlatformKind::Slack
                || workspace != self.team
                || thread_channel != channel
                || !timestamp(&ts)
                || parts.next().is_some()
            {
                return Err(KeyError.into());
            }
        }
        Ok(channel)
    }

    fn check_receipt(&self, receipt: &MessageReceipt) -> PlatformResult<(String, String)> {
        if receipt.connection != self.connection || receipt.message.room != receipt.room.id {
            return Err(KeyError.into());
        }
        if receipt.message.author.as_ref() != Some(&self.account.id) {
            return Err(PlatformError::PermissionDenied);
        }
        let channel = self.channel(&receipt.room)?;
        let ts = native_message(&self.team, &channel, &receipt.message.id)?;
        Ok((channel, ts))
    }

    fn thread_ts(
        &self,
        room: &RoomRef,
        body: &OutgoingMessage<'_>,
    ) -> PlatformResult<Option<String>> {
        let channel = self.channel(room)?;
        let reply = body
            .reply_to
            .map(|target| {
                if target.room != room.id {
                    return Err(KeyError.into());
                }
                native_message(&self.team, &channel, &target.id)
            })
            .transpose()?;
        if let Some(thread) = &room.thread {
            return Ok(Some(thread.components().nth(2).ok_or(KeyError)??));
        }
        Ok(reply)
    }
    /// reactions.get addresses an exact message, including replies absent from
    /// conversations.history, using the installed app's reactions:read scope.
    pub(crate) async fn enrich_reaction(&self, reaction: &mut ReactionEvent) -> PlatformResult<()> {
        let channel = self.channel(&reaction.room)?;
        if reaction.target.room != reaction.room.id {
            return Err(KeyError.into());
        }
        let ts = native_message(&self.team, &channel, &reaction.target.id)?;
        let data = self
            .api(
                "reactions.get",
                json!({"channel":channel,"timestamp":ts,"full":false}),
            )
            .await?;
        self.apply_reaction_context(reaction, &data)
    }

    fn apply_reaction_context(
        &self,
        reaction: &mut ReactionEvent,
        data: &Value,
    ) -> PlatformResult<()> {
        let channel = self.channel(&reaction.room)?;
        let ts = native_message(&self.team, &channel, &reaction.target.id)?;
        let message = &data["message"];
        if reaction.target.room != reaction.room.id
            || data["type"].as_str() != Some("message")
            || data["channel"].as_str() != Some(channel.as_str())
            || message["ts"].as_str() != Some(ts.as_str())
        {
            return Err(transport("Slack returned a different reaction target"));
        }
        let author = message["user"]
            .as_str()
            .ok_or_else(|| transport("Slack reaction target author missing"))?;
        let author = UserKey::new(PlatformKind::Slack, &[&self.team, author])?;
        native_user(&self.team, &author)?;
        let text = message["text"]
            .as_str()
            .ok_or_else(|| transport("Slack reaction target text missing"))?;
        let (text, _) = events::normalize_text(&self.team, text)?;
        let mut room = reaction.room.clone();
        room.thread = message["thread_ts"]
            .as_str()
            .map(|thread| ThreadKey::new(PlatformKind::Slack, &[&self.team, &channel, thread]))
            .transpose()?;
        self.channel(&room)?;
        reaction.room = room;
        reaction.target.author = Some(author);
        reaction.original_text = Some(text);
        Ok(())
    }

    pub(crate) async fn reserve_history(&self) {
        // Wait for a snapshot, then recheck before reserving. A 429 may extend the
        // deadline while another caller sleeps; no lock survives sleep or HTTP.
        loop {
            let deadline = {
                let mut next = self.history_gate.lock().await;
                let now = Instant::now();
                if *next <= now {
                    *next = now + self.history_spacing;
                    return;
                }
                *next
            };
            tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
        }
    }

    async fn defer_history(&self, method: &str, error: &PlatformError) {
        if method != "conversations.history" {
            return;
        }
        if let PlatformError::RateLimited {
            retry_after: Some(delay),
        } = error
        {
            if let Some(until) = Instant::now().checked_add(*delay) {
                let mut next = self.history_gate.lock().await;
                *next = (*next).max(until);
            }
        }
    }

    async fn upload(&self, source: &OutgoingAttachment<'_>) -> PlatformResult<PreparedFile> {
        let (filename, data): (Cow<'_, str>, Cow<'_, [u8]>) = match source {
            OutgoingAttachment::Bytes { filename, data, .. } => {
                (Cow::Borrowed(*filename), Cow::Borrowed(*data))
            }
            OutgoingAttachment::LocalPath(path) => {
                let filename = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| transport("Slack attachment filename is invalid"))?;
                let file = tokio::fs::File::open(path)
                    .await
                    .map_err(|_| transport("Slack attachment could not be opened"))?;
                let metadata = file
                    .metadata()
                    .await
                    .map_err(|_| transport("Slack attachment metadata unavailable"))?;
                if !metadata.is_file() || metadata.len() > MEDIA_LIMIT as u64 {
                    return Err(transport(
                        "Slack attachment exceeds size limit or is not a file",
                    ));
                }
                let mut data = Vec::with_capacity(metadata.len() as usize);
                file.take((MEDIA_LIMIT + 1) as u64)
                    .read_to_end(&mut data)
                    .await
                    .map_err(|_| transport("Slack attachment read failed"))?;
                (Cow::Borrowed(filename), Cow::Owned(data))
            }
        };
        if data.is_empty()
            || data.len() > MEDIA_LIMIT
            || filename.is_empty()
            || filename.len() > 255
            || filename.chars().any(char::is_control)
            || filename.contains(['/', '\\'])
        {
            return Err(transport(
                "Slack attachment is invalid or exceeds size limit",
            ));
        }
        let allocated = self
            .api(
                "files.getUploadURLExternal",
                json!({"filename":filename,"length":data.len()}),
            )
            .await?;
        let url = safe_file_url(
            allocated["upload_url"]
                .as_str()
                .ok_or_else(|| transport("Slack upload URL missing"))?,
            true,
        )?;
        let id = allocated["file_id"]
            .as_str()
            .filter(|id| native_id(id, b"F"))
            .ok_or_else(|| transport("Slack upload file ID missing"))?
            .to_owned();
        // reqwest owns the upload buffer; Bytes inputs are copied only at this I/O boundary.
        let response = self
            .client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(data.into_owned())
            .send()
            .await
            .map_err(|_| transport("Slack upload failed"))?;
        http_status(&response)?;
        bounded_bytes(response, API_LIMIT).await?;
        Ok(PreparedFile {
            id,
            title: filename.into_owned(),
        })
    }
}

impl Platform for SlackAdapter {
    fn connection_id(&self) -> &ConnectionId {
        &self.connection
    }
    fn self_user(&self) -> &UserRef {
        &self.account
    }

    fn capabilities(&self, room: &RoomRef) -> Capabilities {
        let mut caps = Capabilities::unavailable();
        caps.output = OutputLimits {
            text: render::RAW_LIMIT,
            count_unit: CountUnit::UnicodeScalars,
        };
        caps.edit_cadence = Duration::from_millis(1200);
        if self.channel(room).is_err() {
            return caps;
        }
        let known = self.rooms.lock().get(&room.id).cloned();
        let Some(known) = known else {
            return caps;
        };
        let scopes = self.scopes.lock();
        let readable = scopes.contains(if known.kind == RoomKind::Group {
            "groups:history"
        } else {
            "channels:history"
        });
        let send = scopes.contains("chat:write");
        caps.visibility = if readable {
            MessageVisibility::Ordinary
        } else {
            MessageVisibility::MentionsOnly
        };
        caps.history = if readable && room.thread.is_none() {
            HistoryAccess::Bounded
        } else {
            HistoryAccess::None
        };
        caps.decryption_available = readable;
        caps.send = send;
        caps.send_attachments = send && scopes.contains("files:write");
        caps.replies = send;
        caps.threads = send;
        caps.react = readable && scopes.contains("reactions:write");
        caps.observe_reactions = readable && scopes.contains("reactions:read");
        caps.edit = send;
        caps.delete = send;
        caps.private_results = send;
        caps.upload_bytes = (send && scopes.contains("files:write")).then_some(MEDIA_LIMIT as u64);
        caps
    }
    fn route_reply(
        &self,
        mut room: RoomRef,
        reply_to: Option<&MessageKey>,
    ) -> PlatformResult<RoomRef> {
        let channel = self.channel(&room)?;
        let reply = reply_to
            .map(|target| native_message(&self.team, &channel, target))
            .transpose()?;
        if room.thread.is_none() {
            if let Some(ts) = reply {
                room.thread = Some(ThreadKey::new(
                    PlatformKind::Slack,
                    &[&self.team, &channel, &ts],
                )?);
            }
        }
        Ok(room)
    }

    fn render_stream(
        &self,
        _room: &RoomRef,
        mode: ThoughtPresentation,
        thought: &str,
        text: &str,
        done: bool,
    ) -> Vec<String> {
        render::stream(mode, thought, text, done)
    }
    fn render_final(
        &self,
        _room: &RoomRef,
        mode: ThoughtPresentation,
        thought: &str,
        lines: &[String],
    ) -> Vec<String> {
        render::layout(mode, thought, lines)
    }

    async fn send<'a>(
        &'a self,
        room: &'a RoomRef,
        body: &'a OutgoingMessage<'a>,
    ) -> PlatformResult<MessageReceipt> {
        let channel = self.channel(room)?;
        let caps = self.capabilities(room);
        if !caps.send {
            return Err(PlatformError::PermissionDenied);
        }
        if !body.attachments.is_empty() && !caps.send_attachments {
            return Err(PlatformError::PermissionDenied);
        }
        if body.attachments.len() > 10 {
            return Err(PlatformError::Unsupported("Slack attachment count limit"));
        }
        let text = render::outgoing_text(&self.team, body)?;
        let thread = self.thread_ts(room, body)?;
        if text.trim().is_empty() && body.attachments.is_empty() {
            return Err(transport("Slack message is empty"));
        }
        let mut files = Vec::with_capacity(body.attachments.len());
        for attachment in body.attachments {
            files.push(self.upload(attachment).await?);
        }
        let mut request = render::message_payload(&channel, &text);
        if let Some(thread) = &thread {
            request["thread_ts"] = json!(thread);
        }
        // A file-only send still needs an editable, deletable root message receipt.
        if text.trim().is_empty() {
            request["text"] = json!("📎");
        }
        let sent = self.api("chat.postMessage", request).await?;
        if sent["channel"].as_str() != Some(channel.as_str()) {
            return Err(transport("Slack returned a different message channel"));
        }
        let ts = sent["ts"]
            .as_str()
            .filter(|ts| timestamp(ts))
            .ok_or_else(|| transport("Slack message timestamp missing"))?;
        if !files.is_empty() {
            let uploads: Vec<_> = files
                .into_iter()
                .map(|file| json!({"id":file.id,"title":file.title}))
                .collect();
            // Slack cannot attach external uploads to chat.postMessage. Share them
            // in the receipt's thread, retaining the root's ordinary edit/delete semantics.
            self.api("files.completeUploadExternal", json!({"files":uploads,"channel_id":channel,"thread_ts":thread.as_deref().unwrap_or(ts)})).await?;
        }
        Ok(MessageReceipt {
            connection: self.connection.clone(),
            message: MessageRef {
                id: MessageKey::new(PlatformKind::Slack, &[&self.team, &channel, ts])?,
                room: room.id.clone(),
                author: Some(self.account.id.clone()),
            },
            room: room.clone(),
            last_accepted_text: body.text.to_string(),
            version: 0,
        })
    }

    async fn edit<'a>(
        &'a self,
        receipt: &'a MessageReceipt,
        body: &'a OutgoingMessage<'a>,
    ) -> PlatformResult<()> {
        let (channel, ts) = self.check_receipt(receipt)?;
        if !self.capabilities(&receipt.room).edit {
            return Err(PlatformError::PermissionDenied);
        }
        if !body.attachments.is_empty() {
            return Err(PlatformError::Unsupported("Slack attachment edits"));
        }
        self.thread_ts(&receipt.room, body)?;
        let text = render::outgoing_text(&self.team, body)?;
        if text.trim().is_empty() {
            return Err(transport("Slack message is empty"));
        }
        let mut request = render::message_payload(&channel, &text);
        request["ts"] = json!(ts);
        self.api("chat.update", request).await?;
        Ok(())
    }

    async fn delete<'a>(&'a self, receipt: &'a MessageReceipt) -> PlatformResult<()> {
        let (channel, ts) = self.check_receipt(receipt)?;
        if !self.capabilities(&receipt.room).delete {
            return Err(PlatformError::PermissionDenied);
        }
        self.api("chat.delete", json!({"channel":channel,"ts":ts}))
            .await?;
        Ok(())
    }

    async fn react<'a>(&'a self, target: &'a MessageRef, emoji: &'a Emoji) -> PlatformResult<()> {
        let room = RoomRef::from_key(target.room.clone());
        let channel = self.channel(&room)?;
        let ts = native_message(&self.team, &channel, &target.id)?;
        if !self.capabilities(&room).react {
            return Err(PlatformError::PermissionDenied);
        }
        let name = reaction_name(emoji)?;
        let result = self
            .api(
                "reactions.add",
                json!({"channel":channel,"timestamp":ts,"name":name}),
            )
            .await;
        result.map(|_| ())
    }

    async fn typing<'a>(&'a self, _room: &'a RoomRef) -> PlatformResult<()> {
        Err(PlatformError::Unsupported("Slack typing indicator"))
    }
    async fn set_nickname<'a>(
        &'a self,
        _target: NickTarget<'a>,
        _name: &'a str,
    ) -> PlatformResult<()> {
        Err(PlatformError::Unsupported("Slack per-channel nickname"))
    }

    async fn fetch_history<'a>(
        &'a self,
        room: &'a RoomRef,
        page: HistoryRequest,
    ) -> PlatformResult<HistoryPage> {
        let channel = self.channel(room)?;
        if room.thread.is_some() {
            return Err(PlatformError::Unsupported(
                "Slack bot thread history requires conversations.replies user-token scopes",
            ));
        }
        if self.capabilities(room).history == HistoryAccess::None {
            return Err(PlatformError::PermissionDenied);
        }
        if page.limit == 0 {
            return Err(PlatformError::Unsupported("zero-sized history page"));
        }
        let mut request = json!({"channel":channel,"limit":page.limit.min(self.history_limit)});
        if let Some(cursor) = &page.cursor {
            if cursor.len() > 2048 || cursor.chars().any(char::is_control) {
                return Err(transport("Slack history cursor is invalid"));
            }
            request["cursor"] = json!(cursor);
        }
        if let Some(cutoff) = page.cutoff_unix_ms {
            if cutoff < 0 {
                return Err(transport("Slack history cutoff is invalid"));
            }
            request["oldest"] = json!(format!("{}.{:06}", cutoff / 1000, (cutoff % 1000) * 1000));
            request["inclusive"] = json!(true);
        }
        self.reserve_history().await;
        let mut data = self.api("conversations.history", request).await?;
        let native = data["messages"]
            .as_array_mut()
            .ok_or_else(|| transport("Slack history messages missing"))?;
        let mut messages = Vec::with_capacity(native.len());
        for value in native {
            // The base conversation must never import unrelated thread replies.
            if value["thread_ts"]
                .as_str()
                .is_some_and(|thread| Some(thread) != value["ts"].as_str())
            {
                continue;
            }
            value["type"] = json!("message");
            value["channel"] = json!(channel);
            if let Some(ChatEvent::MessageCreated(mut message)) =
                events::decode(&self.connection, &self.team, &self.account.id, value)?
            {
                if page
                    .cutoff_unix_ms
                    .is_some_and(|cutoff| message.sent_at_unix_ms < cutoff)
                {
                    continue;
                }
                if matches!(
                    message.author.automation,
                    Automation::Bot | Automation::Integration
                ) {
                    continue;
                }
                message.room = room.clone();
                message.author = self.resolve_user(&message.author.id).await?;
                if message.author.automation == Automation::Human {
                    messages.push(message);
                }
            }
        }
        let next_cursor = data["response_metadata"]["next_cursor"]
            .as_str()
            .filter(|cursor| !cursor.is_empty())
            .map(str::to_owned);
        Ok(HistoryPage {
            messages,
            next_cursor,
            limitations: vec![HistoryLimitation::Bounded],
            order: HistoryOrder::NewestFirst,
        })
    }

    async fn list_rooms(&self) -> PlatformResult<Vec<RoomRef>> {
        let mut rooms = Vec::new();
        let mut cursor = None::<String>;
        let mut seen = HashSet::new();
        loop {
            let mut request = json!({"types":"public_channel,private_channel","exclude_archived":true,"limit":200});
            if let Some(cursor) = &cursor {
                request["cursor"] = json!(cursor);
            }
            let data = self.api("conversations.list", request).await?;
            let channels = data["channels"]
                .as_array()
                .ok_or_else(|| transport("Slack channel list missing"))?;
            for channel in channels {
                if channel["is_member"].as_bool() != Some(true)
                    || channel["is_archived"].as_bool() == Some(true)
                {
                    continue;
                }
                let id = channel["id"]
                    .as_str()
                    .ok_or_else(|| transport("Slack channel ID missing"))?;
                if !native_id(id, b"CG") {
                    return Err(KeyError.into());
                }
                let mut room = events::room(&self.team, id)?;
                room.name = channel["name"].as_str().map(str::to_owned);
                room.kind = if channel["is_private"].as_bool() == Some(true) {
                    RoomKind::Group
                } else {
                    RoomKind::Channel
                };
                rooms.push(room);
            }
            cursor = data["response_metadata"]["next_cursor"]
                .as_str()
                .filter(|cursor| !cursor.is_empty())
                .map(str::to_owned);
            let Some(next) = &cursor else {
                break;
            };
            if !seen.insert(next.clone()) {
                return Err(transport("Slack room pagination repeated cursor"));
            }
        }
        rooms.sort_by(|left, right| left.id.cmp(&right.id));
        rooms.dedup_by(|left, right| left.id == right.id);
        *self.rooms.lock() = rooms
            .iter()
            .map(|room| (room.id.clone(), room.clone()))
            .collect();
        Ok(rooms)
    }

    async fn resolve_media<'a>(
        &'a self,
        attachment: &'a AttachmentRef,
    ) -> PlatformResult<ModelMedia> {
        let MediaSource::Native { connection, id } = &attachment.source else {
            return Err(PlatformError::Unsupported("non-native Slack media"));
        };
        if connection != &self.connection || !native_id(id, b"F") {
            return Err(KeyError.into());
        }
        if attachment
            .size
            .is_some_and(|size| size > MEDIA_LIMIT as u64)
        {
            return Err(transport("Slack media exceeds size limit"));
        }
        let info = self.api("files.info", json!({"file":id})).await?;
        let file = &info["file"];
        if file["id"].as_str() != Some(id.as_str()) {
            return Err(transport("Slack returned a different file"));
        }
        if file["size"]
            .as_u64()
            .is_some_and(|size| size > MEDIA_LIMIT as u64)
        {
            return Err(transport("Slack media exceeds size limit"));
        }
        let mime = file["mimetype"]
            .as_str()
            .ok_or_else(|| transport("Slack media MIME type missing"))?;
        if !mime.starts_with("image/")
            || mime.len() > 100
            || !mime.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'.' | b'+')
            })
        {
            return Err(PlatformError::Unsupported("Slack non-image model media"));
        }
        let url = file["url_private_download"]
            .as_str()
            .or_else(|| file["url_private"].as_str())
            .ok_or_else(|| transport("Slack private file URL missing"))?;
        let url = safe_file_url(url, false)?;
        let response = self
            .client
            .get(url)
            .bearer_auth(&self.bot_token)
            .send()
            .await
            .map_err(|_| transport("Slack media download failed"))?;
        http_status(&response)?;
        let bytes = bounded_bytes(response, MEDIA_LIMIT).await?;
        if bytes.is_empty() {
            return Err(transport("Slack media download is empty"));
        }
        Ok(ModelMedia::DataUrl(format!(
            "data:{mime};base64,{}",
            STANDARD.encode(bytes)
        )))
    }
}

fn reaction_name(emoji: &Emoji) -> PlatformResult<&str> {
    match emoji {
        Emoji::Custom {
            platform: PlatformKind::Slack,
            id,
            ..
        } => {
            if id.is_empty()
                || id.len() > 100
                || !id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'+'))
            {
                return Err(KeyError.into());
            }
            Ok(id)
        }
        Emoji::Custom { .. } => Err(PlatformError::Unsupported("foreign custom emoji")),
        Emoji::Unicode(value) => match value.as_str() {
            "👍" => Ok("thumbsup"),
            "👎" => Ok("thumbsdown"),
            "❤️" | "❤" => Ok("heart"),
            "😀" => Ok("grinning"),
            "😃" => Ok("smiley"),
            "😄" => Ok("smile"),
            "😁" => Ok("grin"),
            "😂" => Ok("joy"),
            "🤣" => Ok("rolling_on_the_floor_laughing"),
            "😊" => Ok("blush"),
            "😍" => Ok("heart_eyes"),
            "🤔" => Ok("thinking_face"),
            "😢" => Ok("cry"),
            "😭" => Ok("sob"),
            "😎" => Ok("sunglasses"),
            "🙂" => Ok("slightly_smiling_face"),
            "🙃" => Ok("upside_down_face"),
            "🎉" => Ok("tada"),
            "🔥" => Ok("fire"),
            "💀" => Ok("skull"),
            "👀" => Ok("eyes"),
            "✅" => Ok("white_check_mark"),
            "❌" => Ok("x"),
            "🙏" => Ok("pray"),
            "👏" => Ok("clap"),
            "💯" => Ok("100"),
            "🚀" => Ok("rocket"),
            "🤖" => Ok("robot_face"),
            "✨" => Ok("sparkles"),
            "💙" => Ok("blue_heart"),
            "💚" => Ok("green_heart"),
            "💛" => Ok("yellow_heart"),
            "💜" => Ok("purple_heart"),
            _ => Err(PlatformError::Unsupported(
                "unmapped Slack Unicode reaction",
            )),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use std::collections::HashMap;

    fn adapter() -> SlackAdapter {
        SlackAdapter {
            connection: ConnectionId::new("test-slack"),
            account: events::user("T123", "U123").unwrap(),
            team: "T123".into(),
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            bot_token: "not-a-token".into(),
            rooms: Mutex::new(HashMap::new()),
            users: Mutex::new(HashMap::new()),
            scopes: Mutex::new(HashSet::new()),
            history_gate: tokio::sync::Mutex::new(Instant::now()),
            history_spacing: Duration::from_secs(1),
            history_limit: 100,
        }
    }

    #[test]
    fn message_keys_enforce_workspace_channel_and_timestamp() {
        let correct =
            MessageKey::new(PlatformKind::Slack, &["T123", "C123", "1700000000.000123"]).unwrap();
        assert!(native_message("T123", "C123", &correct).is_ok());
        assert!(native_message("T999", "C123", &correct).is_err());
        assert!(native_message("T123", "C999", &correct).is_err());
        let invalid = MessageKey::new(PlatformKind::Slack, &["T123", "C123", "NaN"]).unwrap();
        assert!(native_message("T123", "C123", &invalid).is_err());
    }

    #[test]
    fn receipt_author_connection_and_embedded_channel_are_checked() {
        let adapter = adapter();
        let room = events::room("T123", "C123").unwrap();
        let mut receipt = MessageReceipt {
            connection: adapter.connection.clone(),
            room: room.clone(),
            message: MessageRef {
                id: MessageKey::new(PlatformKind::Slack, &["T123", "C123", "1700000000.000123"])
                    .unwrap(),
                room: room.id.clone(),
                author: Some(adapter.account.id.clone()),
            },
            last_accepted_text: String::new(),
            version: 0,
        };
        assert!(adapter.check_receipt(&receipt).is_ok());
        receipt.connection = ConnectionId::new("other");
        assert!(adapter.check_receipt(&receipt).is_err());
        receipt.connection = adapter.connection.clone();
        receipt.message.id =
            MessageKey::new(PlatformKind::Slack, &["T123", "C999", "1700000000.000123"]).unwrap();
        assert!(adapter.check_receipt(&receipt).is_err());
    }

    #[test]
    fn resolved_reply_routes_every_line_chunk_and_thought_answer_part_to_one_thread() {
        let adapter = adapter();
        let room = events::room("T123", "C123").unwrap();
        let target = MessageRef {
            id: MessageKey::new(PlatformKind::Slack, &["T123", "C123", "1700000000.000123"])
                .unwrap(),
            room: room.id.clone(),
            author: None,
        };
        let routed = adapter.route_reply(room.clone(), Some(&target.id)).unwrap();
        assert!(room.thread.is_none());
        let long_answer = "a".repeat(10_000);
        let lines = vec!["first answer".to_owned(), long_answer];
        let live = adapter.render_stream(
            &routed,
            ThoughtPresentation::Show,
            "thinking about it",
            "first answer\nsecond answer\n",
            false,
        );
        let final_layout = adapter.render_final(
            &routed,
            ThoughtPresentation::Show,
            "thinking about it",
            &lines,
        );
        assert!(live.len() >= 3);
        assert!(final_layout.len() > lines.len());
        for layout in [&live, &final_layout] {
            for (index, part) in layout.iter().enumerate() {
                let mut body = OutgoingMessage::text(part);
                body.reply_to = (index == 0).then_some(&target);
                let thread = adapter.thread_ts(&routed, &body).unwrap();
                assert_eq!(thread.as_deref(), Some("1700000000.000123"));
                let mut payload = render::message_payload("C123", part);
                payload["thread_ts"] = json!(thread.unwrap());
                assert_eq!(payload["thread_ts"], "1700000000.000123");
            }
        }
        // No room-global last-target state: concurrent replies resolve independently.
        let other =
            MessageKey::new(PlatformKind::Slack, &["T123", "C123", "1700000001.000456"]).unwrap();
        let other_room = adapter.route_reply(room.clone(), Some(&other)).unwrap();
        assert_ne!(routed.thread, other_room.thread);
        let no_target = OutgoingMessage::text("later part");
        assert_eq!(
            adapter.thread_ts(&routed, &no_target).unwrap().as_deref(),
            Some("1700000000.000123")
        );
        assert_eq!(
            adapter
                .thread_ts(&other_room, &no_target)
                .unwrap()
                .as_deref(),
            Some("1700000001.000456")
        );
        assert!(adapter.route_reply(room, None).unwrap().thread.is_none());
    }

    #[test]
    fn reply_route_preserves_existing_thread_and_rejects_foreign_targets() {
        let adapter = adapter();
        let mut room = events::room("T123", "C123").unwrap();
        room.thread = Some(
            ThreadKey::new(PlatformKind::Slack, &["T123", "C123", "1700000000.000123"]).unwrap(),
        );
        let reply =
            MessageKey::new(PlatformKind::Slack, &["T123", "C123", "1700000002.000001"]).unwrap();
        let routed = adapter.route_reply(room.clone(), Some(&reply)).unwrap();
        assert_eq!(routed.conversation(), room.conversation());
        assert_eq!(routed.space, room.space);
        assert_eq!(routed.kind, room.kind);
        assert_eq!(routed.name, room.name);
        for components in [
            ["T999", "C123", "1700000002.000001"],
            ["T123", "C999", "1700000002.000001"],
        ] {
            let foreign = MessageKey::new(PlatformKind::Slack, &components).unwrap();
            assert!(adapter.route_reply(room.clone(), Some(&foreign)).is_err());
        }
    }

    #[test]
    fn threaded_bot_reaction_enrichment_retains_recordable_author_text_and_conversation() {
        let adapter = adapter();
        let event = json!({
            "type":"reaction_added", "user":"U789", "reaction":"thumbsup",
            "item":{"type":"message", "channel":"C123", "ts":"1700000002.000001"}
        });
        let Some(ChatEvent::ReactionAdded(mut reaction)) = events::decode(
            &adapter.connection,
            &adapter.team,
            &adapter.account.id,
            &event,
        )
        .unwrap() else {
            panic!("reaction was not decoded");
        };
        assert!(reaction.target.author.is_none());
        assert!(reaction.original_text.is_none());
        let data = json!({
            "ok":true, "type":"message", "channel":"C123",
            "message":{"user":"U123", "text":"bot reply &amp; original context",
                "ts":"1700000002.000001", "thread_ts":"1700000000.000123"}
        });
        adapter
            .apply_reaction_context(&mut reaction, &data)
            .unwrap();
        // These are the fields required by the shared reaction recorder.
        assert_eq!(reaction.target.author.as_ref(), Some(&adapter.account.id));
        assert_eq!(
            reaction.original_text.as_deref(),
            Some("bot reply & original context")
        );
        assert_eq!(reaction.target.room, reaction.room.id);
        assert_eq!(
            reaction.room.conversation().thread,
            Some(
                ThreadKey::new(PlatformKind::Slack, &["T123", "C123", "1700000000.000123"])
                    .unwrap()
            )
        );
        assert_eq!(
            reaction.target.id,
            MessageKey::new(PlatformKind::Slack, &["T123", "C123", "1700000002.000001"]).unwrap()
        );
        let enriched = reaction.clone();
        for wrong in [
            json!({"type":"message","channel":"C999","message":data["message"]}),
            json!({"type":"message","channel":"C123","message":{"ts":"1700000009.000001","user":"U123","text":"wrong"}}),
            json!({"type":"message","channel":"C123","message":{"ts":"1700000002.000001","user":"U123","text":"wrong","thread_ts":"invalid"}}),
        ] {
            assert!(adapter
                .apply_reaction_context(&mut reaction, &wrong)
                .is_err());
            assert_eq!(reaction.target.author, enriched.target.author);
            assert_eq!(reaction.original_text, enriched.original_text);
            assert_eq!(reaction.room.conversation(), enriched.room.conversation());
            assert_eq!(reaction.room.space, enriched.room.space);
            assert_eq!(reaction.room.kind, enriched.room.kind);
            assert_eq!(reaction.room.name, enriched.room.name);
        }
    }

    #[tokio::test]
    async fn enriched_threaded_bot_reaction_is_recorded_in_the_native_conversation() {
        crate::memory::test::ensure_test_db();
        let bot = crate::test::route_fixture();
        let mut adapter = adapter();
        adapter.connection = ConnectionId::new("slack-threaded-reaction-recording");
        let connection = adapter.connection.clone();
        let account = adapter.account.clone();
        let event = json!({
            "type":"reaction_added", "user":"U789", "reaction":"thumbsup",
            "item":{"type":"message","channel":"C87651","ts":"1700000002.000001"}
        });
        let Some(ChatEvent::ReactionAdded(mut reaction)) =
            events::decode(&connection, &adapter.team, &account.id, &event).unwrap()
        else {
            panic!("reaction was not decoded");
        };
        adapter
            .apply_reaction_context(
                &mut reaction,
                &json!({
                    "ok":true,"type":"message","channel":"C87651",
                    "message":{"user":"U123","text":"threaded bot context","ts":"1700000002.000001",
                        "thread_ts":"1700000000.000123"}
                }),
            )
            .unwrap();
        reaction.actor.display_name = "Slack Reactor".into();
        let conversation = reaction.room.conversation();
        let base = RoomRef::from_key(reaction.room.id.clone()).conversation();
        {
            let mut registry = bot
                .adapters
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            registry.register(crate::platform::Adapter::Slack(Box::new(adapter)));
            registry
                .set_room_policy(
                    &connection,
                    crate::platform::registry::RoomPolicy::Approved(HashSet::from([reaction
                        .room
                        .id
                        .clone()])),
                )
                .unwrap();
            registry
                .bind_discovered(&connection, reaction.room.clone())
                .unwrap();
        }
        {
            let mut state = bot.state();
            state.accounts.insert(connection.clone(), account);
            crate::start_chat(&mut state, &conversation, None);
            crate::start_chat(&mut state, &base, None);
        }
        bot.reaction_added(&connection, reaction).await;
        let recorded =
            "Slack Reactor: (tepki :thumbsup:) \"threaded bot context\" mesajına tepki verdi";
        {
            let state = bot.state();
            assert_eq!(
                state.channel_history[&conversation].back().unwrap(),
                recorded
            );
            assert_eq!(
                state.chats[&conversation].history.last().unwrap().content,
                recorded
            );
            assert!(!state
                .channel_history
                .get(&base)
                .is_some_and(|history| history.iter().any(|entry| entry == recorded)));
        }
        assert!(
            crate::memory::read(&crate::platform::keys::room_record(&conversation))
                .contains(recorded)
        );
        assert!(
            !crate::memory::read(&crate::platform::keys::room_record(&base)).contains(recorded)
        );
    }

    #[tokio::test]
    async fn thread_history_is_explicitly_unsupported_without_network() {
        let adapter = adapter();
        let mut room = events::room("T123", "C123").unwrap();
        room.thread = Some(
            ThreadKey::new(PlatformKind::Slack, &["T123", "C123", "1700000000.000123"]).unwrap(),
        );
        assert!(matches!(
            adapter
                .fetch_history(&room, HistoryRequest::default())
                .await,
            Err(PlatformError::Unsupported(_))
        ));
    }

    #[test]
    fn media_urls_never_allow_redirect_targets_or_foreign_hosts() {
        assert!(safe_file_url(
            "https://files.slack.com/files-pri/T123-F123/name.png",
            false
        )
        .is_ok());
        assert!(safe_file_url("https://files.slack.com/upload/v1/example", true).is_ok());
        for value in [
            "http://files.slack.com/files-pri/a",
            "https://files.slack.com.evil.test/files-pri/a",
            "https://token@files.slack.com/files-pri/a",
            "https://files.slack.com:444/files-pri/a",
            "https://slack.com/files-pri/a",
        ] {
            assert!(safe_file_url(value, false).is_err());
        }
    }

    #[test]
    fn unknown_scope_or_membership_does_not_grant_capabilities() {
        let adapter = adapter();
        let room = events::room("T123", "C123").unwrap();
        assert!(!adapter.capabilities(&room).send);
        assert!(!adapter.capabilities(&room).send_attachments);
        adapter.rooms.lock().insert(room.id.clone(), room.clone());
        assert!(!adapter.capabilities(&room).send);
        assert!(!adapter.capabilities(&room).send_attachments);
        adapter.scopes.lock().insert("chat:write".into());
        assert!(adapter.capabilities(&room).send);
        assert_eq!(adapter.capabilities(&room).history, HistoryAccess::None);
        assert!(!adapter.capabilities(&room).send_attachments);
        adapter.scopes.lock().insert("files:write".into());
        assert!(adapter.capabilities(&room).send_attachments);
        assert_eq!(
            adapter.capabilities(&room).upload_bytes,
            Some(MEDIA_LIMIT as u64)
        );
        adapter.scopes.lock().remove("chat:write");
        assert!(!adapter.capabilities(&room).send_attachments);
    }

    #[test]
    fn auth_errors_are_sanitized_and_unknown_reactions_are_unsupported() {
        let error = slack_error(
            &json!({"error":"invalid_auth", "detail":"https://secret.invalid/xoxb-token"}),
        )
        .to_string();
        assert_eq!(error, "Slack API: invalid_auth");
        assert!(matches!(
            reaction_name(&Emoji::Unicode("🫠".into())),
            Err(PlatformError::Unsupported(_))
        ));
    }

    #[tokio::test]
    #[ignore = "live Slack authentication check; requires SLACK_APP_TOKEN and SLACK_BOT_TOKEN"]
    async fn live_invalid_authentication_is_rejected_without_secret_leak() {
        std::env::var("SLACK_APP_TOKEN").expect("SLACK_APP_TOKEN required for live test");
        std::env::var("SLACK_BOT_TOKEN").expect("SLACK_BOT_TOKEN required for live test");
        let error = match SlackAdapter::authenticate(
            ConnectionId::new("live-slack-auth"),
            "deliberately-invalid".into(),
        )
        .await
        {
            Ok(_) => panic!("invalid Slack authentication was accepted"),
            Err(error) => error,
        };
        assert!(matches!(error, PlatformError::Transport(_)));
        assert!(!error.to_string().contains("https://"));
        assert!(!error.to_string().contains("deliberately-invalid"));
    }

    #[tokio::test]
    #[ignore = "live Slack discovery check; requires SLACK_APP_TOKEN and SLACK_BOT_TOKEN"]
    async fn live_authentication_and_joined_room_discovery() {
        std::env::var("SLACK_APP_TOKEN").expect("SLACK_APP_TOKEN required for live test");
        let token =
            std::env::var("SLACK_BOT_TOKEN").expect("SLACK_BOT_TOKEN required for live test");
        let adapter = SlackAdapter::authenticate(ConnectionId::new("live-slack"), token)
            .await
            .unwrap();
        for room in adapter.list_rooms().await.unwrap() {
            assert_eq!(room.id.platform(), PlatformKind::Slack);
            assert!(matches!(room.kind, RoomKind::Group | RoomKind::Channel));
        }
    }
}
