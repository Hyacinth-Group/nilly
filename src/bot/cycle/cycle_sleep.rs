impl Bot {
    // handles the asleep/awake transition; on waking, replies to mentions received while asleep
    /// Does nothing if this tick has no sleep-state transition.
    async fn sleep_transition(&self) {
        let (pending, just_woke) = {
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
                (std::mem::take(&mut state.pending_mentions), true)
            } else {
                (Vec::new(), false)
            }
        };

        // nothing to do this tick if there was no transition
        if !just_woke {
            return;
        }

        if !pending.is_empty() {
            // a mention while asleep always gets a reply: the list is put back on failure
            let Some((channel, _, _)) = pending.last() else {
                return;
            };
            let history = pending_mention_turns(&pending);
            let context = self.prompt_context(channel);
            let instruction = context.render(prompts::current().woke_up);
            match self
                .generate_for(
                    channel,
                    &history,
                    &instruction,
                    Some(200),
                    "uyandim",
                )
                .await
            {
                Ok(c) => match self.send_lines(channel, &c, None, None, None).await {
                    Some(p) => {
                        start_chat(&mut self.state(), channel, Some(p));
                    }
                    None => log::debug!("woke_up: model stayed silent, skipped"),
                },
                Err(e) => {
                    log::error!("ai [woke_up]: {e}");
                    let mut state = self.state();
                    for item in pending {
                        state.pending_mentions.push(item);
                    }
                }
            }
            return;
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
        if !night.is_empty() {
            self.evaluate_waking(&night).await;
        }
    }

    // picks out what actually concerns the bot from what was written overnight; replies with a morning line if it clears the threshold
    async fn evaluate_waking(&self, night: &[String]) {
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
        let result = match self.analyze_with(&night_text, &instruction, 100, "uyanis", &context).await {
            Ok(c) => serde_json::from_str::<WakingResult>(extract_json(&c)),
            Err(e) => {
                log::debug!("waking: evaluation call failed: {e}");
                return;
            }
        };
        let Ok(parsed) = result else {
            log::debug!("waking: couldn't parse the result");
            return;
        };
        log::debug!("waking: interest={} topic={}", parsed.ilgi, parsed.konu);
        if parsed.ilgi < 5 {
            return;
        }
        let Some(channel) = self.state().last_channel.clone() else {
            return;
        };
        if self.state().chats.contains_key(&channel) {
            return;
        }
        let name = self.state().bot_name.clone();
        let context = self.prompt_context(&channel);
        let instruction = context.render_with(prompts::current().waking_reply,
            &[("{ad}", &name), ("{konu}", &parsed.konu)]);
        match self
            .generate_for(
                &channel,
                &[user(format!("sen uyurken yazılanlar:\n{night_text}"))],
                &instruction,
                Some(250),
                "uyanis_cevap",
            )
            .await
        {
            Ok(c) => match self.send_lines(&channel, &c, None, None, None).await {
                Some(p) => {
                    start_chat(&mut self.state(), &channel, Some(p));
                }
                None => log::debug!("waking_reply: model stayed silent, skipped"),
            },
            Err(e) => log::error!("ai [waking_reply]: {e}"),
        }
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
}
