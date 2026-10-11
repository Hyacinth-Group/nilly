//! Independent Discord client lifecycle; shared shutdown is supplied by the runtime.
use super::{events, handler::DiscordHandler};
use crate::platform::{
    runtime::{EventSender, ShutdownReceiver},
    types::*,
};
use crate::{Bot, BotError};
use serenity::all::{Client, GatewayIntents};
use std::{future::IntoFuture, sync::Arc};

fn intents() -> GatewayIntents {
    GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::GUILD_MEMBERS
        | GatewayIntents::GUILD_MESSAGE_REACTIONS
        | GatewayIntents::MESSAGE_CONTENT
}

pub(crate) async fn run(
    bot: Arc<Bot>,
    connection: ConnectionId,
    token: String,
    ingress: EventSender,
    mut shutdown: ShutdownReceiver,
) -> Result<(), BotError> {
    // No native credentials preflight: the real client returns the native failure.
    let result = async {
        if *shutdown.borrow() {
            return Ok(());
        }
        let mut client = tokio::select! {
            result = Client::builder(&token, intents()).event_handler(DiscordHandler {
                bot, ingress: ingress.clone(), connection: connection.clone(),
                registered: tokio::sync::watch::channel(false).0,
            }).into_future() => result?,
            _ = async {
                while !*shutdown.borrow() {
                    if shutdown.changed().await.is_err() { break; }
                }
            } => return Ok(()),
        };
        let shards = client.shard_manager.clone();
        if *shutdown.borrow() {
            shards.shutdown_all().await;
            return Ok(());
        }
        tokio::select! {
            result = client.start() => result.map_err(BotError::from),
            _ = async {
                while !*shutdown.borrow() {
                    if shutdown.changed().await.is_err() {
                        break;
                    }
                }
            } => {
                shards.shutdown_all().await;
                Ok(())
            }
        }
    }
    .await;
    // Closing this connection must not stop siblings or the shared personality.
    if let Err(error) = ingress
        .send(events::synthetic_envelope(
            &connection,
            ChatEvent::ConnectionClosed,
            "closed",
        ))
        .await
    {
        log::debug!("Discord close ingress [{connection}]: {error}");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_intents_preserve_ordinary_messages_members_and_reactions() {
        assert_eq!(
            intents(),
            GatewayIntents::GUILDS
                | GatewayIntents::GUILD_MESSAGES
                | GatewayIntents::GUILD_MEMBERS
                | GatewayIntents::GUILD_MESSAGE_REACTIONS
                | GatewayIntents::MESSAGE_CONTENT
        );
    }

    #[tokio::test]
    async fn shutdown_before_start_never_builds_client_and_still_closes_connection() {
        let bot = Arc::new(Bot {
            state: std::sync::Mutex::new(crate::State::default()),
            adapters: std::sync::Mutex::new(crate::platform::ConnectionRegistry::default()),
            cycles_started: std::sync::atomic::AtomicBool::new(false),
            announcement_sent: std::sync::atomic::AtomicBool::new(false),
            http: reqwest::Client::new(),
            api_url: "https://custom.invalid/chat/completions".into(),
            key: String::new(),
            news_channel: None,
            firecrawl: None,
            guild_id: None,
            allowed_channels: None,
            debug_channel: None,
            image_analysis: false,
            stealth: false,
            reasoning_mandatory_models: std::sync::Mutex::new(std::collections::HashSet::new()),
        });
        let (ingress, mut receiver) = EventSender::channel(1);
        let (_shutdown, receiver_shutdown) = tokio::sync::watch::channel(true);
        let connection = ConnectionId::new("cancelled-before-connect");
        // An empty native token is deliberately unusable. Cancellation must return
        // before constructing any Serenity client or touching a network endpoint.
        run(
            bot.clone(),
            connection.clone(),
            String::new(),
            ingress,
            receiver_shutdown,
        )
        .await
        .unwrap();
        let closed = receiver.recv().await.unwrap();
        assert_eq!(closed.connection, connection);
        assert!(matches!(closed.event, ChatEvent::ConnectionClosed));
        assert!(receiver.recv().await.is_none());
        assert!(bot.state().accounts.is_empty());
        assert!(!bot.cycles_started.load(std::sync::atomic::Ordering::SeqCst));
    }
}
