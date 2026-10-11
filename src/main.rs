mod agenda;
mod agents;
mod chat_cli;
mod command;
mod growth;
mod init;
mod lang;
mod logging;
mod memory;
mod memory_migration;
mod migrate;
mod modal;
pub mod platform;
mod prompts;
mod sleep;
mod strings;
mod travel;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use agents::random_image;
use platform::capabilities::{CountUnit, OutputLimits};
use platform::discord::render::thought_display;
#[cfg(test)]
use platform::discord::render::{stream_view, thought_counter};
use platform::types::*;
use platform::types::{Emoji, UserRef};
use platform::Platform;
use serde::{Deserialize, Serialize};
use serenity::all::*;
use serenity::async_trait;
use tokio::time::sleep;

// The old content of this file (constants, State/Bot/Handler, the chat engine, the
// background cycles, Discord events) was split by topic into src/bot/. `include!` is
// used (not real `mod`) so visibility and `use super::*` never change anywhere — these
// files compile as if they were written inline in main.rs's own module, exactly like
// before the split, so agents.rs/command.rs/modal.rs/agenda.rs/chat_cli.rs/sleep.rs
// keep the same access they always had.
include!("bot/types/types.rs");
include!("bot/text/text.rs");
include!("bot/provider/provider.rs");
include!("bot/chat/chat.rs");
include!("bot/cycle/cycle.rs");
include!("bot/handler/handler.rs");
include!("bot/setup.rs");

/// Runs the Discord client or terminal chat bench until shutdown or failure.
#[tokio::main]
async fn main() -> Result<(), BotError> {
    dotenvy::dotenv().ok();
    logging::init();
    // panics should land in the log with a backtrace, so a background task that dies
    // silently in a spawned cycle still leaves a trace
    std::panic::set_hook(Box::new(|panic_info| {
        let trace = std::backtrace::Backtrace::force_capture();
        log::error!("PANIC: {panic_info}\n{trace}");
    }));
    if std::env::args().nth(1).as_deref() == Some("init") {
        let args: Vec<String> = std::env::args().skip(2).collect();
        if let Err(e) = init::run(&args) {
            eprintln!("init failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    // `cargo run -- migrate-durum`: one-time import of a pre-redb durum/ markdown tree
    // into hafiza.redb (see migrate.rs). Never touches Discord.
    if std::env::args().nth(1).as_deref() == Some("migrate-durum") {
        let args: Vec<String> = std::env::args().skip(2).collect();
        if let Err(e) = migrate::run(&args) {
            eprintln!("migrate-durum failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    // `cargo run -- chat`: a terminal chat bench that never connects to Discord.
    // No token needed, only a model key; missing key prints one line and exits 1
    if std::env::args().nth(1).as_deref() == Some("chat") {
        let bot = match Bot::setup() {
            Ok(b) => b,
            Err(e) => {
                eprintln!("chat mode failed to start: {e}");
                std::process::exit(1);
            }
        };
        bot.chat_cli().await;
        return Ok(());
    }
    let token = match setting("DISCORD_TOKEN") {
        Ok(token) => token,
        Err(_) => {
            eprintln!("DISCORD_TOKEN is missing. Run `discord-bot init` (or `cargo run -- init`) to create .env.");
            std::process::exit(1);
        }
    };
    let bot = Bot::setup()?;

    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::GUILD_MEMBERS
        | GatewayIntents::GUILD_MESSAGE_REACTIONS
        | GatewayIntents::MESSAGE_CONTENT;

    let mut client = Client::builder(&token, intents)
        .event_handler(Handler {
            bot,
            started: AtomicBool::new(false),
            announced: AtomicBool::new(false),
        })
        .await?;

    // shut down cleanly on ctrl+c or sigterm
    let shard_manager = client.shard_manager.clone();
    tokio::spawn(async move {
        wait_for_shutdown().await;
        log::info!("shutting down");
        // cycles should not open a new round, and the watchdog should not restart them
        SHUTTING_DOWN.store(true, Ordering::SeqCst);
        shard_manager.shutdown_all().await;
    });

    client.start().await?;
    Ok(())
}
include!("bot/tests/tests.rs");
