use super::{events, render, MatrixAdapter};
use crate::platform::{
    capabilities::*, outgoing::*, types::*, Platform, PlatformError, PlatformResult,
};
use base64::Engine;
use matrix_sdk::{
    room::MessagesOptions,
    ruma::{
        events::{room::power_levels::PowerLevelAction, MessageLikeEventType},
        EventId, RoomId, UInt,
    },
    Room, RoomState,
};
use serde_json::{json, Value};
use std::time::Duration;

const MEDIA_LIMIT: u64 = 10 * 1024 * 1024;
fn transport(error: impl std::fmt::Display) -> PlatformError {
    PlatformError::Transport(error.to_string())
}
fn native(key: &RoomKey) -> PlatformResult<String> {
    if key.platform() != PlatformKind::Matrix {
        return Err(PlatformError::InvalidIdentity(KeyError));
    }
    key.components()
        .next()
        .ok_or(PlatformError::InvalidIdentity(KeyError))?
        .map_err(Into::into)
}
fn event_id(key: &MessageKey) -> PlatformResult<matrix_sdk::ruma::OwnedEventId> {
    if key.platform() != PlatformKind::Matrix {
        return Err(PlatformError::InvalidIdentity(KeyError));
    }
    let id = key
        .components()
        .next()
        .ok_or(PlatformError::InvalidIdentity(KeyError))??;
    EventId::parse(id).map_err(transport)
}
impl MatrixAdapter {
    pub(crate) fn room(&self, key: &RoomKey) -> PlatformResult<Room> {
        let id = RoomId::parse(native(key)?).map_err(transport)?;
        let room = self.client.get_room(&id).ok_or(PlatformError::NotFound)?;
        if room.state() != RoomState::Joined {
            return Err(PlatformError::PermissionDenied);
        }
        if room.encryption_state().is_encrypted() {
            return Err(PlatformError::Unsupported(
                "encrypted rooms not supported yet",
            ));
        }
        if !matches!(
            room.encryption_state(),
            matrix_sdk::EncryptionState::NotEncrypted
        ) {
            return Err(PlatformError::Disconnected);
        }
        Ok(room)
    }
    async fn plaintext_room(&self, key: &RoomKey) -> PlatformResult<Room> {
        let id = RoomId::parse(native(key)?).map_err(transport)?;
        let room = self.client.get_room(&id).ok_or(PlatformError::NotFound)?;
        room.request_encryption_state().await.map_err(transport)?;
        // SDK crypto is disabled, so check the authoritative encryption state
        // before text delivery rather than rely on a potentially stale sync.
        let request =
            matrix_sdk::ruma::api::client::state::get_state_event_for_key::v3::Request::new(
                id,
                matrix_sdk::ruma::events::StateEventType::RoomEncryption,
                String::new(),
            );
        match self.client.send(request).await {
            Ok(_) => Err(PlatformError::Unsupported(
                "encrypted rooms not supported yet",
            )),
            Err(error)
                if error.client_api_error_kind()
                    == Some(&matrix_sdk::ruma::api::error::ErrorKind::NotFound) =>
            {
                self.room(key)
            }
            Err(error) => Err(transport(error)),
        }
    }
    pub(crate) async fn refresh_capabilities(
        &self,
        room: &RoomRef,
    ) -> PlatformResult<Capabilities> {
        let mut caps = Capabilities::unavailable();
        caps.output = render::OUTPUT_LIMITS;
        if let Some(native) = self
            .client
            .get_room(&RoomId::parse(native(&room.id)?).map_err(transport)?)
        {
            native.request_encryption_state().await.map_err(transport)?;
        }
        if let Ok(native) = self.room(&room.id) {
            let user = self.client.user_id().ok_or(PlatformError::Disconnected)?;
            let power = native.power_levels().await.map_err(transport)?;
            let level = power.for_user(user);
            caps.visibility = MessageVisibility::Ordinary;
            caps.history = HistoryAccess::Bounded;
            caps.replies = true;
            caps.send = level
                >= power.for_action(PowerLevelAction::SendMessage(
                    MessageLikeEventType::RoomMessage,
                ));
            caps.send_attachments = caps.send;
            caps.react = level
                >= power.for_action(PowerLevelAction::SendMessage(
                    MessageLikeEventType::Reaction,
                ));
            caps.observe_reactions = true;
            caps.edit = caps.send;
            caps.delete = level >= power.for_action(PowerLevelAction::RedactOwn);
            caps.typing = true;
            // Own joined membership profile updates do not require state_default.
            caps.nickname = NicknameScope::Room;
            caps.edit_cadence = Duration::from_millis(1200);
            caps.upload_bytes = Some(MEDIA_LIMIT);
        }
        self.capabilities
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(room.id.clone(), caps.clone());
        Ok(caps)
    }
    async fn send_content(
        &self,
        room: &RoomRef,
        body: &OutgoingMessage<'_>,
    ) -> PlatformResult<Value> {
        if body.reply_to.is_some_and(|reply| reply.room != room.id) {
            return Err(PlatformError::InvalidIdentity(KeyError));
        }
        let mut content = render::content(body)?;
        if body.attachments.len() > 1 {
            return Err(PlatformError::Unsupported(
                "multiple Matrix attachments in one event",
            ));
        }
        if let Some(attachment) = body.attachments.first() {
            let (filename, bytes, mime) = match attachment {
                OutgoingAttachment::LocalPath(path) => {
                    let metadata = tokio::fs::metadata(path).await.map_err(transport)?;
                    if metadata.len() > MEDIA_LIMIT {
                        return Err(PlatformError::Unsupported("media exceeds 10 MiB"));
                    }
                    let filename = path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .ok_or(PlatformError::Unsupported("invalid media filename"))?
                        .to_owned();
                    let mime = match path
                        .extension()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .as_str()
                    {
                        "png" => "image/png",
                        "jpg" | "jpeg" => "image/jpeg",
                        "gif" => "image/gif",
                        "webp" => "image/webp",
                        _ => "application/octet-stream",
                    }
                    .to_owned();
                    (
                        filename,
                        tokio::fs::read(path).await.map_err(transport)?,
                        mime,
                    )
                }
                OutgoingAttachment::Bytes {
                    filename,
                    data,
                    mime,
                } => {
                    if data.len() as u64 > MEDIA_LIMIT {
                        return Err(PlatformError::Unsupported("media exceeds 10 MiB"));
                    }
                    (
                        (*filename).to_owned(),
                        data.to_vec(),
                        mime.unwrap_or("application/octet-stream").to_owned(),
                    )
                }
            };
            if bytes.len() as u64 > MEDIA_LIMIT {
                return Err(PlatformError::Unsupported("media exceeds 10 MiB"));
            }
            let length = bytes.len();
            let mut request =
                matrix_sdk::ruma::api::client::media::create_content::v3::Request::new(bytes);
            request.filename = Some(filename.clone());
            request.content_type = Some(mime.clone());
            let response = self.client.send(request).await.map_err(transport)?;
            content["msgtype"] = json!(if mime.starts_with("image/") {
                "m.image"
            } else {
                "m.file"
            });
            content["filename"] = json!(filename);
            content["url"] = json!(response.content_uri);
            content["info"] = json!({"mimetype":mime,"size":length});
            if body.text.is_empty() {
                content["body"] = content["filename"].clone();
            }
        }
        Ok(content)
    }
}
impl Platform for MatrixAdapter {
    fn connection_id(&self) -> &ConnectionId {
        &self.connection
    }
    fn self_user(&self) -> &UserRef {
        &self.account
    }
    fn capabilities(&self, room: &RoomRef) -> Capabilities {
        self.capabilities
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&room.id)
            .cloned()
            .unwrap_or_else(|| {
                let mut caps = Capabilities::unavailable();
                caps.output = render::OUTPUT_LIMITS;
                caps
            })
    }
    fn render_stream(
        &self,
        _: &RoomRef,
        mode: ThoughtPresentation,
        thought: &str,
        text: &str,
        done: bool,
    ) -> Vec<String> {
        render::render_stream(mode, thought, text, done)
    }
    fn render_final(
        &self,
        _: &RoomRef,
        mode: ThoughtPresentation,
        thought: &str,
        lines: &[String],
    ) -> Vec<String> {
        render::render_final(mode, thought, lines)
    }
    async fn send<'a>(
        &'a self,
        room: &'a RoomRef,
        body: &'a OutgoingMessage<'a>,
    ) -> PlatformResult<MessageReceipt> {
        let native = self.plaintext_room(&room.id).await?;
        let response = native
            .send_raw("m.room.message", self.send_content(room, body).await?)
            .await
            .map_err(transport)?;
        Ok(MessageReceipt {
            connection: self.connection.clone(),
            message: MessageRef {
                id: events::message_key(response.response.event_id.as_str())?,
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
        sent: &'a MessageReceipt,
        body: &'a OutgoingMessage<'a>,
    ) -> PlatformResult<()> {
        if sent.connection != self.connection
            || sent.message.author.as_ref() != Some(&self.account.id)
        {
            return Err(PlatformError::PermissionDenied);
        }
        let native = self.plaintext_room(&sent.room.id).await?;
        let replacement = self.send_content(&sent.room, body).await?;
        let mut content = replacement.clone();
        content["m.new_content"] = replacement;
        content["m.relates_to"] =
            json!({"rel_type":"m.replace","event_id":event_id(&sent.message.id)?});
        content["body"] = json!(format!("* {}", body.text));
        native
            .send_raw("m.room.message", content)
            .await
            .map_err(transport)?;
        Ok(())
    }
    async fn delete<'a>(&'a self, sent: &'a MessageReceipt) -> PlatformResult<()> {
        if sent.connection != self.connection
            || sent.message.author.as_ref() != Some(&self.account.id)
        {
            return Err(PlatformError::PermissionDenied);
        }
        self.room(&sent.room.id)?
            .redact(&event_id(&sent.message.id)?, None, None)
            .await
            .map_err(transport)?;
        Ok(())
    }
    async fn react<'a>(&'a self, target: &'a MessageRef, emoji: &'a Emoji) -> PlatformResult<()> {
        let Emoji::Unicode(key) = emoji else {
            return Err(PlatformError::Unsupported("custom Matrix reactions"));
        };
        self.room(&target.room)?.send_raw("m.reaction", json!({"m.relates_to":{"rel_type":"m.annotation","event_id":event_id(&target.id)?,"key":key}})).await.map_err(transport)?;
        Ok(())
    }
    async fn typing<'a>(&'a self, room: &'a RoomRef) -> PlatformResult<()> {
        self.room(&room.id)?
            .typing_notice(true)
            .await
            .map_err(transport)
    }
    async fn set_nickname<'a>(
        &'a self,
        target: NickTarget<'a>,
        name: &'a str,
    ) -> PlatformResult<()> {
        let NickTarget::Room(room) = target else {
            return Err(PlatformError::Unsupported("Matrix nicknames are per-room"));
        };
        let native = self.room(&room.id)?;
        let user = self.client.user_id().ok_or(PlatformError::Disconnected)?;
        let member = native
            .get_member_no_sync(user)
            .await
            .map_err(transport)?
            .ok_or(PlatformError::NotFound)?;
        native
            .send_state_event_raw(
                "m.room.member",
                user.as_str(),
                json!({"membership":"join","displayname":name,"avatar_url":member.avatar_url()}),
            )
            .await
            .map_err(transport)?;
        Ok(())
    }
    async fn fetch_history<'a>(
        &'a self,
        room: &'a RoomRef,
        page: HistoryRequest,
    ) -> PlatformResult<HistoryPage> {
        let native = self.plaintext_room(&room.id).await?;
        let mut options = MessagesOptions::backward();
        options.from = page.cursor;
        options.limit = UInt::try_from(page.limit.clamp(1, 100)).map_err(transport)?;
        let response = native.messages(options).await.map_err(transport)?;
        let mut messages = Vec::new();
        let mut cutoff = false;
        for event in response.chunk {
            let raw: Value = serde_json::from_str(event.raw().json().get()).map_err(transport)?;
            if page.cutoff_unix_ms.is_some_and(|cutoff| {
                raw["origin_server_ts"]
                    .as_i64()
                    .is_some_and(|time| time < cutoff)
            }) {
                cutoff = true;
                continue;
            }
            if let Some(ChatEvent::MessageCreated(message)) =
                events::decode_event(&self.connection, room, &self.account.id, &raw, false)?
            {
                if super::runner::command_text(&message.text).is_none() {
                    messages.push(message);
                }
            }
        }
        Ok(HistoryPage {
            messages,
            next_cursor: if cutoff { None } else { response.end },
            limitations: vec![HistoryLimitation::Bounded],
            order: HistoryOrder::NewestFirst,
        })
    }
    async fn list_rooms(&self) -> PlatformResult<Vec<RoomRef>> {
        let approved = self
            .capabilities
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        Ok(approved
            .into_iter()
            .filter(|key| self.room(key).is_ok())
            .map(RoomRef::from_key)
            .collect())
    }
    async fn resolve_media<'a>(
        &'a self,
        attachment: &'a AttachmentRef,
    ) -> PlatformResult<ModelMedia> {
        if !self.image_analysis {
            return Err(PlatformError::Unsupported("image analysis is disabled"));
        }
        if !matches!(attachment.kind, MediaKind::Image)
            || attachment.size.is_some_and(|size| size > MEDIA_LIMIT)
        {
            return Err(PlatformError::Unsupported("media type or size"));
        }
        let MediaSource::Native { connection, id } = &attachment.source else {
            return Err(PlatformError::Unsupported("Matrix media must use mxc"));
        };
        if connection != &self.connection {
            return Err(PlatformError::PermissionDenied);
        }
        let uri: &matrix_sdk::ruma::MxcUri = id.as_str().into();
        let (server, media) = uri.parts().map_err(transport)?;
        let mut url = self.client.homeserver();
        url.path_segments_mut()
            .map_err(|_| PlatformError::Unsupported("homeserver URL"))?
            .pop_if_empty()
            .extend([
                "_matrix",
                "client",
                "v1",
                "media",
                "download",
                server.as_str(),
                media,
            ]);
        let token = self
            .client
            .access_token()
            .ok_or(PlatformError::Disconnected)?;
        let mut response = self
            .media_http
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(transport)?
            .error_for_status()
            .map_err(transport)?;
        if response
            .content_length()
            .is_some_and(|size| size > MEDIA_LIMIT)
        {
            return Err(PlatformError::Unsupported("media exceeds 10 MiB"));
        }
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_owned();
        if !matches!(
            mime.as_str(),
            "image/jpeg" | "image/png" | "image/gif" | "image/webp"
        ) {
            return Err(PlatformError::Unsupported("unsupported image MIME"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport)? {
            if bytes.len() as u64 + chunk.len() as u64 > MEDIA_LIMIT {
                return Err(PlatformError::Unsupported("media exceeds 10 MiB"));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(ModelMedia::DataUrl(format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )))
    }
}
