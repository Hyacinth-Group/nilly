//! Native exclusions, safe-content decoding and reaction enrichment live here.
use crate::platform::types::*;
use serenity::all::{
    Attachment, ChannelId, Context, GuildId, Member, Message, Reaction, ReactionType, User,
};
use std::{collections::HashSet, sync::Arc};

pub(crate) fn connection_id() -> &'static ConnectionId {
    static CONNECTION: std::sync::LazyLock<ConnectionId> =
        std::sync::LazyLock::new(|| ConnectionId::new("discord-default"));
    &CONNECTION
}

pub(crate) fn display_name(user: &User) -> String {
    user.global_name
        .clone()
        .unwrap_or_else(|| user.name.clone())
}

pub(crate) fn decode_user(user: &User) -> UserRef {
    UserRef {
        id: UserKey::discord(user.id.get()),
        display_name: display_name(user),
        username: Some(user.name.clone()),
        automation: if user.bot {
            Automation::Bot
        } else {
            Automation::Human
        },
        stability: IdentityStability::Persistent,
    }
}

fn room(channel: ChannelId, guild: Option<GuildId>) -> RoomRef {
    RoomRef {
        id: RoomKey::discord(channel.get()),
        space: guild.map(|id| SpaceKey::discord(id.get())),
        kind: if guild.is_some() {
            RoomKind::Channel
        } else {
            RoomKind::Direct
        },
        name: None,
        thread: None,
    }
}

fn message_input<'a>(
    msg: &'a Message,
    cache: &Arc<serenity::cache::Cache>,
    room: &RoomRef,
    guild_id: Option<&SpaceKey>,
    allowed_channels: Option<&HashSet<RoomKey>>,
    image_analysis: bool,
) -> Option<(String, Option<&'a Attachment>)> {
    if guild_id.is_some_and(|guild| room.space.as_ref() != Some(guild)) {
        return None;
    }
    if allowed_channels.is_some_and(|channels| !channels.contains(&room.id)) {
        return None;
    }
    let raw_text = msg.content_safe(cache);
    // IMAGE_ANALYSIS off treats all attachments as absent. MIME, not extension or
    // dimensions, selects the first image and its original CDN URL.
    let image = if image_analysis {
        msg.attachments.iter().find(|att| {
            att.content_type
                .as_deref()
                .is_some_and(|mime| mime.starts_with("image/"))
        })
    } else {
        None
    };
    if raw_text.trim().is_empty() && image.is_none() {
        return None;
    }
    let text = match image {
        None => raw_text,
        Some(_) if raw_text.trim().is_empty() => "[resim attı]".to_string(),
        Some(_) => format!("[resim] {raw_text}"),
    };
    Some((text, image))
}

pub(crate) fn decode_message(
    msg: &Message,
    cache: &Arc<serenity::cache::Cache>,
    guild_id: Option<&SpaceKey>,
    allowed_channels: Option<&HashSet<RoomKey>>,
    image_analysis: bool,
) -> Option<IncomingMessage> {
    // A webhook author is excluded; application/system metadata alone is not.
    if msg.author.bot || msg.webhook_id.is_some() || msg.guild_id.is_none() {
        return None;
    }
    let room = room(msg.channel_id, msg.guild_id);
    let (text, image) = message_input(
        msg,
        cache,
        &room,
        guild_id,
        allowed_channels,
        image_analysis,
    )?;
    Some(IncomingMessage {
        id: MessageKey::discord(msg.id.get()),
        room,
        author: decode_user(&msg.author),
        sent_at_unix_ms: msg.timestamp.timestamp_millis(),
        text,
        mentions: msg
            .mentions
            .iter()
            .map(|user| UserKey::discord(user.id.get()))
            .collect(),
        reply_to: msg.referenced_message.as_ref().map(|message| MessageRef {
            id: MessageKey::discord(message.id.get()),
            room: RoomKey::discord(message.channel_id.get()),
            author: Some(UserKey::discord(message.author.id.get())),
        }),
        attachments: image
            .into_iter()
            .map(|attachment| AttachmentRef {
                kind: MediaKind::Image,
                mime: attachment.content_type.clone(),
                size: Some(u64::from(attachment.size)),
                source: MediaSource::PublicUrl(attachment.url.clone()),
            })
            .collect(),
    })
}

/// Retains the original system-channel preference, including the unrestricted
/// default-channel fallback when the joining guild has no system channel.
pub(crate) fn decode_member_join(
    bot: &crate::Bot,
    ctx: &Context,
    member: &Member,
) -> Option<(RoomRef, UserRef)> {
    let space = SpaceKey::discord(member.guild_id.get());
    if bot.guild_id.as_ref().is_some_and(|guild| guild != &space) {
        return None;
    }
    let system = ctx
        .cache
        .guild(member.guild_id)
        .and_then(|guild| guild.system_channel_id);
    let channel = system
        .map(|id| RoomKey::discord(id.get()))
        .or_else(|| crate::default_channel(bot, ctx))?;
    Some((
        RoomRef {
            id: channel,
            space: Some(space),
            kind: RoomKind::Channel,
            name: None,
            thread: None,
        },
        decode_user(&member.user),
    ))
}

