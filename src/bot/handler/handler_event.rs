/// Serenity's `EventHandler` implementor. Holds `bot: Arc<Bot>` (shared with every
/// background cycle) plus two once-per-process flags: `started` (guards against starting
/// the background cycles twice on a reconnect) and `announced` (the startup version
/// announcement, see `guild_create` below).
struct Handler {
    bot: Arc<Bot>,
    started: AtomicBool,
    announced: AtomicBool, // the version announcement goes out once per process
}

#[async_trait]
impl EventHandler for Handler {
    /// Serenity's `ready` callback — fires once per gateway connection (including
    /// reconnects). Background cycles start only once, guarded by `self.started`.
    async fn ready(&self, ctx: Context, data: Ready) {
        {
            let mut state = self.bot.state();
            state.username = data.user.name.clone();
            state.bot_name = state
                .growth
                .name
                .clone()
                .unwrap_or_else(|| data.user.name.clone());
        }
        log::info!("logged in: {}", data.user.name);
        if data.guilds.is_empty() {
            log::warn!(
                "not in any server yet, invite me: {}",
                init::invite_url(data.application.id.get())
            );
        }

        // slash commands (/durum /yardim /zihin): registered per guild, idempotent. In
        // stealth the command API isn't touched at all
        if !self.bot.stealth {
            for guild in ctx.cache.guilds() {
                if let Err(e) = modal::register_commands(&ctx.http, guild).await {
                    log::warn!("couldn't register slash commands [{guild}]: {e}");
                }
            }
        }

        // ready fires again on every reconnect; the cycles should only start once —
        // the watchdog restarts them on panic
        if !self.started.swap(true, Ordering::SeqCst) {
            let (bot, ctx2) = (self.bot.clone(), ctx.clone());
            run_cycle("news", move || news_cycle(bot.clone(), ctx2.clone()));
            let (bot, ctx2) = (self.bot.clone(), ctx.clone());
            run_cycle("poke", move || poke_cycle(bot.clone(), ctx2.clone()));
            let (bot, ctx2) = (self.bot.clone(), ctx.clone());
            run_cycle("prank", move || prank_cycle(bot.clone(), ctx2.clone()));
            let bot = self.bot.clone();
            run_cycle("wanderer", move || wanderer_cycle(bot.clone()));
            let bot = self.bot.clone();
            run_cycle("memory", move || memory_cycle(bot.clone()));
            let (bot, ctx2) = (self.bot.clone(), ctx.clone());
            run_cycle("sleep", move || sleep_cycle(bot.clone(), ctx2.clone()));
        }
    }

    /// Serenity's `guild_create` callback — fires once per guild on every connect/reconnect.
    async fn guild_create(&self, ctx: Context, guild: Guild, _new: Option<bool>) {
        // if GUILD_ID is set, only that guild gets scanned; it also never enters the on-disk scanned list
        if self.bot.guild_id.is_some_and(|g| g != guild.id) {
            return;
        }
        let first_time = {
            let mut state = self.bot.state();
            let is_new = state.scanned.insert(guild.id);
            if is_new {
                let list: Vec<String> = state.scanned.iter().map(|g| g.get().to_string()).collect();
                memory::write("taranan.md", &list.join("\n"));
            }
            is_new
        };
        // once per restart: shows which build is running, in-channel (requested by Emin).
        // The guild cache isn't populated yet in `ready`, so the channel is found here
        // instead. Not written to memory: the bot shouldn't mistake this for its own
        // words and start making small talk about versions.
        if !self.bot.stealth && !self.announced.swap(true, Ordering::SeqCst) {
            if let Some(channel) = default_channel(&self.bot, &ctx) {
                let (model, mode) = {
                    let state = self.bot.state();
                    (state.model.clone(), state.thinking_mode.label())
                };
                let description = strings::t("announce.description")
                    .replace("{model}", &model)
                    .replace("{mode}", mode);
                let title = format!("{} · {}", strings::t("announce.title"), version_text());
                let embed = modal::info_embed(&title, &description);
                let msg = CreateMessage::new().embed(embed);
                if let Err(e) = channel.send_message(&ctx.http, msg).await {
                    log::warn!("couldn't send version announcement ({channel}): {e}");
                }
            }
        }
        if !first_time {
            return;
        }
        let bot = self.bot.clone();
        tokio::spawn(async move {
            read_history(&bot, &ctx, &guild).await;
            bot.profiler().await;
            if bot.state().temperament.is_empty() {
                bot.coach().await;
            }
        });
    }

