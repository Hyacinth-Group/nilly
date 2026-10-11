// ---------- startup ----------

/// Reads and trims a required environment setting; errors name a missing/empty variable.
fn setting(name: &str) -> Result<String, BotError> {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => Ok(v.trim().to_string()),
        _ => Err(format!("{name} is missing, check .env").into()),
    }
}

impl Bot {
    // picks the provider from environment variables (main loads .env), opens the state
    // directories, loads state from disk, and assembles the bot. Doesn't connect to
    // Discord, doesn't need a token: both main and chat CLI mode go through this.
    /// Assembles a ready-to-use `Bot` from `.env`/disk state, without touching Discord.
    fn setup() -> Result<Arc<Bot>, BotError> {
        // BOT_LANG: resolved once here (first touch), fixed for the rest of the process —
        // every prompts::current()/strings::t() call downstream reads this same choice.
        log::info!("language: {:?}", lang::Lang::current());
        log::info!(
            "owner: {}",
            owner_id().map_or("none".into(), |i| i.to_string())
        );
        // provider choice: PROVIDER=mistral forces it; otherwise whichever key is set,
        // openrouter if both are
        let provider = std::env::var("PROVIDER").unwrap_or_default().to_lowercase();
        let (api_url, key, default_model) = if provider == "mistral"
            || (setting("OPENROUTER_KEY").is_err() && setting("MISTRAL_KEY").is_ok())
        {
            (MISTRAL_URL, setting("MISTRAL_KEY")?, MISTRAL_MODEL)
        } else {
            (OPENROUTER_URL, setting("OPENROUTER_KEY")?, OPENROUTER_MODEL)
        };
        let model = setting("MODEL").unwrap_or_else(|_| default_model.to_string());
        // API_URL, if set, overrides the provider's default endpoint: for pointing an
        // OpenAI-compatible request at your own router (e.g. a local network one)
        let api_url = match std::env::var("API_URL") {
            Ok(v) if !v.trim().is_empty() => v.trim().to_string(),
            _ => api_url.to_string(),
        };
        log::info!("provider: {api_url} · model: {model}");
        // read once at startup only; no command/button toggles it, and it stays fixed until the process restarts
        let image_analysis = !matches!(
            std::env::var("IMAGE_ANALYSIS").unwrap_or_default().trim(),
            "kapali" | "kapalı" | "off" | "hayir" | "hayır" | "0"
        );
        log::info!(
            "image analysis: {}",
            if image_analysis { "on" } else { "off" }
        );
        // defaults to off, the opposite of IMAGE_ANALYSIS: only an explicit "on" enables it
        let stealth = matches!(
            std::env::var("STEALTH").unwrap_or_default().trim(),
            "acik" | "açık" | "on" | "evet" | "1"
        );
        log::info!("stealth: {}", if stealth { "on" } else { "off" });
        let news_channel = match std::env::var("NEWS_CHANNEL") {
            Ok(v) if !v.trim().is_empty() => Some(RoomKey::discord(v.trim().parse()?)),
            _ => None,
        };
        // both optional: if unset, the bot behaves as before and runs in every server/channel it can reach
        let guild_id = match std::env::var("GUILD_ID") {
            Ok(v) if !v.trim().is_empty() => Some(SpaceKey::discord(v.trim().parse()?)),
            _ => None,
        };
        let allowed_channels = match std::env::var("CHANNELS") {
            Ok(v) if !v.trim().is_empty() => {
                let mut s = HashSet::new();
                for part in v.split(',') {
                    let part = part.trim();
                    if part.is_empty() {
                        continue;
                    }
                    s.insert(RoomKey::discord(part.parse()?));
                }
                Some(s)
            }
            _ => None,
        };
        // only arsiv/ is still real files (see memory.rs's module doc); everything else
        // that used to be its own directory tree is now key prefixes inside hafiza.redb
        std::fs::create_dir_all(PathBuf::from(STATE_DIR).join("arsiv"))?;
        std::fs::create_dir_all(IMAGE_DIR)?;
        memory::init_result(&PathBuf::from(STATE_DIR).join("hafiza.redb"))?;

        let mut state = State::load();
        sleep::update(&mut state);
        let selected = memory::read("model.md");
        state.model = if selected.trim().is_empty() {
            model
        } else {
            selected.trim().to_string()
        };
        log::info!("model: {}", state.model);
        Ok(Arc::new(Bot {
            state: Mutex::new(state),
            adapters: Mutex::new(platform::ConnectionRegistry::default()),
            // let a failed connection attempt fail fast (CONNECT_TIMEOUT); no overall
            // time limit, only a per-chunk cap of READ_TIMEOUT (P0: a single 60s timeout
            // used to be able to cut off a long thinking stream mid-way)
            http: reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .read_timeout(READ_TIMEOUT)
                .build()?,
            api_url,
            key,
            news_channel,
            firecrawl: std::env::var("FIRECRAWL_KEY")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            guild_id,
            allowed_channels,
            debug_channel: std::env::var("DEBUG_CHANNEL")
                .ok()
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map(RoomKey::discord),
            image_analysis,
            stealth,
            reasoning_mandatory_models: Mutex::new(HashSet::new()),
        }))
    }

    // debug mode: the decision trace as a single line. If off, nothing happens (the
    // caller only builds the format! while debug is on). Not written to memory or the
    // channel note — the bot shouldn't mistake it for its own words — and since it's a
    // bot message, it doesn't reach the message handler either
    async fn debug_note(&self, _ctx: &Context, channel: &RoomKey, text: String) {
        if !self.state().debug {
            return;
        }
        log::info!("debug [{channel}]: {text}");
        // stealth: the trace stays in the log, never posted as an embed
        if self.stealth {
            return;
        }
        let target = self.debug_channel.as_ref().unwrap_or(channel);
        let body: String = text.chars().take(300).collect();
        self.send_card(target, strings::t("debug.trace_title"), &body).await;
    }

    // sends several traces as a single line; silent if there's nothing to trace or debug is off

    async fn debug_trace(
        &self,
        _ctx: &Context,
        channel: &RoomKey,
        enabled: bool,
        trace: &[String],
    ) {
        if enabled && !trace.is_empty() {
            self.debug_note(_ctx, channel, trace.join(" · ")).await;
        }
    }

    /// Sends presentation-only diagnostics without adding them to conversational memory.
    async fn send_card(&self, channel: &RoomKey, title: &str, description: &str) {
        let transport = match self.transport(channel) {
            Ok(transport) => transport,
            Err(e) => { log::warn!("card [{channel}]: {e}"); return; }
        };
        let room = platform::types::RoomRef::from_key(channel.clone());
        let mut body = platform::outgoing::OutgoingMessage::text("");
        body.presentation = Some(platform::outgoing::PresentationPayload::Card { title, description });
        if let Err(e) = transport.send(&room, &body).await {
            log::warn!("couldn't send card ({channel}): {e}");
        }
    }
}

/// Waits for ctrl-c or, on Unix, SIGTERM.
async fn wait_for_shutdown() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("signal");
        tokio::select! {
            _ = ctrl_c => {},
            _ = term.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        let _ = ctrl_c.await;
    }
}
