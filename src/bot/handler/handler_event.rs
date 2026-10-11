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
            state.accounts.insert(
                platform::discord::events::connection_id().clone(),
                platform::discord::events::decode_user(&data.user),
            );
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
        let space = SpaceKey::discord(guild.id.get());
        if self.bot.guild_id.as_ref().is_some_and(|g| g != &space) {
            return;
        }
        let first_time = {
            let mut state = self.bot.state();
            let is_new = state.scanned.insert(space);
            if is_new {
                let list: Vec<String> = state.scanned.iter()
                    .filter_map(|key| platform::discord::native_space(key).ok())
                    .map(|guild| guild.get().to_string())
                    .collect();
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
                let Ok(native_channel) = platform::discord::native_room(&channel) else {
                    return;
                };
                if let Err(e) = native_channel.send_message(&ctx.http, msg).await {
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
        if let Some((room, user)) =
            platform::discord::events::decode_member_join(&self.bot, &ctx, &member)
        {
            self.member_joined(&ctx, room, user).await;
        }
    }

    async fn message(&self, ctx: Context, msg: Message) {
        let Some(message) = platform::discord::events::decode_message(
            &msg,
            &ctx.cache,
            self.bot.guild_id.as_ref(),
            self.bot.allowed_channels.as_ref(),
            self.bot.image_analysis,
        ) else {
            return;
        };
        self.message_created(&ctx, platform::discord::events::connection_id(), message)
            .await;
    }

    async fn reaction_add(&self, ctx: Context, add_reaction: Reaction) {
        if let Some(event) =
            platform::discord::events::decode_reaction(&self.bot, &ctx, &add_reaction).await
        {
            self.reaction_added(&ctx, event).await;
        }
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
                        let Some(id) = values.first().and_then(|v| v.parse::<UserKey>().ok()) else {
                            return;
                        };
                        Some(modal::person_modal(&id))
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


impl Handler {
    /// Common welcome decision; Context is only a temporary outgoing transport edge.
    async fn member_joined(&self, ctx: &Context, room: RoomRef, member: UserRef) {
        let channel = &room.id;
        let name = &member.display_name;
        {
            let mut state = self.bot.state();
            if is_owner(&member.id) {
                state.favorite_name = Some(name.clone());
            }
            if state.chats.contains_key(channel) {
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
            .send_lines(ctx, channel, &greeting, None, None, Some(&member.id))
            .await
        {
            Some(p) => {
                start_chat(&mut self.bot.state(), channel, Some(p));
            }
            None => log::debug!("hos_geldin: model stayed silent, skipped"),
        }
    }

    /// Common message decisions consume decoded text and identities, never native cache.
    async fn message_created(
        &self,
        ctx: &Context,
        connection: &ConnectionId,
        msg: IncomingMessage,
    ) {
        let channel = &msg.room.id;
        let name = &msg.author.display_name;
        let text = &msg.text;
        let image = msg.attachments.iter().find_map(|attachment| {
            match (&attachment.kind, &attachment.source) {
                (MediaKind::Image, MediaSource::PublicUrl(url)) => Some(url.as_str()),
                _ => None,
            }
        });

        // phase 1 (locked): recording facts + flag decisions
        let (direct_reply, tagged, evaluate, ongoing_dialog, debug) = {
            let mut state = self.bot.state();
            // was it mentioned, named, or replied to?
            let tagged = message_is_tagged(&msg, &state.bot_name, state.accounts.get(connection));
            state.growth.messages += 1;
            remember(&mut state, name, text);
            state
                .name_to_id
                .insert(name.to_lowercase(), msg.author.id.clone());
            if let Some(username) = &msg.author.username {
                state.usernames.insert(msg.author.id.clone(), username.clone());
            }
            state.last_channel = Some(channel.clone());
            if is_owner(&msg.author.id) {
                state.favorite_name = Some(name.clone());
            }

            // news was posted, a comment was expected, but the window ran out
            if state
                .awaiting_comment
                .get(channel)
                .is_some_and(|t| Instant::now() > *t)
            {
                state.chats.remove(channel);
                state.awaiting_comment.remove(channel);
            }

            // no writing while asleep; a mention gets a reply once it wakes up
            if !sleep::is_awake(&state) {
                if tagged {
                    state
                        .pending_mentions
                        .push((channel.clone(), format!("{name}: {text}")));
                    if state.pending_mentions.len() > 20 {
                        state.pending_mentions.remove(0);
                    }
                }
                return;
            }

            let open = state.chats.contains_key(channel);
            // an open chat alone doesn't mean "reply to everyone": like a real person, it
            // automatically continues with whoever it JUST talked to (the chat's last user
            // message belongs to the same person who sent this one), but if someone else
            // in the channel wrote, or the chat has gone cold, it still runs the
            // willingness evaluation.
            let ongoing_dialog = open
                && state.chats.get(channel).is_some_and(|chat| {
                    chat.history
                        .iter()
                        .rev()
                        .find(|m| m.role == "user")
                        .and_then(|m| m.content.split_once(": ").map(|(speaker, _)| speaker))
                        // eq_ignore_ascii_case misses on Turkish İ/i̇; casefold exists exactly for this
                        .is_some_and(|speaker| casefold(speaker) == casefold(name))
                });
            let evaluate = if !tagged && !ongoing_dialog {
                // rate limit: at most one willingness call every 2 minutes per channel
                let now = Instant::now();
                let allowed = state
                    .last_evaluation
                    .get(channel)
                    .is_none_or(|t| now.duration_since(*t) >= EVALUATION_INTERVAL);
                if allowed {
                    state.last_evaluation.insert(channel.clone(), now);
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
            let mut open = state.chats.contains_key(channel);
            if !open && join {
                start_chat(&mut state, channel, None);
                open = true;
            }
            if let Some(chat) = state.chats.get_mut(channel) {
                chat.participants.insert(name.to_lowercase(), msg.author.id.clone());
                // only the latest image goes to the model: older entries' links are dropped
                for m in chat.history.iter_mut() {
                    m.image = None;
                }
                chat.history.push(match image {
                    Some(url) => user_with_image(format!("{name}: {text}"), url),
                    None => user(format!("{name}: {text}")),
                });
                chat.last_message = Some(msg.id.clone());
                chat.last_was_tagged = tagged;
                chat.incoming += 1;
                chat.recent_arrivals.push_back((name.clone(), msg.id.clone()));
                if chat.recent_arrivals.len() > 20 {
                    chat.recent_arrivals.pop_front();
                }
                if chat.history.len() > CHAT_SIZE {
                    chat.history.drain(..chat.history.len() - CHAT_SIZE);
                }
                state.last_activity.insert(channel.clone(), Instant::now());
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
                .debug_note(ctx, channel, format!("{trace} → {decision}"))
                .await;
        }
        if should_reply {
            self.bot.reply(ctx, channel).await;
        }
    }

    /// Social reactions record context but never trigger a willingness check or reply.
    async fn reaction_added(&self, ctx: &Context, event: ReactionEvent) {
        let channel = &event.room.id;
        let name = &event.actor.display_name;
        let Some(content) = &event.original_text else {
            return;
        };
        let emoji = match &event.emoji {
            Emoji::Unicode(text) => std::borrow::Cow::Borrowed(text.as_str()),
            Emoji::Custom { label: Some(name), .. } => std::borrow::Cow::Owned(format!(":{name}:")),
            Emoji::Custom { label: None, .. } => std::borrow::Cow::Borrowed("bilinmeyen emoji"),
        };
        let text = format!(
            "(tepki {emoji}) \"{}\" mesajına tepki verdi",
            memory::trim(content, 200)
        );
        {
            let mut state = self.bot.state();
            state
                .name_to_id
                .insert(name.to_lowercase(), event.actor.id.clone());
            if let Some(username) = &event.actor.username {
                state.usernames.insert(event.actor.id.clone(), username.clone());
            }
            remember(&mut state, name, &text);
            if let Some(chat) = state.chats.get_mut(channel) {
                chat.participants.insert(name.to_lowercase(), event.actor.id.clone());
                chat.history.push(user(format!("{name}: {text}")));
                if chat.history.len() > CHAT_SIZE {
                    chat.history.drain(..chat.history.len() - CHAT_SIZE);
                }
            }
            channel_note(&mut state, channel, format!("{name}: {text}"));
        }
        self.bot
            .debug_note(ctx, channel, format!("tepki: {name} → {emoji}"))
            .await;
    }
}

/// Addressing is connection-local even when several transports share one personality.
fn message_is_tagged(msg: &IncomingMessage, bot_name: &str, account: Option<&UserRef>) -> bool {
    let addressed = account.is_some_and(|account| {
        msg.mentions.contains(&account.id)
            || msg.reply_to.as_ref().is_some_and(|reply| reply.author.as_ref() == Some(&account.id))
    });
    if addressed {
        return true;
    }
    let account_name = account.map(|account| {
        account.username.as_deref().unwrap_or(&account.display_name)
    }).unwrap_or("");
    if bot_name.is_empty() && account_name.is_empty() {
        return false;
    }
    let text = msg.text.to_lowercase();
    [bot_name, account_name].iter()
        .any(|name| !name.is_empty() && text.contains(&name.to_lowercase()))
}