    /// Serenity's `guild_member_addition` callback — fires when a new member joins.
    async fn guild_member_addition(&self, ctx: Context, member: Member) {
        if self.bot.guild_id.is_some_and(|g| g != member.guild_id) {
            return;
        }
        let channel = {
            let system = ctx
                .cache
                .guild(member.guild_id)
                .and_then(|g| g.system_channel_id);
            match system.or_else(|| default_channel(&self.bot, &ctx)) {
                Some(k) => k,
                None => return,
            }
        };
        let name = display_name(&member.user);
        {
            let mut state = self.bot.state();
            if is_owner(member.user.id.get()) {
                state.favorite_name = Some(name.clone());
            }
            if state.chats.contains_key(&channel) {
                return;
            }
        }

        let greeting = match self
            .bot
            .generate(
                &[user(format!("{name} sunucuya yeni katıldı."))],
                prompts::current().welcome,
                Some(200),
                "hos_geldin",
            )
            .await
        {
            Ok(s) => s,
            Err(e) => {
                log::error!("ai [hos_geldin]: {e}");
                return;
            }
        };
        // the mention is attached at send time on the first line (send_lines adds it):
        // pasting it onto the front of the text made the "-" and "tepki:" protocol lines
        // unrecognizable
        match self
            .bot
            .send_lines(&ctx, channel, &greeting, None, None, Some(member.user.id))
            .await
        {
            Some(p) => {
                start_chat(&mut self.bot.state(), channel, Some(p));
            }
            None => log::debug!("hos_geldin: model stayed silent, skipped"),
        }
    }

