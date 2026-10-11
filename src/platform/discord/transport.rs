//! Private Serenity transport. Receipts never retain a native message or cache guard.
use super::{events::decode_user, native_message, native_room, native_space, native_user, render};
use crate::platform::{
    capabilities::*, outgoing::*, types::*, Platform, PlatformError, PlatformResult,
};
use serenity::all::{
    ChannelId, ChannelType, Context, CreateAttachment, EmojiId, GetMessages, GuildChannel, GuildId,
    Message, ModelError, Permissions, ReactionType,
};
use std::time::Duration;

pub struct DiscordAdapter {
    context: Context,
    connection: ConnectionId,
    account: UserRef,
}

impl DiscordAdapter {
    /// Called only from Ready, after Discord accepted the configured privileged intents.
    pub fn new(context: Context, connection: ConnectionId, account: UserRef) -> Self {
        Self {
            context,
            connection,
            account,
        }
    }

    fn ready(&self) -> bool {
        native_user(&self.account.id).is_ok_and(|id| self.context.cache.current_user().id == id)
    }

    fn guild_for_room(&self, room: &RoomRef) -> Option<GuildId> {
        let channel = native_room(&room.id).ok()?;
        if let Some(space) = &room.space {
            let id = native_space(space).ok()?;
            let guild = self.context.cache.guild(id)?;
            return (guild.channels.contains_key(&channel)
                || guild.threads.iter().any(|t| t.id == channel))
            .then_some(id);
        }
        self.context.cache.guilds().into_iter().find(|id| {
            self.context.cache.guild(*id).is_some_and(|guild| {
                guild.channels.contains_key(&channel)
                    || guild.threads.iter().any(|t| t.id == channel)
            })
        })
    }

    fn check_receipt(
        &self,
        receipt: &MessageReceipt,
    ) -> PlatformResult<(ChannelId, serenity::all::MessageId)> {
        if receipt.room.thread.is_some() {
            return Err(PlatformError::Unsupported("separate Discord thread scope"));
        }
        if receipt.connection != self.connection || receipt.message.room != receipt.room.id {
            return Err(PlatformError::InvalidIdentity(KeyError));
        }
        if receipt.message.author.as_ref() != Some(&self.account.id) {
            return Err(PlatformError::PermissionDenied);
        }
        Ok((
            native_room(&receipt.room.id)?,
            native_message(&receipt.message.id)?,
        ))
    }

