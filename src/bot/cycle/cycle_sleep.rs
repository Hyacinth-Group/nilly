impl Bot {
    // handles the asleep/awake transition; on waking, replies to mentions received while asleep
    /// Input: `&self`; `ctx: &Context`. Output: none (no-op if there was no asleep↔awake
    /// transition this tick). Uses: `sleep::is_awake`, `self.state()`, `self.generate` (for
    /// `WOKE_UP`), `self.send_lines`, `start_chat`, `self.evaluate_waking`. Used by:
    /// `sleep_cycle` (`cycle_background.rs`), `Bot::cmd_wake`/`cmd_sleep`
    /// (`command/actions.rs`), `Handler::setting_button` (`handler_buttons.rs`).
    async fn sleep_transition(&self, ctx: &Context) {
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
            let Some((channel, _)) = pending.last() else {
                return;
            };
            let list = pending
                .iter()
                .map(|(_, m)| format!("- {m}"))
                .collect::<Vec<_>>()
                .join("\n");
            match self
                .generate(
                    &[user(format!("uyurken sana yazılanlar:\n{list}"))],
                    prompts::current().woke_up,
                    Some(200),
                    "uyandim",
                )
                .await
            {
                Ok(c) => match self.send_lines(ctx, channel, &c, None, None, None).await {
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
            self.evaluate_waking(ctx, &night).await;
        }
    }

    // picks out what actually concerns the bot from what was written overnight; replies with a morning line if it clears the threshold
    /// Input: `&self`; `ctx: &Context`; `night: &[String]` — lines written while asleep.
    /// Output: none. Uses: `WAKING`/`WAKING_REPLY` (`prompts.rs`), `self.analyze`,
    /// `extract_json`, `self.state()`, `self.generate`, `self.send_lines`, `start_chat`.
    /// Used by: `sleep_transition` above, the only caller.
    async fn evaluate_waking(&self, ctx: &Context, night: &[String]) {
        let night_text = night.join("\n");
        let instruction = {
            let state = self.state();
            prompts::current().waking.replace("{ad}", &state.bot_name)
        };
        #[derive(Deserialize)]
        struct WakingResult {
            #[serde(default)]
            ilgi: i32,
            #[serde(default)]
            konu: String,
        }
        let result = match self.analyze(&night_text, &instruction, 100, "uyanis").await {
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
        let instruction = {
            let state = self.state();
            prompts::current()
                .waking_reply
                .replace("{ad}", &state.bot_name)
                .replace("{konu}", &parsed.konu)
        };
        match self
            .generate(
                &[user(format!("sen uyurken yazılanlar:\n{night_text}"))],
                &instruction,
                Some(250),
                "uyanis_cevap",
            )
            .await
        {
            Ok(c) => match self.send_lines(ctx, &channel, &c, None, None, None).await {
                Some(p) => {
                    start_chat(&mut self.state(), &channel, Some(p));
                }
                None => log::debug!("waking_reply: model stayed silent, skipped"),
            },
            Err(e) => log::error!("ai [waking_reply]: {e}"),
        }
    }
}