    /// Serenity's `message` callback — the core of the bot's Discord-side logic (see
    /// `docs/flows.md`'s "A message arrives" for the full step list).
    async fn message(&self, ctx: Context, msg: Message) {
        // bots, webhooks, and DMs are excluded, so there's no bot-to-bot loop
        if msg.author.bot || msg.webhook_id.is_some() || msg.guild_id.is_none() {
            return;
        }
        // if GUILD_ID/CHANNELS are set, only the allowed guild/channel is handled (.env, optional)
        if self.bot.guild_id.is_some_and(|g| msg.guild_id != Some(g)) {
            return;
        }
        if self
            .bot
            .allowed_channels
            .as_ref()
            .is_some_and(|k| !k.contains(&msg.channel_id))
        {
            return;
        }
        let raw_text = msg.content_safe(&ctx.cache);
        // the first attached image goes to the model; a message that's just an image (no
        // text) is still processed. When IMAGE_ANALYSIS is off, it's never looked at at all
        // (Bot::image_analysis is read only at startup, no command can change it) — the
        // message is handled as if it had no attachment at all.
        let image = self
            .bot
            .image_analysis
            .then(|| {
                msg.attachments
                    .iter()
                    .find(|att| {
                        att.content_type
                            .as_deref()
                            .is_some_and(|t| t.starts_with("image/"))
                    })
                    .map(|att| att.url.clone())
            })
            .flatten();
        if raw_text.trim().is_empty() && image.is_none() {
            return;
        }
        // memory, the channel note, and the chat line all carry the same text: the image marker lives inside the text
        let text = match &image {
            None => raw_text,
            Some(_) if raw_text.trim().is_empty() => "[resim attı]".to_string(),
            Some(_) => format!("[resim] {raw_text}"),
        };
        let channel = msg.channel_id;
        let name = display_name(&msg.author);
        let bot_id = ctx.cache.current_user().id;

        // phase 1 (locked): recording facts + flag decisions
        let (direct_reply, tagged, evaluate, ongoing_dialog, debug) = {
            let mut state = self.bot.state();
            // was it mentioned, named, or replied to?
            let tagged = msg.mentions.iter().any(|u| u.id == bot_id)
                || msg
                    .referenced_message
                    .as_ref()
                    .is_some_and(|r| r.author.id == bot_id)
                || [&state.bot_name, &state.username]
                    .iter()
                    .any(|a| !a.is_empty() && text.to_lowercase().contains(&a.to_lowercase()));
            state.growth.messages += 1;
            remember(&mut state, &name, &text);
            state
                .name_to_id
                .insert(name.to_lowercase(), msg.author.id.get());
            state
                .usernames
                .insert(msg.author.id.get(), msg.author.name.clone());
            state.last_channel = Some(channel);
            if is_owner(msg.author.id.get()) {
                state.favorite_name = Some(name.clone());
            }

            // news was posted, a comment was expected, but the window ran out
            if state
                .awaiting_comment
                .get(&channel)
                .is_some_and(|t| Instant::now() > *t)
            {
                state.chats.remove(&channel);
                state.awaiting_comment.remove(&channel);
            }

            // no writing while asleep; a mention gets a reply once it wakes up
            if !sleep::is_awake(&state) {
                if tagged {
                    state
                        .pending_mentions
                        .push((channel, format!("{name}: {text}")));
                    if state.pending_mentions.len() > 20 {
                        state.pending_mentions.remove(0);
                    }
                }
                return;
            }

            let open = state.chats.contains_key(&channel);
            // an open chat alone doesn't mean "reply to everyone": like a real person, it
            // automatically continues with whoever it JUST talked to (the chat's last user
            // message belongs to the same person who sent this one), but if someone else
            // in the channel wrote, or the chat has gone cold, it still runs the
            // willingness evaluation.
            let ongoing_dialog = open
                && state.chats.get(&channel).is_some_and(|chat| {
                    chat.history
                        .iter()
                        .rev()
                        .find(|m| m.role == "user")
                        .and_then(|m| m.content.split_once(": ").map(|(speaker, _)| speaker))
                        // eq_ignore_ascii_case misses on Turkish İ/i̇; casefold exists exactly for this
                        .is_some_and(|speaker| casefold(speaker) == casefold(&name))
                });
            let evaluate = if !tagged && !ongoing_dialog {
                // rate limit: at most one willingness call every 2 minutes per channel
                let now = Instant::now();
                let allowed = state
                    .last_evaluation
                    .get(&channel)
                    .is_none_or(|t| now.duration_since(*t) >= EVALUATION_INTERVAL);
                if allowed {
                    state.last_evaluation.insert(channel, now);
                }
                allowed
            } else {
                false
            };
            (
                tagged || ongoing_dialog,
                tagged,
                evaluate,
                ongoing_dialog,
                state.debug,
            )
        };

        // phase 2 (unlocked): willingness evaluation — doesn't jump on every message,
        // weighs topic/personality/interest; a mention and an ongoing dialog are already answered directly
        let mut join = direct_reply;
        // debug trace: the reasoning behind the decision (only posted to the channel when debug is on)
        let mut trace = if tagged {
            "tag".to_string()
        } else if ongoing_dialog {
            "dialog ongoing (same person)".to_string()
        } else if evaluate {
            String::new()
        } else {
            "willingness: 2min limit, not evaluated".to_string()
        };
        if evaluate {
            let threshold = {
                let state = self.bot.state();
                // stage confidence scales the threshold: new is shy, an old hand is relaxed
                let mut threshold = WILLINGNESS_THRESHOLD as i32;
                let confidence = growth::stage(&state.growth).confidence;
                if confidence < 0.9 {
                    threshold += 1;
                } else if confidence > 1.1 {
                    threshold -= 1;
                }
                if travel::now().is_some() {
                    threshold += 2; // joins in less while traveling
                }
                threshold
            };
            match self.bot.willingness().await {
                Some((score, reason)) => {
                    log::debug!("willingness [{channel}]: score={score} threshold={threshold} reason={reason}");
                    join = i32::from(score) >= threshold;
                    if debug {
                        trace = format!("willingness {score}/{threshold} · reason: {reason}");
                    }
                }
                None => {
                    // the call failed: fall back to the old die roll
                    join = rand::random::<f64>() < CHANCE;
                    log::debug!("willingness [{channel}]: no call, fallback die={join}");
                    if debug {
                        trace = format!("willingness: no call → fallback die {join}");
                    }
                }
            }
        }

        // phase 3 (locked): join the chat and process the message
        let should_reply = {
            let mut state = self.bot.state();
            let mut open = state.chats.contains_key(&channel);
            if !open && join {
                start_chat(&mut state, channel, None);
                open = true;
            }
            if let Some(chat) = state.chats.get_mut(&channel) {
                // only the latest image goes to the model: older entries' links are dropped
                for m in chat.history.iter_mut() {
                    m.image = None;
                }
                chat.history.push(match &image {
                    Some(url) => user_with_image(format!("{name}: {text}"), url),
                    None => user(format!("{name}: {text}")),
                });
                chat.last_message = Some(msg.id);
                chat.last_was_tagged = tagged;
                chat.incoming += 1;
                chat.recent_arrivals.push_back((name.clone(), msg.id));
                if chat.recent_arrivals.len() > 20 {
                    chat.recent_arrivals.pop_front();
                }
                if chat.history.len() > CHAT_SIZE {
                    chat.history.drain(..chat.history.len() - CHAT_SIZE);
                }
                state.last_activity.insert(channel, Instant::now());
            }
            channel_note(&mut state, channel, format!("{name}: {text}"));
            // the willingness result also applies to an already-open chat: if someone
            // else wrote and the score was below the threshold, the message still enters
            // history but gets no reply (otherwise the evaluation would just burn tokens for nothing)
            open && join
        };

        if debug {
            let decision = if should_reply { "reply" } else { "silent" };
            self.bot
                .debug_note(&ctx, channel, format!("{trace} → {decision}"))
                .await;
        }
        if should_reply {
            self.bot.reply(&ctx, channel).await;
        }
    }