    fn permission_snapshot(&self, room: &RoomRef) -> Option<Capabilities> {
        if !self.ready() || room.thread.is_some() {
            return None;
        }
        let channel_id = native_room(&room.id).ok()?;
        let guild_id = self.guild_for_room(room)?;
        let guild = self.context.cache.guild(guild_id)?;
        let channel = guild
            .channels
            .get(&channel_id)
            .or_else(|| guild.threads.iter().find(|t| t.id == channel_id))?;
        let member = guild.members.get(&native_user(&self.account.id).ok()?)?;
        // Unknown role records cannot be treated as a known permission grant.
        if !guild
            .roles
            .contains_key(&serenity::all::RoleId::new(guild_id.get()))
            || member.roles.iter().any(|id| !guild.roles.contains_key(id))
        {
            return None;
        }
        let thread = matches!(
            channel.kind,
            ChannelType::NewsThread | ChannelType::PublicThread | ChannelType::PrivateThread
        );
        let permission_channel = if thread {
            guild.channels.get(&channel.parent_id?)?
        } else {
            channel
        };
        let perms = guild.user_permissions_in(permission_channel, member);
        let visible = perms.contains(Permissions::VIEW_CHANNEL)
            && (channel.kind != ChannelType::PrivateThread
                || channel.member.is_some()
                || perms.contains(Permissions::MANAGE_THREADS));
        let text_based = matches!(
            channel.kind,
            ChannelType::Text
                | ChannelType::News
                | ChannelType::Voice
                | ChannelType::Stage
                | ChannelType::NewsThread
                | ChannelType::PublicThread
                | ChannelType::PrivateThread
        );
        let unarchived = !thread
            || channel
                .thread_metadata
                .is_some_and(|meta| !meta.archived && !meta.locked);
        let send = visible
            && text_based
            && unarchived
            && perms.contains(if thread {
                Permissions::SEND_MESSAGES_IN_THREADS
            } else {
                Permissions::SEND_MESSAGES
            })
            && member
                .communication_disabled_until
                .is_none_or(|until| until <= serenity::all::Timestamp::now());
        let history = visible && text_based && perms.contains(Permissions::READ_MESSAGE_HISTORY);
        let application = self.account.automation == Automation::Bot
            && self.context.http.application_id().is_some();
        Some(Capabilities {
            // Ready proves acceptance of main's MESSAGE_CONTENT and reaction intents.
            visibility: if visible {
                MessageVisibility::Ordinary
            } else {
                MessageVisibility::MentionsOnly
            },
            history: if history {
                HistoryAccess::Paginated
            } else {
                HistoryAccess::None
            },
            decryption_available: visible && text_based,
            replies: send && history,
            threads: visible && text_based && perms.contains(Permissions::SEND_MESSAGES_IN_THREADS),
            send,
            react: history && perms.contains(Permissions::ADD_REACTIONS) && send,
            observe_reactions: visible && text_based,
            // These refer to the adapter's own messages, not moderator powers.
            edit: send,
            delete: visible && text_based,
            typing: send,
            nickname: if visible && perms.contains(Permissions::CHANGE_NICKNAME) {
                NicknameScope::Space
            } else {
                NicknameScope::None
            },
            rich_cards: send && perms.contains(Permissions::EMBED_LINKS),
            components: send && application,
            modals: visible && application,
            native_commands: visible
                && application
                && perms.contains(Permissions::USE_APPLICATION_COMMANDS),
            private_results: visible && application,
            output: OutputLimits::DISCORD,
            edit_cadence: Duration::from_millis(1200),
            // No credential-specific upload ceiling has been observed in this snapshot.
            upload_bytes: None,
        })
    }
}

fn classify(error: serenity::Error) -> PlatformError {
    match &error {
        serenity::Error::Http(http) => match http.status_code().map(|code| code.as_u16()) {
            Some(401) => return PlatformError::Disconnected,
            Some(403) => return PlatformError::PermissionDenied,
            Some(404) => return PlatformError::NotFound,
            Some(429) => return PlatformError::RateLimited { retry_after: None },
            _ => {}
        },
        serenity::Error::Model(ModelError::InvalidPermissions { .. } | ModelError::InvalidUser) => {
            return PlatformError::PermissionDenied
        }
        serenity::Error::Gateway(_) => return PlatformError::Disconnected,
        _ => {}
    }
    PlatformError::Transport(error.to_string())
}

async fn attachment(source: &OutgoingAttachment<'_>) -> Option<CreateAttachment> {
    match source {
        OutgoingAttachment::LocalPath(path) => match CreateAttachment::path(path).await {
            Ok(file) => Some(file),
            // Legacy Discord behavior sends the text even when an optional image cannot be read.
            Err(error) => {
                log::warn!("couldn't read image ({}): {error}", path.display());
                None
            }
        },
        OutgoingAttachment::Bytes { filename, data, .. } => {
            Some(CreateAttachment::bytes(data.to_vec(), *filename))
        }
    }
}

fn room_ref(channel: &GuildChannel) -> RoomRef {
    RoomRef {
        id: RoomKey::discord(channel.id.get()),
        space: Some(SpaceKey::discord(channel.guild_id.get())),
        kind: RoomKind::Channel,
        name: Some(channel.name.clone()),
        thread: None,
    }
}

