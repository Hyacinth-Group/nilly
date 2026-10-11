/// One-time (per guild) scan of the last `HISTORY_DAYS` of messages, on first join.
/// Input: `bot: &Bot`; `ctx: &Context`; `guild: &Guild`. Output: none (populates
/// `state.recent_messages`/`name_to_id`/`favorite_name`, paginating via `GetMessages`).
/// Uses: `display_name`, `now_unix`, `bot.state()`. Used by: `Handler::guild_create`
/// (`handler_event.rs`), the only caller (spawned in the background, once per guild).
async fn read_history(bot: &Bot, ctx: &Context, guild: &Guild) {
    let bot_id = ctx.cache.current_user().id;
    let member = match guild.member(ctx, bot_id).await {
        Ok(u) => u,
        Err(e) => {
            log::warn!("{}: couldn't get membership: {e}", guild.name);
            return;
        }
    };
    let cutoff = now_unix() - HISTORY_DAYS * 24 * 60 * 60;

    let mut channels: Vec<&GuildChannel> = guild
        .channels
        .values()
        .filter(|k| k.kind == ChannelType::Text)
        .collect();
    channels.sort_by_key(|k| k.position);

    let mut collected: Vec<(i64, String, UserKey, String)> = Vec::new();
    for ch in channels {
        if bot
            .allowed_channels
            .as_ref()
            .is_some_and(|s| !s.contains(&RoomKey::discord(ch.id.get())))
        {
            continue;
        }
        let perms = guild.user_permissions_in(ch, &member);
        if !perms.contains(Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY) {
            continue;
        }
        let mut before: Option<MessageId> = None;
        loop {
            let mut query = GetMessages::new().limit(100);
            if let Some(id) = before {
                query = query.before(id);
            }
            let batch = match ch.id.messages(&ctx.http, query).await {
                Ok(p) if !p.is_empty() => p,
                _ => break,
            };
            let mut hit_cutoff = false;
            for m in &batch {
                if m.timestamp.unix_timestamp() < cutoff {
                    hit_cutoff = true;
                    break;
                }
                if !m.author.bot && !m.content.trim().is_empty() {
                    collected.push((
                        m.timestamp.unix_timestamp(),
                        display_name(&m.author),
                        UserKey::discord(m.author.id.get()),
                        m.content_safe(&ctx.cache),
                    ));
                }
            }
            if hit_cutoff || batch.len() < 100 {
                break;
            }
            before = batch.last().map(|m| m.id);
        }
    }

    // discord hands back newest-to-oldest, we want oldest-to-newest
    collected.sort_by_key(|t| t.0);
    let skip = collected.len().saturating_sub(MEMORY_SIZE);
    let mut state = bot.state();
    for (_, name, id, _) in collected.iter().skip(skip) {
        if is_owner(id) {
            state.favorite_name = Some(name.clone());
        }
        // live mappings take priority: the scan shouldn't overwrite them with stale info
        state.name_to_id.entry(name.to_lowercase()).or_insert_with(|| id.clone());
    }
    // live messages may have entered memory while the scan was running: history is
    // prepended, not appended — appending would break chronological order and get
    // overwritten by the live messages
    for (_, name, _, text) in collected.iter().skip(skip).rev() {
        state.recent_messages.push_front(format!("{name}: {text}"));
    }
    while state.recent_messages.len() > MEMORY_SIZE {
        state.recent_messages.pop_front();
    }
    log::debug!("{}: read {} message(s)", guild.name, collected.len());
}

// channel used for news and welcome messages: the configured one if set, else the
// server's system channel, else the topmost text channel
/// Input: `bot: &Bot`; `ctx: &Context`. Output: `Option<RoomKey>` — see comment above;
/// `None` if nothing qualifies. Uses: `bot.news_channel`/`allowed_channels`/`guild_id`,
/// `ctx.cache`. Used by: `news_cycle`/`post_problem` callers (`cycle_news.rs`), `poke_cycle`
/// (`cycle_background.rs`), `Handler::guild_member_addition`/`guild_create`
/// (`handler_event.rs`), `Bot::pick_name` (`cycle_growth.rs`).
fn default_channel(bot: &Bot, ctx: &Context) -> Option<RoomKey> {
    if let Some(k) = &bot.news_channel {
        return Some(k.clone());
    }
    let allowed = |id: ChannelId| {
        bot.allowed_channels
            .as_ref()
            .is_none_or(|s| s.contains(&RoomKey::discord(id.get())))
    };
    for guild_id in ctx.cache.guilds() {
        if bot.guild_id.as_ref().is_some_and(|g| *g != SpaceKey::discord(guild_id.get())) {
            continue;
        }
        let Some(guild) = ctx.cache.guild(guild_id) else {
            continue;
        };
        if let Some(k) = guild.system_channel_id.filter(|id| allowed(*id)) {
            return Some(RoomKey::discord(k.get()));
        }
        if let Some(k) = guild
            .channels
            .values()
            .filter(|k| k.kind == ChannelType::Text && allowed(k.id))
            .min_by_key(|k| k.position)
        {
            return Some(RoomKey::discord(k.id.get()));
        }
    }
    None
}

// ---------- background cycles ----------

// keeps a cycle alive: on a panic or an unexpected return, logs it and restarts after 5s
// (the panic hook writes the backtrace; this watchdog prevents a silent death; the 5s
// wait is on both branches so a cycle that returns immediately after starting can't burn CPU).
/// Input: `name: &'static str` — for log messages; `build: F` — a closure that produces a
/// fresh cycle future each restart (`F: Fn() -> Fut`). Output: none (spawns a `tokio::spawn`
/// supervisor loop that never returns until `SHUTTING_DOWN` is set). Uses: `tokio::spawn`,
/// `SHUTTING_DOWN`. Used by: `Handler::ready` (`handler_event.rs`), once per background
/// cycle (`news_cycle`, `poke_cycle`, `prank_cycle`, `wanderer_cycle`, `memory_cycle`,
/// `sleep_cycle`).
// Never restarts once the shutdown signal is set.
fn run_cycle<F, Fut>(name: &'static str, build: F)
where
    F: Fn() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        loop {
            if SHUTTING_DOWN.load(Ordering::SeqCst) {
                return;
            }
            match tokio::spawn(build()).await {
                Ok(()) => {
                    if SHUTTING_DOWN.load(Ordering::SeqCst) {
                        return;
                    }
                    // cycles are meant to run forever; if one returns on its own, restart it anyway
                    log::warn!("cycle [{name}]: returned unexpectedly, restarting");
                }
                Err(e) => {
                    log::error!("cycle [{name}]: panicked, restarting in 5s: {e}");
                }
            }
            sleep(Duration::from_secs(5)).await;
        }
    });
}

// every six hours: the agents run, then a hacker news item gets posted