pub(crate) fn decode_emoji(emoji: &ReactionType) -> Emoji {
    match emoji {
        ReactionType::Unicode(text) => Emoji::Unicode(text.clone()),
        ReactionType::Custom { animated, id, name } => Emoji::Custom {
            platform: PlatformKind::Discord,
            id: id.get().to_string(),
            label: name.clone(),
            animated: *animated,
        },
        // Preserve the original display fallback without inventing a native emoji ID.
        _ => Emoji::Unicode("bilinmeyen emoji".to_string()),
    }
}

pub(crate) async fn decode_reaction(
    bot: &crate::Bot,
    ctx: &Context,
    reaction: &Reaction,
) -> Option<ReactionEvent> {
    let bot_id = ctx.cache.current_user().id;
    if reaction.guild_id.is_none() || reaction.message_author_id != Some(bot_id) {
        return None;
    }
    let room = room(reaction.channel_id, reaction.guild_id);
    if bot
        .guild_id
        .as_ref()
        .is_some_and(|guild| room.space.as_ref() != Some(guild))
    {
        return None;
    }
    if bot
        .allowed_channels
        .as_ref()
        .is_some_and(|channels| !channels.contains(&room.id))
    {
        return None;
    }
    let actor = match reaction.user(ctx).await {
        Ok(user) if !user.bot && user.id != bot_id => decode_user(&user),
        Ok(_) => return None,
        Err(error) => {
            log::debug!("reaction_add: couldn't fetch reactor: {error}");
            return None;
        }
    };
    let content = match reaction.message(ctx).await {
        Ok(message) => message.content_safe(&ctx.cache),
        Err(error) => {
            log::debug!("reaction_add: couldn't fetch reacted message: {error}");
            return None;
        }
    };
    // Embed-only slash cards and debug lines carry no bot words to react to.
    if content.trim().is_empty() {
        return None;
    }
    Some(ReactionEvent {
        actor,
        target: MessageRef {
            id: MessageKey::discord(reaction.message_id.get()),
            room: room.id.clone(),
            author: Some(UserKey::discord(bot_id.get())),
        },
        emoji: decode_emoji(&reaction.emoji),
        original_text: Some(content),
        room,
    })
}

// Step-0 characterization tests retain their exact native fixture assertions.
#[cfg(test)]
pub(crate) fn discord_message_input(
    msg: &Message,
    cache: &Arc<serenity::cache::Cache>,
    guild_id: Option<GuildId>,
    allowed_channels: Option<&HashSet<ChannelId>>,
    image_analysis: bool,
) -> Option<(String, Option<String>)> {
    let guild = guild_id.map(|id| SpaceKey::discord(id.get()));
    let channels = allowed_channels.map(|channels| {
        channels
            .iter()
            .map(|id| RoomKey::discord(id.get()))
            .collect()
    });
    decode_message(
        msg,
        cache,
        guild.as_ref(),
        channels.as_ref(),
        image_analysis,
    )
    .map(|message| {
        let image =
            message
                .attachments
                .into_iter()
                .find_map(|attachment| match attachment.source {
                    MediaSource::PublicUrl(url) => Some(url),
                    MediaSource::Native { .. } => None,
                });
        (message.text, image)
    })
}

