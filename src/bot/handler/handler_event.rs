impl Bot {
    /// Common welcome decision consumes only decoded room and member identities.
    async fn member_joined(self: &Arc<Self>, connection: &ConnectionId, room: RoomRef, member: UserRef) {
        if !self.accepts_author(connection, &member) { return; }
        let channel = &room.conversation();
        if !self.member_room_allowed(connection, &room) || !self.adapters.lock().unwrap_or_else(|error| error.into_inner()).is_bound(connection, &room) { return; }
        let name = &member.display_name;
        {
            let mut state = self.state();
            if is_owner(&member.id) {
                state.favorite_name = Some(name.clone());
            }
            if state.chats.contains_key(channel) {
                return;
            }
        }

        let greeting = match self

            .generate_for(channel, 
                &[user(format!("{name} sunucuya yeni katıldı."))],
                &self.prompt_context(channel).render(prompts::current().welcome),
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

            .send_lines(channel, &greeting, None, None, Some(&member.id), true)
            .await
        {
            Ok(Some(p)) => {
                start_chat(&mut self.state(), channel, Some(p));
            }
            Ok(None) => log::debug!("hos_geldin: model stayed silent, skipped"),
            Err(e) => log::error!("hos_geldin delivery: {e}"),
        }
    }

    /// Common decisions consume normalized identities and resolve media only when enabled.
    async fn message_created(
        self: &Arc<Self>,
        connection: &ConnectionId,
        msg: IncomingMessage,
    ) {
        if !self.accepts_author(connection, &msg.author) || msg.room.kind == platform::types::RoomKind::Direct { return; }
        let channel = &msg.room.conversation();
        if !self.accepts_room(connection, &msg.room) { return; }
        let Ok((adapter, _bound_room)) = self.transport(channel) else { return; };
        let selected = self.image_analysis.then(|| msg.attachments.iter().find(|attachment| attachment.kind == MediaKind::Image)).flatten();
        let text = &incoming_text(&msg.text, selected.is_some(), self.image_analysis);
        if text.trim().is_empty() { return; }
        let name = &observe_user(&mut self.state(), &msg.author);

        // phase 1 (locked): recording facts + flag decisions
        let (direct_reply, tagged, evaluate, ongoing_dialog, debug) = {
            let mut state = self.state();
            // was it mentioned, named, or replied to?
            let tagged = message_is_tagged(&msg, &state.bot_name, state.accounts.get(connection));
            state.growth.messages += 1;
            remember(&mut state, name, text);
            // Identity was captured together with this raw observation.
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
                        .push((channel.clone(), format!("{name}: {text}"), msg.author.clone()));
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
                        .and_then(|m| m.observed_user.as_ref())
                        .is_some_and(|speaker| speaker.id == msg.author.id)
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
                let state = self.state();
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
            match self.willingness(channel).await {
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

        // Media resolution is only needed for a model turn, never a sleeping/raw observation.
        let needs_turn = join || self.state().chats.contains_key(channel);
        let image = match selected.filter(|_| needs_turn) {
            Some(attachment) => adapter.resolve_media(attachment).await.ok(),
            None => None,
        };

        // phase 3 (locked): join the chat and process the message
        let should_reply = {
            let mut state = self.state();
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
                let mut turn = match image.as_ref() {
                    Some(platform::outgoing::ModelMedia::PublicUrl(url) | platform::outgoing::ModelMedia::DataUrl(url)) => user_with_image(format!("{name}: {text}"), url),
                    None => user(format!("{name}: {text}")),
                };
                turn.observed_user = Some(msg.author.clone());
                chat.history.push(turn);
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
            channel_observe(&mut state, channel, format!("{name}: {text}"), &msg.author);
            // the willingness result also applies to an already-open chat: if someone
            // else wrote and the score was below the threshold, the message still enters
            // history but gets no reply (otherwise the evaluation would just burn tokens for nothing)
            open && join
        };

        if debug {
            let decision = if should_reply { "reply" } else { "silent" };
            self
                .debug_note(channel, format!("{trace} → {decision}"))
                .await;
        }
        if should_reply {
            self.reply(channel).await;
        }
    }

    /// Social reactions record context but never trigger a willingness check or reply.
    async fn reaction_added(self: &Arc<Self>, connection: &ConnectionId, event: ReactionEvent) {
        if !self.accepts_author(connection, &event.actor) { return; }
        let channel = &event.room.conversation();
        if !self.accepts_room(connection, &event.room) { return; }
        let account = self.state().accounts.get(connection).map(|account| account.id.clone());
        if account.is_none() || event.target.author.as_ref() != account.as_ref() { return; }
        let name = &observe_user(&mut self.state(), &event.actor);
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
            let mut state = self.state();
            // Actor identity is captured before recording the social observation.
            remember(&mut state, name, &text);
            if let Some(chat) = state.chats.get_mut(channel) {
                chat.participants.insert(name.to_lowercase(), event.actor.id.clone());
                let mut turn = user(format!("{name}: {text}"));
                turn.observed_user = Some(event.actor.clone());
                chat.history.push(turn);
                if chat.history.len() > CHAT_SIZE {
                    chat.history.drain(..chat.history.len() - CHAT_SIZE);
                }
            }
            channel_observe(&mut state, channel, format!("{name}: {text}"), &event.actor);
        }
        self
            .debug_note(channel, format!("tepki: {name} → {emoji}"))
            .await;
    }

    fn accepts_room(&self, connection: &ConnectionId, room: &RoomRef) -> bool {
        let registry = self.adapters.lock().unwrap_or_else(|error| error.into_inner());
        registry.allows(connection, room) && registry.is_bound(connection, room)
    }

    fn accepts_author(&self, connection: &ConnectionId, person: &UserRef) -> bool {
        !matches!(person.automation, platform::types::Automation::Bot | platform::types::Automation::Integration)
            && self.state().accounts.get(connection).is_none_or(|account| account.id != person.id)
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
