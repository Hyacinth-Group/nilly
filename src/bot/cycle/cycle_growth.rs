impl Bot {
    // closes chats that have gone quiet: no goodbye message, no channel ban.
    // a closed chat's transcript goes to the diarist and the critic (moved to the queue in the memory step)
    async fn close_timed_out(&self) {
        let closed: Vec<(ConversationKey, Chat)> = {
            let mut state = self.state();
            // drop activity records for chats that no longer exist
            let open_channels: HashSet<ConversationKey> = state.chats.keys().cloned().collect();
            state.last_activity.retain(|channel, _| open_channels.contains(channel));
            let now = Instant::now();
            let to_close: Vec<ConversationKey> = state
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
            let news_expired: Vec<ConversationKey> = state
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
            let channel_name = self.adapters.lock().unwrap_or_else(|e| e.into_inner())
                .room(&channel)
                .and_then(|room| room.name)
                .unwrap_or_else(|| channel.to_string());
            let observed = observed_users(&chat.history);
            // not handled inline, processed in the memory cycle instead (so the critic runs too)
            let queue_len = {
                let mut state = self.state();
                state.memory_queue.push_back((
                    transcript_text,
                    "biten sohbet".to_string(),
                    channel_name,
                    true,
                    observed,
                ));
                state.memory_queue.len()
            };
            let minutes = CHAT_TIMEOUT.as_secs() / 60;
            log::info!(
                "mind: chat closed [{channel}] ({minutes} min quiet) → queued ({queue_len}), diarist within 10 min"
            );
            self.debug_note(&channel, format!("sohbet kapandı ({minutes} dk sessiz)"))
                .await;
            self.state().growth.chats += 1;
            self.check_growth().await;
        }
    }
}

// ---------- growth ----------

impl Bot {
    // jumps to the earned stage and persists it; picks a name once it reaches the established stage
    async fn check_growth(&self) {
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
            self.pick_name().await;
        }
    }

    // picks its own name, changes its nickname on every server, and announces it to the group
    async fn pick_name(&self) {
        let context = self.default_prompt_context();
        let instruction = context.render(prompts::current().name_pick);
        let reply = match self
            .generate(
                &[user("isim seçme vakti")],
                &instruction,
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
        let targets: Vec<_> = {
            let registry = self.adapters.lock().unwrap_or_else(|e| e.into_inner());
            registry.discovered_spaces().into_iter().filter_map(|(connection, space)| {
                let adapter = registry.get(&connection)?;
                // Legacy Discord changed every cached guild nickname, even when
                // GUILD_ID restricts learning. Other adapters require approved spaces.
                let approved = space.id.platform() == PlatformKind::Discord
                    || space.rooms.iter().any(|room| registry.allows(&connection, room));
                approved.then_some((adapter, space.id))
            }).collect()
        };
        for (adapter, space) in targets {
            if let Err(error) = adapter.set_nickname(
                platform::outgoing::NickTarget::Space(&space), &name,
            ).await {
                log::warn!("name: couldn't change nickname ({space}): {error}");
            }
        }
        {
            let mut state = self.state();
            state.growth.name = Some(name.clone());
            state.bot_name = name.clone();
            growth::save(&state.growth);
        }
        log::info!("growth: new name {name}");

        let Some(channel) = default_channel(self) else {
            return;
        };
        let context = self.prompt_context(&channel);
        let instruction = context.render_with(prompts::current().name_announce, &[("{isim}", &name)]);
        match self
            .generate_for(
                &channel,
                &[user("ismini seçtin")],
                &instruction,
                Some(150),
                "isim_duyuru",
            )
            .await
        {
            Ok(announcement) => match self
                .send_lines(&channel, &announcement, None, None, None, true)
                .await
            {
                Ok(Some(p)) => {
                    start_chat(&mut self.state(), &channel, Some(p));
                }
                Ok(None) => log::debug!("name: model stayed silent for the announcement, skipped"),
                Err(e) => log::error!("name delivery: {e}"),
            },
            Err(e) => log::error!("name: {e}"),
        }
    }
}