/// History intentionally differs from live ingress: raw emptiness gates safe text,
/// bots are excluded but webhook metadata and attachments do not change the old scan.
fn decode_history(
    message: Message,
    room: &RoomRef,
    cache: &std::sync::Arc<serenity::cache::Cache>,
) -> Option<IncomingMessage> {
    if message.author.bot || message.content.trim().is_empty() {
        return None;
    }
    Some(IncomingMessage {
        id: MessageKey::discord(message.id.get()),
        room: room.clone(),
        author: decode_user(&message.author),
        sent_at_unix_ms: message.timestamp.timestamp_millis(),
        text: message.content_safe(cache),
        mentions: message
            .mentions
            .iter()
            .map(|user| UserKey::discord(user.id.get()))
            .collect(),
        reply_to: message
            .referenced_message
            .as_ref()
            .map(|target| MessageRef {
                id: MessageKey::discord(target.id.get()),
                room: RoomKey::discord(target.channel_id.get()),
                author: Some(UserKey::discord(target.author.id.get())),
            }),
        attachments: Vec::new(),
    })
}

impl Platform for DiscordAdapter {
    fn connection_id(&self) -> &ConnectionId {
        &self.connection
    }
    fn self_user(&self) -> &UserRef {
        &self.account
    }
    fn capabilities(&self, room: &RoomRef) -> Capabilities {
        self.permission_snapshot(room)
            .unwrap_or_else(Capabilities::unavailable)
    }

    fn render_stream(
        &self,
        _room: &RoomRef,
        mode: ThoughtPresentation,
        thought: &str,
        text: &str,
        done: bool,
    ) -> Vec<String> {
        render::stream_view(thinking_mode(mode), thought, text, done)
    }
    fn render_final(
        &self,
        _room: &RoomRef,
        mode: ThoughtPresentation,
        thought: &str,
        lines: &[String],
    ) -> Vec<String> {
        render::stream_layout(thinking_mode(mode), thought, lines)
    }

