/// Owns unanswered wake mentions across awaits and restores them on cancellation.
struct PendingWakeMentions<'a> {
    state: &'a Mutex<State>,
    mentions: Vec<(ConversationKey, String, UserRef)>,
}

impl Drop for PendingWakeMentions<'_> {
    fn drop(&mut self) {
        if self.mentions.is_empty() {
            return;
        }
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        // Keep original observations before anything queued while delivery was in flight.
        self.mentions.append(&mut state.pending_mentions);
        state.pending_mentions = std::mem::take(&mut self.mentions);
    }
}

impl Bot {
    // handles the asleep/awake transition; on waking, replies to mentions received while asleep
    /// Returns (state changed, public reply delivered); errors preserve pending mentions.
    async fn sleep_transition(&self) -> Result<(bool, bool), BotError> {
        let (pending, just_woke, changed) = {
            let mut state = self.state();
            let awake = sleep::is_awake(&state);
            if awake == state.asleep {
                log::info!("sleep: {}", if awake { "woke up" } else { "fell asleep" });
            }
            let just_woke = awake && state.asleep;
            let just_slept = !awake && !state.asleep;
            state.asleep = !awake;
            if just_slept {
                // starting markers so overnight messages can be sliced out later
                state.sleep_start = now_unix();
                state.sleep_start_memory_len = state.recent_messages.len();
            }
            if just_woke {
                (std::mem::take(&mut state.pending_mentions), true, true)
            } else {
                (Vec::new(), false, just_slept)
            }
        };

        // nothing to do this tick if there was no transition
        if !just_woke {
            return Ok((changed, false));
        }
        let mut pending = PendingWakeMentions { state: &self.state, mentions: pending };

        if let Some((channel, _, _)) = pending.mentions.last() {
            let history = pending_mention_turns(&pending.mentions);
            let context = self.prompt_context(channel);
            let instruction = context.render(prompts::current().woke_up);
            let result: Result<bool, BotError> = async {
                self.authorize_automatic_delivery(channel)?;
                let reply = self.generate_for(channel, &history, &instruction, Some(200), "uyandim").await?;
                let posted = self.send_lines(channel, &reply, None, None, None, true).await?;
                if let Some(posted) = posted {
                    start_chat(&mut self.state(), channel, Some(posted));
                    Ok(true)
                } else {
                    Ok(false)
                }
            }.await;
            if matches!(&result, Ok(true)) {
                pending.mentions.clear();
            }
            return result.map(|delivered| (true, delivered));
        }

        // with no mentions, evaluate what was written overnight: if something caught its interest, reply with a morning line
        let night: Vec<String> = {
            let state = self.state();
            state
                .recent_messages
                .iter()
                .skip(state.sleep_start_memory_len)
                .cloned()
                .collect()
        };
        let delivered = if night.is_empty() { false } else { self.evaluate_waking(&night).await? };
        Ok((true, delivered))
    }

    // picks out what actually concerns the bot from what was written overnight; replies with a morning line if it clears the threshold
    async fn evaluate_waking(&self, night: &[String]) -> Result<bool, BotError> {
        let night_text = night.join("\n");
        let (name, last_channel) = {
            let state = self.state();
            (state.bot_name.clone(), state.last_channel.clone())
        };
        let context = last_channel.as_ref().map(|channel| self.prompt_context(channel))
            .unwrap_or_else(|| self.default_prompt_context());
        let instruction = context.render_with(prompts::current().waking, &[("{ad}", &name)]);
        #[derive(Deserialize)]
        struct WakingResult {
            #[serde(default)]
            ilgi: i32,
            #[serde(default)]
            konu: String,
        }
        let raw = self.analyze_with(&night_text, &instruction, 100, "uyanis", &context).await?;
        let parsed = serde_json::from_str::<WakingResult>(extract_json(&raw))?;
        log::debug!("waking: interest={} topic={}", parsed.ilgi, parsed.konu);
        if parsed.ilgi < 5 {
            return Ok(false);
        }
        let Some(channel) = self.state().last_channel.clone() else {
            return Err(platform::PlatformError::Disconnected.into());
        };
        if self.state().chats.contains_key(&channel) {
            return Ok(false);
        }
        self.authorize_automatic_delivery(&channel)?;
        let name = self.state().bot_name.clone();
        let context = self.prompt_context(&channel);
        let instruction = context.render_with(prompts::current().waking_reply,
            &[("{ad}", &name), ("{konu}", &parsed.konu)]);
        let reply = self.generate_for(
            &channel,
            &[user(format!("sen uyurken yazılanlar:\n{night_text}"))],
            &instruction,
            Some(250),
            "uyanis_cevap",
        ).await?;
        let Some(posted) = self.send_lines(&channel, &reply, None, None, None, true).await? else {
            return Ok(false);
        };
        start_chat(&mut self.state(), &channel, Some(posted));
        Ok(true)
    }
}

