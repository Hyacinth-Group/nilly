mod auth;
mod dashboard;
mod routes;
mod settings;
mod setup;
mod views;

use crate::{config, Bot, BotError, Handler, SHUTTING_DOWN};
use parking_lot::Mutex;
use serde::Serialize;
use serenity::all::{Client, GatewayIntents, ShardManager};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, watch};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Unconfigured,
    ConfiguredStarting,
    Running,
    ConfigurationError,
    Stopping,
}

impl Lifecycle {
    fn start(&mut self) -> Result<(), &'static str> {
        if !matches!(self, Self::Unconfigured | Self::ConfigurationError) {
            return Err("runtime already started");
        }
        *self = Self::ConfiguredStarting;
        Ok(())
    }
}

pub struct Runtime {
    lifecycle: Lifecycle,
    attempted: bool,
    bot: Option<Arc<Bot>>,
    manager: Option<Arc<ShardManager>>,
    cache: Option<Arc<serenity::cache::Cache>>,
}

pub struct App {
    auth: auth::AuthService,
    origin: auth::OriginPolicy,
    secure: bool,
    config: Mutex<config::Config>,
    runtime: Mutex<Runtime>,
    drafts: Mutex<HashMap<String, setup::Draft>>,
    start: mpsc::Sender<config::Config>,
    stop: watch::Receiver<bool>,
    tests: Arc<tokio::sync::Semaphore>,
    streams: Arc<tokio::sync::Semaphore>,
}

pub async fn entry() -> Result<bool, BotError> {
    let command = std::env::args().nth(1);
    if matches!(command.as_deref(), Some("init" | "chat" | "migrate-durum")) {
        return Ok(false);
    }
    if command.as_deref() == Some("panel") {
        if std::env::args().nth(2).as_deref() != Some("reset-password") {
            return Err("usage: discord-bot panel reset-password".into());
        }
        let directory = state_directory()?;
        crate::memory::init_result(&directory.join("hafiza.redb"))?;
        let auth = auth::AuthService::new().await?;
        let password = auth.reset_password().await?;
        println!("Administrator password replaced. Save this password now: {password}");
        return Ok(true);
    }
    let environment: HashMap<String, String> = std::env::vars().collect();
    let path = PathBuf::from(
        environment
            .get("PANEL_CONFIG_PATH")
            .map_or(".env", String::as_str),
    );
    let config = config::Config::load(&path, &environment)?;
    if config.get("PANEL_ENABLED") == "off" {
        register_secrets(&config)?;
        config::activate(config)?;
        return Ok(false);
    }
    crate::logging::init_with_settings(config.get("LOG_LEVEL"), config.get("LOG_COLOR"));
    run(config).await?;
    Ok(true)
}

fn state_directory() -> Result<PathBuf, BotError> {
    let directory = PathBuf::from(crate::STATE_DIR);
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(directory)
}