    async fn send<'a>(
        &'a self,
        room: &'a RoomRef,
        body: &'a OutgoingMessage<'a>,
    ) -> PlatformResult<MessageReceipt> {
        if room.thread.is_some() {
            return Err(PlatformError::Unsupported("separate Discord thread scope"));
        }
        let channel = native_room(&room.id)?;
        let mut builder = render::outgoing_message(channel, body)?;
        for source in body.attachments {
            if let Some(file) = attachment(source).await {
                builder = builder.add_file(file);
            }
        }
        let message = channel
            .send_message(&self.context.http, builder)
            .await
            .map_err(classify)?;
        Ok(MessageReceipt {
            connection: self.connection.clone(),
            message: MessageRef {
                id: MessageKey::discord(message.id.get()),
                room: room.id.clone(),
                author: Some(self.account.id.clone()),
            },
            room: room.clone(),
            last_accepted_text: message.content,
            version: 0,
        })
    }

    async fn edit<'a>(
        &'a self,
        receipt: &'a MessageReceipt,
        body: &'a OutgoingMessage<'a>,
    ) -> PlatformResult<()> {
        let (channel, message) = self.check_receipt(receipt)?;
        let mut builder = render::outgoing_edit(body)?;
        for source in body.attachments {
            if let Some(file) = attachment(source).await {
                builder = builder.new_attachment(file);
            }
        }
        channel
            .edit_message(&self.context.http, message, builder)
            .await
            .map_err(classify)?;
        // No receipt mutation: the caller advances accepted text/version only on Ok.
        Ok(())
    }

    async fn delete<'a>(&'a self, receipt: &'a MessageReceipt) -> PlatformResult<()> {
        let (channel, message) = self.check_receipt(receipt)?;
        channel
            .delete_message(&self.context.http, message)
            .await
            .map_err(classify)
    }

    async fn react<'a>(&'a self, target: &'a MessageRef, emoji: &'a Emoji) -> PlatformResult<()> {
        let native = match emoji {
            Emoji::Unicode(text) => ReactionType::Unicode(text.clone()),
            Emoji::Custom {
                platform: PlatformKind::Discord,
                id,
                label,
                animated,
            } => {
                let id = id
                    .parse::<u64>()
                    .ok()
                    .filter(|id| *id != 0)
                    .ok_or(PlatformError::InvalidIdentity(KeyError))?;
                ReactionType::Custom {
                    animated: *animated,
                    id: EmojiId::new(id),
                    name: label.clone(),
                }
            }
            Emoji::Custom { .. } => return Err(PlatformError::Unsupported("foreign custom emoji")),
        };
        self.context
            .http
            .create_reaction(
                native_room(&target.room)?,
                native_message(&target.id)?,
                &native,
            )
            .await
            .map_err(classify)
    }

    async fn typing<'a>(&'a self, room: &'a RoomRef) -> PlatformResult<()> {
        if room.thread.is_some() {
            return Err(PlatformError::Unsupported("separate Discord thread scope"));
        }
        native_room(&room.id)?
            .broadcast_typing(&self.context.http)
            .await
            .map_err(classify)
    }

    async fn set_nickname<'a>(
        &'a self,
        target: NickTarget<'a>,
        name: &'a str,
    ) -> PlatformResult<()> {
        match target {
            NickTarget::Space(space) => native_space(space)?
                .edit_nickname(&self.context.http, Some(name))
                .await
                .map_err(classify),
            NickTarget::Room(_) => Err(PlatformError::Unsupported("room-scoped Discord nickname")),
            NickTarget::Account => Err(PlatformError::Unsupported(
                "account-scoped Discord nickname",
            )),
        }
    }

    async fn fetch_history<'a>(
        &'a self,
        room: &'a RoomRef,
        request: HistoryRequest,
    ) -> PlatformResult<HistoryPage> {
        if room.thread.is_some() {
            return Err(PlatformError::Unsupported("separate Discord thread scope"));
        }
        let channel = native_room(&room.id)?;
        // Discord can return an indistinguishable empty page without READ_MESSAGE_HISTORY.
        // Resolve permissions first; unknown permission state is not successful empty history.
        if self.capabilities(room).history == HistoryAccess::None {
            return Err(PlatformError::PermissionDenied);
        }
        let limit = request.limit.min(100);
        if limit == 0 {
            return Err(PlatformError::Unsupported("zero-sized history page"));
        }
        let mut query = GetMessages::new().limit(limit as u8);
        if let Some(cursor) = request.cursor {
            let key: MessageKey = cursor.parse()?;
            query = query.before(native_message(&key)?);
        }
        let batch = channel
            .messages(&self.context.http, query)
            .await
            .map_err(classify)?;
        let next = if batch.len() == limit {
            batch
                .last()
                .map(|m| MessageKey::discord(m.id.get()).to_string())
        } else {
            None
        };
        let mut hit_cutoff = false;
        let mut messages = Vec::with_capacity(batch.len());
        for message in batch {
            if request
                .cutoff_unix_ms
                .is_some_and(|cutoff| message.timestamp.timestamp_millis() < cutoff)
            {
                hit_cutoff = true;
                break;
            }
            if let Some(decoded) = decode_history(message, room, &self.context.cache) {
                messages.push(decoded);
            }
        }
        Ok(HistoryPage {
            messages,
            next_cursor: if hit_cutoff { None } else { next },
            limitations: Vec::new(),
            order: HistoryOrder::NewestFirst,
        })
    }

    async fn list_rooms(&self) -> PlatformResult<Vec<RoomRef>> {
        if !self.ready() {
            return Err(PlatformError::Disconnected);
        }
        let mut rooms = Vec::new();
        for guild_id in self.context.cache.guilds() {
            // Fetch the real channel list; only cached guild/member facts decide usability.
            let channels = guild_id
                .channels(&self.context.http)
                .await
                .map_err(classify)?;
            let mut channels: Vec<_> = channels.into_values().collect();
            channels.sort_by_key(|channel| (channel.position, channel.id));
            for channel in channels {
                let room = room_ref(&channel);
                if matches!(
                    channel.kind,
                    ChannelType::Text | ChannelType::News | ChannelType::Voice | ChannelType::Stage
                ) {
                    let capabilities = self
                        .permission_snapshot(&room)
                        .ok_or(PlatformError::PermissionDenied)?;
                    if capabilities.decryption_available {
                        rooms.push(room);
                    }
                }
            }
            let threads = self
                .context
                .cache
                .guild(guild_id)
                .map(|guild| guild.threads.iter().map(room_ref).collect::<Vec<_>>())
                .unwrap_or_default();
            for room in threads {
                if self.capabilities(&room).decryption_available {
                    rooms.push(room);
                }
            }
        }
        Ok(rooms)
    }

    async fn resolve_media<'a>(
        &'a self,
        attachment: &'a AttachmentRef,
    ) -> PlatformResult<ModelMedia> {
        match (&attachment.kind, &attachment.source) {
            (MediaKind::Image, MediaSource::PublicUrl(url)) => {
                Ok(ModelMedia::PublicUrl(url.clone()))
            }
            (MediaKind::Image, MediaSource::Native { .. }) => {
                Err(PlatformError::Unsupported("native Discord media reference"))
            }
            _ => Err(PlatformError::Unsupported("non-image model media")),
        }
    }
}

