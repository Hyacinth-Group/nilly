//! Native gateway callbacks only decode/enrich and feed the bounded shared ingress.
use super::{commands, events, DiscordAdapter};
use crate::platform::{runtime::EventSender, types::*, Adapter};
use crate::Bot;
use serenity::{all::*, async_trait};
use std::sync::Arc;

pub(super) struct DiscordHandler {
    pub(super) bot: Arc<Bot>,
    pub(super) ingress: EventSender,
    pub(super) connection: ConnectionId,
    pub(super) registered: tokio::sync::watch::Sender<bool>,
}

impl DiscordHandler {
    async fn emit(&self, event: ChatEvent, scope: &str) {
        let envelope = events::synthetic_envelope(&self.connection, event, scope);
        if let Err(error) = self.ingress.send(envelope).await {
            log::debug!("Discord ingress [{}]: {error}", self.connection);
        }
    }
}

#[async_trait]
impl EventHandler for DiscordHandler {
    async fn ready(&self, ctx: Context, data: Ready) {
        let account = events::decode_user(&data.user);
        // Registration and account metadata precede the first UI await: guild callbacks
        // and their outbound work can already resolve this actual connection.
        match self
            .bot
            .register_adapter(Adapter::Discord(DiscordAdapter::new(
                &ctx,
                self.connection.clone(),
                account.clone(),
            ))) {
            Ok(_) => {}
            Err(error) => {
                log::error!(
                    "couldn't register Discord adapter [{}]: {error}",
                    self.connection
                );
                return;
            }
        }
        self.registered.send_replace(true);
        log::info!("logged in [{}]: {}", self.connection, data.user.name);
        if data.guilds.is_empty() {
            log::warn!(
                "not in any server yet, invite me: {}",
                crate::init::invite_url(data.application.id.get())
            );
        }
        // Quiet mode never touches the command API, including stale registrations.
        if !self.bot.stealth {
            for guild in ctx.cache.guilds() {
                if let Err(error) = super::modal::register_commands(&ctx.http, guild).await {
                    log::warn!("couldn't register slash commands [{guild}]: {error}");
                }
            }
        }
        let scope = format!("ready:{}", account.id);
        self.emit(ChatEvent::ConnectionReady(account), &scope).await;
    }

    async fn guild_create(&self, _ctx: Context, guild: Guild, _new: Option<bool>) {
        // Discovery is not approval. Core applies the configured message/history
        // policy, but keeps all discovered spaces for the legacy nickname scope.
        let mut registered = self.registered.subscribe();
        loop {
            if *registered.borrow() {
                break;
            }
            if registered.changed().await.is_err() {
                return;
            }
        }
        let space = events::decode_space(&guild);
        if let Some(channel) = guild
            .system_channel_id
            .and_then(|id| guild.channels.get(&id))
            .filter(|channel| channel.kind != ChannelType::Text)
        {
            let room = events::decode_room(channel);
            let scope = format!("room:{}", room.id);
            self.emit(ChatEvent::RoomReady(room), &scope).await;
        }
        let scope = format!("space:{}", guild.id);
        self.emit(
            ChatEvent::SpaceReady {
                space,
                history: HistoryInitializationPolicy::AttemptOnce,
            },
            &scope,
        )
        .await;
    }

    async fn guild_member_addition(&self, ctx: Context, member: Member) {
        if let Some((room, user)) = events::decode_member_join(&self.bot, &ctx, &member) {
            let scope = format!("member:{}:{}", room.id, user.id);
            self.emit(ChatEvent::MemberJoined { room, user }, &scope)
                .await;
        }
    }

    async fn message(&self, ctx: Context, msg: Message) {
        let Some(message) = events::decode_message(
            &msg,
            &ctx.cache,
            self.bot.guild_id.as_ref(),
            self.bot.allowed_channels.as_ref(),
            self.bot.image_analysis,
        ) else {
            return;
        };
        let envelope = events::message_envelope(&self.connection, message);
        if let Err(error) = self.ingress.send(envelope).await {
            log::debug!("Discord ingress [{}]: {error}", self.connection);
        }
    }

    async fn reaction_add(&self, ctx: Context, reaction: Reaction) {
        if let Some(event) = events::decode_reaction(&self.bot, &ctx, &reaction).await {
            let scope = format!(
                "reaction:{}:{}:{}",
                event.room.id, event.target.id, event.actor.id
            );
            self.emit(ChatEvent::ReactionAdded(event), &scope).await;
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        commands::interaction_create(&self.bot, &self.connection, &ctx, interaction).await;
    }
}