async fn run(config: config::Config) -> Result<(), BotError> {
    let bind = config.get("PANEL_BIND").to_string();
    let port: u16 = config.get("PANEL_PORT").parse()?;
    let public_url = config.get("PANEL_PUBLIC_URL");
    let origin = if public_url.is_empty() {
        format!("http://localhost:{port}")
    } else {
        public_url.to_string()
    };
    let origin_policy = auth::OriginPolicy::new(&origin)?;
    let secure = origin_policy.secure();
    if !bind.parse::<std::net::IpAddr>()?.is_loopback() {
        eprintln!("WARNING: panel listens on a non-loopback address. Publish only to localhost or use a private tunnel. Public access requires a reviewed HTTPS proxy.");
    }
    let listener = tokio::net::TcpListener::bind((bind.as_str(), port)).await?;
    let directory = state_directory()?;
    crate::memory::init_result(&directory.join("hafiza.redb"))?;
    let auth = auth::AuthService::new().await?;
    if let Some(token) = auth.renew_claim()? {
        println!(
            "Open this one-use setup link (expires in 15 minutes): {origin}/setup#token={token}"
        );
    }
    let valid = config.validate_runtime().is_ok();
    let initial = if valid {
        Lifecycle::ConfiguredStarting
    } else if config.get("DISCORD_TOKEN").is_empty()
        || (config.get("OPENROUTER_KEY").is_empty() && config.get("MISTRAL_KEY").is_empty())
    {
        Lifecycle::Unconfigured
    } else {
        Lifecycle::ConfigurationError
    };
    let (start, receiver) = mpsc::channel(1);
    let (stop_sender, stop) = watch::channel(false);
    let app = Arc::new(App {
        auth,
        origin: origin_policy,
        secure,
        config: Mutex::new(config.clone()),
        runtime: Mutex::new(Runtime {
            lifecycle: initial,
            attempted: false,
            bot: None,
            manager: None,
            cache: None,
        }),
        drafts: Mutex::new(HashMap::new()),
        start,
        stop: stop.clone(),
        tests: Arc::new(tokio::sync::Semaphore::new(2)),
        streams: Arc::new(tokio::sync::Semaphore::new(8)),
    });
    register_secrets(&config)?;
    if valid {
        app.start.send(config).await?;
    }
    let runtime = tokio::spawn(runtime_owner(app.clone(), receiver, stop.clone()));
    let shutdown_app = app.clone();
    let server = axum::serve(
        listener,
        routes::router(app).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        crate::wait_for_shutdown().await;
        shutdown_app.runtime.lock().lifecycle = Lifecycle::Stopping;
        SHUTTING_DOWN.store(true, Ordering::SeqCst);
        let _ = stop_sender.send(true);
    });
    server.await?;
    runtime.await??;
    Ok(())
}

fn register_secrets(config: &config::Config) -> Result<(), BotError> {
    crate::logging::set_secrets(config.secret_values()).map_err(Into::into)
}

async fn runtime_owner(
    app: Arc<App>,
    mut starts: mpsc::Receiver<config::Config>,
    mut stop: watch::Receiver<bool>,
) -> Result<(), BotError> {
    let config = tokio::select! {
        config = starts.recv() => match config { Some(config) => config, None => return Ok(()) },
        _ = stop.changed() => return Ok(()),
    };
    app.runtime.lock().attempted = true;
    config::activate(config.clone())?;
    let bot = match Bot::setup() {
        Ok(bot) => bot,
        Err(_) => {
            app.runtime.lock().lifecycle = Lifecycle::ConfigurationError;
            let _ = stop.changed().await;
            return Ok(());
        }
    };
    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::GUILD_MEMBERS
        | GatewayIntents::GUILD_MESSAGE_REACTIONS
        | GatewayIntents::MESSAGE_CONTENT;
    let built = Client::builder(config.get("DISCORD_TOKEN"), intents)
        .event_handler(Handler {
            bot: bot.clone(),
            started: AtomicBool::new(false),
            announced: AtomicBool::new(false),
        })
        .await;
    let mut client = match built {
        Ok(client) => client,
        Err(_) => {
            app.runtime.lock().lifecycle = Lifecycle::ConfigurationError;
            let _ = stop.changed().await;
            return Ok(());
        }
    };
    {
        let mut runtime = app.runtime.lock();
        runtime.bot = Some(bot);
        runtime.manager = Some(client.shard_manager.clone());
        runtime.cache = Some(client.cache.clone());
        runtime.lifecycle = Lifecycle::Running;
    }
    tokio::select! {
        result = client.start() => {
            if result.is_err() { app.runtime.lock().lifecycle = Lifecycle::ConfigurationError; }
            if !*stop.borrow() { let _ = stop.changed().await; }
        },
        _ = stop.changed() => { client.shard_manager.shutdown_all().await; },
    }
    Ok(())
}

#[cfg(test)]
mod tests;
