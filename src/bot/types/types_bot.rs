/// RAII guard that clears a channel's busy flag when dropped — on the normal return path, an
/// early return, or a panic. Holds: `state` (a reference to `Bot.state`, to reach it without
/// going through `Bot`) and `channel` (which entry in `State.busy` to clear). Constructed by
/// `Bot::reply` (`chat_reply.rs`) at the top of a reply turn; its `Drop` impl below is what
/// actually does the work.
struct BusyGuard<'a> {
    state: &'a Mutex<State>,
    channel: RoomKey,
}

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .busy
            .remove(&self.channel);
    }
}

/// The bot's top-level handle: one instance, wrapped in `Arc`, shared across every Discord
/// event handler and background cycle. Holds:
/// - `state`: the single `Mutex<State>` (see `types_chat_state.rs`).
/// - `http`: the shared `reqwest::Client` used for every outbound request (provider API,
///   Firecrawl, RSS, Hacker News).
/// - `api_url`/`key`: the chosen provider's endpoint and bearer key.
/// - `news_channel`/`guild_id`/`allowed_channels`/`debug_channel`/`image_analysis`: fixed
///   config read once from `.env` in `Bot::setup` (`setup.rs`) — unlike `State`, these never
///   change at runtime (see the `image_analysis` field comment for why that's deliberate).
/// - `firecrawl`: the optional Firecrawl API key; `None` means pages are downloaded plain.
/// - `reasoning_mandatory_models`: a cache of model names known to reject "turn reasoning
///   off" (see `mark_reasoning_mandatory` below).
///
/// Nearly every function in `src/bot/*.rs`, `src/agents.rs`, `src/agenda.rs` is a method on
/// this type (`impl Bot`) or takes `&Bot`/`Arc<Bot>`.
struct Bot {
    state: Mutex<State>,
    http: reqwest::Client,
    api_url: String, // chat/completions endpoint (openrouter or mistral)
    key: String,
    news_channel: Option<RoomKey>,
    firecrawl: Option<String>, // without it, pages are downloaded plain instead
    guild_id: Option<SpaceKey>, // .env GUILD_ID; restricts the Discord connection
    allowed_channels: Option<HashSet<RoomKey>>, // .env CHANNELS; approved rooms
    debug_channel: Option<RoomKey>, // .env DEBUG_CHANNEL; otherwise the same room
    // .env IMAGE_ANALYSIS; read only at startup, no command/button can change it at runtime
    // (deliberate: an operator who turns it off shouldn't be able to turn it back on
    // without restarting the process)
    image_analysis: bool,
    // .env STEALTH; startup-only like image_analysis. When on, nothing bot-shaped reaches
    // Discord: slash commands aren't registered (or touched at all), interactions are ignored,
    // no embeds (announcement, debug trace, prank notice), no thought display/button, and
    // a streamed reply is posted once finished instead of being live-edited in
    stealth: bool,
    // models learned to refuse turning reasoning off (see reasoning_mandatory_error): once
    // a model is known, the "turn it off" attempt is skipped entirely and the call goes
    // out with low-effort reasoning from the start — otherwise every call would eat the
    // same 400 and waste a whole round trip
    reasoning_mandatory_models: Mutex<HashSet<String>>,
}

impl Bot {
    /// Locks and returns the shared state. **Never hold the returned guard across an
    /// `.await`** (see AGENTS.md rule 1) — always drop it before awaiting anything.
    /// Recovers the data after mutex poisoning instead of propagating the prior panic.
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    // adds model usage to the session metrics, broken down by category too (!status dumps this)
    /// Folds one API call's token usage into the session-wide metrics.
    fn add_metric(&self, category: &'static str, usage: Usage) {
        log::debug!(
            "api [{category}]: in={} cache={} out={}",
            usage.prompt_tokens,
            usage.prompt_tokens_details.cached_tokens,
            usage.completion_tokens,
        );
        let mut s = self.state();
        s.metrics.calls += 1;
        s.metrics.input_tokens += usage.prompt_tokens;
        s.metrics.cache_tokens += usage.prompt_tokens_details.cached_tokens;
        s.metrics.output_tokens += usage.completion_tokens;
        s.metrics.last_call_secs = now_unix();
        s.metrics.categories.entry(category).or_default().add(usage);
    }

    // has this model already been seen refusing to turn reasoning off?
    /// Checks the cache built by `mark_reasoning_mandatory` below.
    fn reasoning_mandatory_known(&self, model: &str) -> bool {
        self.reasoning_mandatory_models
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(model)
    }

    // called on the first 400 "mandatory" error; "turn it off" is never tried again for this model
    /// Records that `model` refuses to let reasoning be turned off, so future calls skip
    /// straight to low-effort-reasoning instead of retrying the disable attempt.
    /// Logs only the first insertion.
    fn mark_reasoning_mandatory(&self, model: &str) {
        let inserted = self
            .reasoning_mandatory_models
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(model.to_string());
        if inserted {
            log::info!("reasoning: {model} won't allow turning it off, won't try again");
        }
    }
}