#[cfg(test)]
pub(crate) fn reaction_label(emoji: &ReactionType) -> String {
    match decode_emoji(emoji) {
        Emoji::Unicode(text) => text,
        Emoji::Custom {
            label: Some(name), ..
        } => format!(":{name}:"),
        Emoji::Custom { label: None, .. } => "bilinmeyen emoji".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serenity::all::{MessageId, Timestamp, UserId};

    fn fixture() -> Message {
        let mut message = Message::default();
        message.id = MessageId::new(81);
        message.channel_id = ChannelId::new(7);
        message.guild_id = Some(GuildId::new(9));
        message.author.id = UserId::new(42);
        message.author.name = "account".into();
        message.author.global_name = Some("Display".into());
        message.timestamp = Timestamp::from_millis(1_700_000_000_123).unwrap();
        message.content = "hello".into();
        message
    }

    #[test]
    fn live_message_decoder_preserves_structured_identities_and_reply_targets() {
        let cache = Arc::new(serenity::cache::Cache::new());
        let mut message = fixture();
        let mut account = User::default();
        account.id = UserId::new(99);
        account.name = "robot".into();
        message.mentions.push(account.clone());
        let mut reply = fixture();
        reply.id = MessageId::new(80);
        reply.author = account;
        message.referenced_message = Some(Box::new(reply));
        let decoded = decode_message(&message, &cache, None, None, false).unwrap();
        assert_eq!(decoded.id, MessageKey::discord(81));
        assert_eq!(decoded.room.id, RoomKey::discord(7));
        assert_eq!(decoded.room.space, Some(SpaceKey::discord(9)));
        assert_eq!(decoded.room.kind, RoomKind::Channel);
        assert_eq!(decoded.author.id, UserKey::discord(42));
        assert_eq!(decoded.author.display_name, "Display");
        assert_eq!(decoded.author.username.as_deref(), Some("account"));
        assert_eq!(decoded.author.automation, Automation::Human);
        assert_eq!(decoded.author.stability, IdentityStability::Persistent);
        assert_eq!(decoded.sent_at_unix_ms, 1_700_000_000_123);
        assert_eq!(decoded.text, "hello");
        assert_eq!(decoded.mentions, vec![UserKey::discord(99)]);
        let reply = decoded.reply_to.unwrap();
        assert_eq!(reply.id, MessageKey::discord(80));
        assert_eq!(reply.room, RoomKey::discord(7));
        assert_eq!(reply.author, Some(UserKey::discord(99)));
        assert!(decoded.attachments.is_empty());
    }

    #[test]
    fn live_message_decoder_carries_only_the_first_enabled_mime_image() {
        let cache = Arc::new(serenity::cache::Cache::new());
        let mut message = fixture();
        message.attachments = ["first", "second"]
            .map(|name| {
                serde_json::from_value(serde_json::json!({
                    "id": "1",
                    "filename": format!("{name}.png"),
                    "content_type": "image/png",
                    "url": format!("https://cdn.discordapp.com/{name}.png"),
                    "proxy_url": "https://media.discordapp.net/proxy.png",
                    "size": 123
                }))
                .unwrap()
            })
            .into();
        let decoded = decode_message(&message, &cache, None, None, true).unwrap();
        assert_eq!(decoded.text, "[resim] hello");
        assert_eq!(decoded.attachments.len(), 1);
        let attachment = &decoded.attachments[0];
        assert_eq!(attachment.kind, MediaKind::Image);
        assert_eq!(attachment.mime.as_deref(), Some("image/png"));
        assert_eq!(attachment.size, Some(123));
        assert!(
            matches!(&attachment.source, MediaSource::PublicUrl(url) if url == "https://cdn.discordapp.com/first.png")
        );
        let decoded = decode_message(&message, &cache, None, None, false).unwrap();
        assert_eq!(decoded.text, "hello");
        assert!(decoded.attachments.is_empty());
    }

    #[test]
    fn common_addressing_uses_decoded_connection_identity_not_another_account() {
        let cache = Arc::new(serenity::cache::Cache::new());
        let mut account = User::default();
        account.id = UserId::new(99);
        account.name = "robot".into();
        let account = decode_user(&account);
        let mut other = account.clone();
        other.id = UserKey::new(PlatformKind::Telegram, &["99"]).unwrap();
        other.username = Some("other".into());
        let mut state = crate::State::default();
        state.accounts.insert(connection_id().clone(), account);
        state
            .accounts
            .insert(ConnectionId::new("telegram-default"), other.clone());
        let mut message = decode_message(&fixture(), &cache, None, None, false).unwrap();
        message.mentions.push(UserKey::discord(99));
        assert!(crate::message_is_tagged(
            &message,
            "",
            state.accounts.get(connection_id())
        ));
        assert!(!crate::message_is_tagged(&message, "", Some(&other)));
        message.mentions.clear();
        message.reply_to = Some(MessageRef {
            id: MessageKey::discord(80),
            room: message.room.id.clone(),
            author: Some(UserKey::discord(99)),
        });
        assert!(crate::message_is_tagged(
            &message,
            "",
            state.accounts.get(connection_id())
        ));
        assert!(!crate::message_is_tagged(&message, "", Some(&other)));
        message.reply_to = None;
        message.text = "ROBOT, hey".into();
        assert!(crate::message_is_tagged(
            &message,
            "",
            state.accounts.get(connection_id())
        ));
        assert!(!crate::message_is_tagged(&message, "", Some(&other)));
        message.text = "KAjU, hey".into();
        assert!(crate::message_is_tagged(&message, "kaju", None));
        assert!(!crate::message_is_tagged(&message, "", None));
    }

    #[test]
    fn custom_reaction_decoder_retains_native_identity_without_raw_mention_markup() {
        let emoji = ReactionType::Custom {
            animated: true,
            id: serenity::all::EmojiId::new(17),
            name: Some("kekw".into()),
        };
        let Emoji::Custom {
            platform,
            id,
            label,
            animated,
        } = decode_emoji(&emoji)
        else {
            panic!("custom emoji must retain its native identity");
        };
        assert_eq!(platform, PlatformKind::Discord);
        assert_eq!(id, "17");
        assert_eq!(label.as_deref(), Some("kekw"));
        assert!(animated);
        assert_eq!(reaction_label(&emoji), ":kekw:");
    }
}