    // a reaction on the bot's own message is treated as a social signal, the same way a
    // spoken message is: it's recorded, and if a chat is open it enters the model's
    // context too — but unlike a message, it never triggers a reply by itself (no
    // willingness check, no new message goes out); the bot just notices it and may bring
    // it up next time it naturally replies
    /// Serenity's `reaction_add` callback — fires when anyone reacts to any message.
    /// Only records human reactions to the bot's text messages in a guild.
    /// Fetches the reactor and message over HTTP because `Reaction` carries no text.
    async fn reaction_add(&self, ctx: Context, add_reaction: Reaction) {
        let bot_id = ctx.cache.current_user().id;
        // DMs, and reactions on someone else's message, aren't this bot's business
        if add_reaction.guild_id.is_none() || add_reaction.message_author_id != Some(bot_id) {
            return;
        }
        if self
            .bot
            .guild_id
            .is_some_and(|g| add_reaction.guild_id != Some(g))
        {
            return;
        }
        if self
            .bot
            .allowed_channels
            .as_ref()
            .is_some_and(|k| !k.contains(&add_reaction.channel_id))
        {
            return;
        }
        let reactor = match add_reaction.user(&ctx).await {
            // a bot's own `tepki:` reaction on its own earlier message, or another bot: skip
            Ok(u) if !u.bot && u.id != bot_id => u,
            Ok(_) => return,
            Err(e) => {
                log::debug!("reaction_add: couldn't fetch reactor: {e}");
                return;
            }
        };
        let content = match add_reaction.message(&ctx).await {
            Ok(m) => m.content_safe(&ctx.cache),
            Err(e) => {
                log::debug!("reaction_add: couldn't fetch reacted message: {e}");
                return;
            }
        };
        // an embed-only message (a slash-command card, a debug line) has nothing to react
        // to in the bot's own words
        if content.trim().is_empty() {
            return;
        }
        let channel = add_reaction.channel_id;
        let name = display_name(&reactor);
        let emoji = reaction_label(&add_reaction.emoji);
        let text = format!(
            "(tepki {emoji}) \"{}\" mesajına tepki verdi",
            memory::trim(&content, 200)
        );
        {
            let mut state = self.bot.state();
            state
                .name_to_id
                .insert(name.to_lowercase(), reactor.id.get());
            state
                .usernames
                .insert(reactor.id.get(), reactor.name.clone());
            remember(&mut state, &name, &text);
            if let Some(chat) = state.chats.get_mut(&channel) {
                chat.history.push(user(format!("{name}: {text}")));
                if chat.history.len() > CHAT_SIZE {
                    chat.history.drain(..chat.history.len() - CHAT_SIZE);
                }
            }
            channel_note(&mut state, channel, format!("{name}: {text}"));
        }
        self.bot
            .debug_note(&ctx, channel, format!("tepki: {name} → {emoji}"))
            .await;
    }

