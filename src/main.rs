mod agenda;
mod agents;
mod chat_cli;
pub mod command;
mod growth;
mod init;
mod lang;
mod logging;
mod memory;
mod memory_migration;
mod migrate;
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
#[cfg(test)]
use platform::discord::render::{stream_view, thought_counter, thought_display};
use platform::types::*;
use platform::types::{Emoji, UserRef};
use platform::Platform;
use serde::{Deserialize, Serialize};
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
include!("bot/lifecycle.rs");

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
    let mut connections = Vec::new();
    if let Ok(token) = setting("DISCORD_TOKEN") {
        connections.push(platform::runtime::ConnectionConfig::Discord {
            connection: ConnectionId::new("discord-default"),
            token,
        });
    }
    match (setting("SLACK_APP_TOKEN"), setting("SLACK_BOT_TOKEN")) {
        (Ok(app_token), Ok(bot_token)) => {
            connections.push(platform::runtime::ConnectionConfig::Slack {
                connection: ConnectionId::new("slack-default"),
                app_token,
                bot_token,
            })
        }
        (Err(_), Err(_)) => {}
        _ => {
            return Err(platform::PlatformError::Transport(
                strings::t("startup.slack_token_pair").into(),
            )
            .into())
        }
    }
    if connections.is_empty() {
        eprintln!("{}", strings::t("startup.no_platform"));
        std::process::exit(1);
    }
    let bot = Bot::setup()?;
    platform::runtime::run(bot, connections).await
}
include!("bot/tests/tests.rs");