fn pending_mention_turns(pending: &[(ConversationKey, String, UserRef)]) -> Vec<ChatMessage> {
    let mut turns = Vec::with_capacity(pending.len() + 1);
    turns.push(user("uyurken sana yazılanlar:"));
    for (_, text, person) in pending {
        let mut turn = user(text.clone());
        turn.observed_user = Some(person.clone());
        turns.push(turn);
    }
    turns
}

#[cfg(test)]
mod generic_sleep_provenance_tests {
    use super::*;

    #[test]
    fn pending_mentions_capture_distinct_speakers_without_display_name_identity_inference() {
        let route = RoomRef::from_key(RoomKey::discord(7)).conversation();
        let person = |id| UserRef {
            id: UserKey::discord(id),
            display_name: "Alice".into(),
            username: None,
            automation: platform::types::Automation::Human,
            stability: platform::types::IdentityStability::Persistent,
        };
        let pending = vec![
            (route.clone(), "Alice: first".into(), person(1)),
            (route, "Alice [discord:user:2]: second".into(), person(2)),
        ];
        let turns = pending_mention_turns(&pending);
        let captured = observed_users(&turns);
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[1].content, "Alice: first");
        assert_eq!(turns[2].content, "Alice [discord:user:2]: second");
        assert_eq!(captured["alice"].id, UserKey::discord(1));
        assert_eq!(captured["alice [discord:user:2]"].id, UserKey::discord(2));
    }
    #[tokio::test]
    async fn cancelling_a_wake_future_restores_mentions_without_holding_state_across_await() {
        let state = Mutex::new(State::default());
        let route = RoomRef::from_key(RoomKey::discord(17)).conversation();
        let person = UserRef {
            id: UserKey::discord(18),
            display_name: "Alice".into(),
            username: None,
            automation: platform::types::Automation::Human,
            stability: platform::types::IdentityStability::Persistent,
        };
        state.lock().unwrap_or_else(|error| error.into_inner())
            .pending_mentions.push((route.clone(), "first".into(), person.clone()));
        let work = async {
            let mentions = std::mem::take(&mut state.lock().unwrap_or_else(|error| error.into_inner()).pending_mentions);
            let mut pending = PendingWakeMentions { state: &state, mentions };
            std::future::pending::<()>().await;
            pending.mentions.clear();
        };
        let mut future = Box::pin(work);
        assert!(futures_util::poll!(future.as_mut()).is_pending());
        {
            let mut available = state.try_lock().expect("wake future must not hold state across await");
            assert!(available.pending_mentions.is_empty());
            available.pending_mentions.push((route, "second".into(), person));
        }
        drop(future);
        let restored = state.lock().unwrap_or_else(|error| error.into_inner());
        assert_eq!(restored.pending_mentions.len(), 2);
        assert_eq!(restored.pending_mentions[0].1, "first");
        assert_eq!(restored.pending_mentions[1].1, "second");
    }

    #[test]
    fn delivered_wake_mentions_are_disarmed_instead_of_requeued() {
        let state = Mutex::new(State::default());
        let person = UserRef {
            id: UserKey::discord(19),
            display_name: "Alice".into(),
            username: None,
            automation: platform::types::Automation::Human,
            stability: platform::types::IdentityStability::Persistent,
        };
        let mut pending = PendingWakeMentions {
            state: &state,
            mentions: vec![(RoomRef::from_key(RoomKey::discord(20)).conversation(), "answered".into(), person)],
        };
        pending.mentions.clear();
        drop(pending);
        assert!(state.lock().unwrap_or_else(|error| error.into_inner()).pending_mentions.is_empty());
    }
}