fn thinking_mode(mode: ThoughtPresentation) -> crate::ThinkingMode {
    match mode {
        ThoughtPresentation::Show => crate::ThinkingMode::Show,
        ThoughtPresentation::Hide => crate::ThinkingMode::Hide,
        ThoughtPresentation::Silent => crate::ThinkingMode::Silent,
        ThoughtPresentation::Off => crate::ThinkingMode::Off,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serenity::all::{MessageId, Timestamp, UserId, WebhookId};

    #[test]
    fn history_keeps_raw_nonempty_webhook_text_but_excludes_bots_and_empty_content() {
        let cache = std::sync::Arc::new(serenity::cache::Cache::new());
        let room = RoomRef::from_key(RoomKey::discord(7));
        let fixture = || {
            let mut message = Message::default();
            message.id = MessageId::new(81);
            message.channel_id = ChannelId::new(7);
            message.guild_id = Some(GuildId::new(9));
            message.author.id = UserId::new(42);
            message.author.name = "account".into();
            message.author.global_name = Some("Display".into());
            message.timestamp = Timestamp::from_millis(1_700_000_000_123).unwrap();
            message.content = "hello <@42>".into();
            message.webhook_id = Some(WebhookId::new(99));
            message
        };
        let message = fixture();
        let safe = message.content_safe(&cache);
        let decoded = decode_history(message, &room, &cache).unwrap();
        assert_eq!(decoded.text, safe);
        assert_eq!(decoded.author.display_name, "Display");
        assert_eq!(decoded.id, MessageKey::discord(81));
        assert_eq!(decoded.sent_at_unix_ms, 1_700_000_000_123);
        assert!(decoded.attachments.is_empty());
        let mut bot = fixture();
        bot.author.bot = true;
        assert!(decode_history(bot, &room, &cache).is_none());
        let mut empty = fixture();
        empty.content = " \n\t ".into();
        assert!(decode_history(empty, &room, &cache).is_none());
    }

    #[test]
    fn sdk_permission_failures_remain_distinguishable_from_transport_failures() {
        assert!(matches!(
            classify(serenity::Error::Model(ModelError::InvalidUser)),
            PlatformError::PermissionDenied
        ));
        assert!(matches!(
            classify(serenity::Error::Io(std::io::Error::other("offline"))),
            PlatformError::Transport(_)
        ));
    }
}