    // slash commands run from the command::definitions() table (the bot is only managed
    // via slash); the menu/buttons on the mind card lead to detail modals; a modal
    // submission just gets a short acknowledgment (display-only, no input is collected);
    // the thought button follows its own older path
    /// Serenity's `interaction_create` callback — routes slash commands, modal submissions,
    /// and button/select-menu clicks.
    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        // stealth: no commands are registered and no buttons are sent, so anything arriving
        // here is stale (an old button on an earlier message) — left unanswered
        if self.bot.stealth {
            return;
        }
        match interaction {
            Interaction::Command(cmd) => {
                match command::definitions()
                    .iter()
                    .find(|k| k.name == cmd.data.name)
                {
                    Some(k) => (k.run)(&self.bot, &ctx, &cmd).await,
                    None => log::warn!("unknown slash command: {}", cmd.data.name),
                }
            }
            Interaction::Modal(modal_interaction) => {
                let _ = modal_interaction
                    .create_response(
                        &ctx.http,
                        CreateInteractionResponse::Message(
                            CreateInteractionResponseMessage::new()
                                .ephemeral(true)
                                .content("Görüntüleme amaçlı; bir şey kaydetmedim."),
                        ),
                    )
                    .await;
            }
            Interaction::Component(component) => {
                if component.data.custom_id == THOUGHT_BUTTON {
                    self.thought_button(&ctx, component).await;
                    return;
                }
                if component.data.custom_id.starts_with("setting_") {
                    self.setting_button(&ctx, component).await;
                    return;
                }
                // the mind detail layer: buttons open a section modal, the menu opens a person modal
                let to_open = match component.data.custom_id.as_str() {
                    modal::MIND_TOPICS => Some(modal::topics_modal()),
                    modal::MIND_EVENTS => Some(modal::events_modal()),
                    modal::MIND_SUMMARY => {
                        let state = self.bot.state();
                        Some(modal::summary_modal(&state))
                    }
                    modal::MIND_PERSON_PICK => {
                        let ComponentInteractionDataKind::StringSelect { values } =
                            &component.data.kind
                        else {
                            return;
                        };
                        let Some(id) = values.first().and_then(|v| v.parse::<u64>().ok()) else {
                            return;
                        };
                        Some(modal::person_modal(id))
                    }
                    _ => None,
                };
                if let Some(m) = to_open {
                    if let Err(e) = component
                        .create_response(&ctx.http, CreateInteractionResponse::Modal(m))
                        .await
                    {
                        log::warn!("couldn't send detail modal: {e}");
                    }
                }
            }
            _ => {}
        }
    }
}

// ReactionType's own Display gives Discord's raw mention form for a custom emoji
// (`<:name:id>`), which the model would just repeat back verbatim — not what a person means
// when they say what emoji someone used
/// Custom emoji names are wrapped in colons; missing names become "bilinmeyen emoji".
fn reaction_label(emoji: &ReactionType) -> String {
    match emoji {
        ReactionType::Unicode(s) => s.clone(),
        ReactionType::Custom { name: Some(n), .. } => format!(":{n}:"),
        _ => "bilinmeyen emoji".to_string(),
    }
}
