impl Bot {
    // closes chats that have gone quiet: no goodbye message, no channel ban.
    // a closed chat's transcript goes to the diarist and the critic (moved to the queue in the memory step)
    /// Input: `&self`; `ctx: &Context`. Output: none. Uses: `self.state()`, `end_chat`,
    /// `transcript`, `self.debug_note`, `self.check_growth`. Used by: `sleep_cycle`
    /// (`cycle_background.rs`), the only caller (runs on the minute tick).
    async fn close_timed_out(&self, ctx: &Context) {
        let closed: Vec<(RoomKey, Chat)> = {
            let mut state = self.state();
            // drop activity records for chats that no longer exist
            let open_channels: HashSet<RoomKey> = state.chats.keys().cloned().collect();
            state.last_activity.retain(|channel, _| open_channels.contains(channel));
            let now = Instant::now();
            let to_close: Vec<RoomKey> = state
                .last_activity
                .iter()
                .filter(|(channel, t)| {
                    !state.busy.contains(channel) && now.duration_since(**t) >= CHAT_TIMEOUT
                })
                .map(|(channel, _)| channel.clone())
                .collect();
            let mut closed = Vec::new();
            for channel in to_close {
                if let Some(chat) = end_chat(&mut state, &channel) {
                    state.last_activity.remove(&channel);
                    closed.push((channel, chat));
                }
            }
            // news chats past their window: if nobody commented (no activity for the whole
            // window, or the record already dropped), it closes silently and the map doesn't bloat
            let news_expired: Vec<RoomKey> = state
                .awaiting_comment
                .iter()
                .filter(|(channel, t)| {
                    now >= **t
                        && state
                            .last_activity
                            .get(channel)
                            .is_none_or(|a| now.duration_since(*a) >= COMMENT_WINDOW)
                })
                .map(|(channel, _)| channel.clone())
                .collect();
            for channel in news_expired {
                state.awaiting_comment.remove(&channel);
                state.chats.remove(&channel);
                state.last_activity.remove(&channel);
                log::debug!("news [{channel}]: no comment came in, chat closed silently");
            }
            closed
        };
        for (channel, chat) in closed {
            let bot_name = self.state().bot_name.clone();
            let transcript_text = transcript(&chat.history, &bot_name);
            let channel_name = match platform::discord::native_room(&channel) {
                Ok(native) => native.name(ctx).await.unwrap_or_else(|_| channel.to_string()),
                Err(_) => channel.to_string(),
            };
            // not handled inline, processed in the memory cycle instead (so the critic runs too)
            let queue_len = {
                let mut state = self.state();
                state.memory_queue.push_back((
                    transcript_text,
                    "biten sohbet".to_string(),
                    channel_name,
                    true,
                ));
                state.memory_queue.len()
            };
            let minutes = CHAT_TIMEOUT.as_secs() / 60;
            log::info!(
                "mind: chat closed [{channel}] ({minutes} min quiet) → queued ({queue_len}), diarist within 10 min"
            );
            self.debug_note(ctx, &channel, format!("sohbet kapandı ({minutes} dk sessiz)"))
                .await;
            self.state().growth.chats += 1;
            self.check_growth(ctx).await;
        }
    }
}

// ---------- growth ----------

impl Bot {
    // jumps to the earned stage and persists it; picks a name once it reaches the established stage
    /// Input: `&self`; `ctx: &Context`. Output: none. Uses: `growth::earned_stage`/`stage`/
    /// `save`, `self.pick_name`. Used by: `close_timed_out` above, `news_cycle`
    /// (`cycle_news.rs`), `Bot::cmd_agents` (`command/actions.rs`).
    async fn check_growth(&self, ctx: &Context) {
        let needs_name = {
            let mut state = self.state();
            let earned = growth::earned_stage(&state.growth);
            if earned > state.growth.stage {
                state.growth.stage = earned;
                log::info!("growth: advanced to stage {}", growth::stage(&state.growth).name);
            }
            growth::save(&state.growth);
            state.growth.name.is_none() && state.growth.stage >= growth::NAME_STAGE
        };
        if needs_name {
            self.pick_name(ctx).await;
        }
    }

    // picks its own name, changes its nickname on every server, and announces it to the group
    /// Input: `&self`; `ctx: &Context`. Output: none. Uses: `self.generate` (twice: pick,
    /// then announcement), `NAME_PICK`/`NAME_ANNOUNCE` (`prompts.rs`), `growth::clean_name`,
    /// `growth::save`, `default_channel`, `self.send_lines`, `start_chat`. Used by:
    /// `check_growth` above, the only caller.
    async fn pick_name(&self, ctx: &Context) {
        let reply = match self
            .generate(
                &[user("isim seçme vakti")],
                prompts::current().name_pick,
                Some(12),
                "isim_sec",
            )
            .await
        {
            Ok(c) => c,
            Err(e) => return log::error!("name: {e}"),
        };
        let Some(name) = growth::clean_name(&reply) else {
            return log::warn!("name: couldn't parse the choice: {reply}");
        };
        for guild in ctx.cache.guilds() {
            if let Err(e) = guild.edit_nickname(&ctx.http, Some(&name)).await {
                log::warn!("name: couldn't change nickname ({guild}): {e}");
            }
        }
        {
            let mut state = self.state();
            state.growth.name = Some(name.clone());
            state.bot_name = name.clone();
            growth::save(&state.growth);
        }
        log::info!("growth: new name {name}");

        let Some(channel) = default_channel(self, ctx) else {
            return;
        };
        match self
            .generate(
                &[user("ismini seçtin")],
                &prompts::current().name_announce.replace("{isim}", &name),
                Some(150),
                "isim_duyuru",
            )
            .await
        {
            Ok(announcement) => match self
                .send_lines(ctx, &channel, &announcement, None, None, None)
                .await
            {
                Some(p) => {
                    start_chat(&mut self.state(), &channel, Some(p));
                }
                None => log::debug!("name: model stayed silent for the announcement, skipped"),
            },
            Err(e) => log::error!("name: {e}"),
        }
    }
}

// ---------- memory ----------

// reads a channel's last two weeks on joining a server
