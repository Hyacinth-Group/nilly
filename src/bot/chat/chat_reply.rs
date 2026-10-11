impl Bot {
    // generates the next reply in an open chat; only one reply is generated per channel at
    // a time — messages that arrive meanwhile are appended to history and seen on the next reply
    /// The full reply turn: picks an instruction, evaluates mood/research/target/question-cap,
    /// streams a reply (falling back to non-streaming on an empty stream), records it, and
    /// loops again if a new message arrived meanwhile. See `docs/flows.md`'s `reply` diagram
    /// for the full step-by-step.
    async fn reply(&self, ctx: &Context, channel: &RoomKey) {
        loop {
            let instruction = {
                let mut state = self.state();
                if state.busy.contains(channel) {
                    return;
                }
                let Some(chat) = state.chats.get(channel) else {
                    return;
                };
                let instruction = if chat.hacked > 1 {
                    prompts::current().hack_continue
                } else if chat.hacked == 1 {
                    prompts::current().hack_exit
                } else {
                    ""
                };
                state.busy.insert(channel.clone());
                (instruction, state.debug)
            };
            let (instruction, debug) = instruction;
            // debug trace: this turn's decisions, posted to the channel as a single line at the end
            let mut trace: Vec<String> = Vec::new();
            // RAII: releases the flag on every exit from the function, panics included
            let _busy_guard = BusyGuard {
                state: &self.state,
                channel: channel.clone(),
            };

            // leave a short reading window so messages typed back-to-back land in one context
            sleep(Duration::from_millis(150 + (rand::random::<u64>() % 200))).await;
            let (
                history,
                last_message,
                last_was_tagged,
                incoming,
                last_text,
                waiting,
                counter,
                old_mood,
            ) = {
                let state = self.state();
                let Some(chat) = state.chats.get(channel) else {
                    return;
                };
                let last_text = chat
                    .history
                    .iter()
                    .rev()
                    .find(|m| m.role == "user")
                    .map(|m| {
                        m.content
                            .split_once(": ")
                            .map(|(_, t)| t)
                            .unwrap_or(&m.content)
                    })
                    .unwrap_or("")
                    .to_string();
                (
                    chat.history.clone(),
                    chat.last_message.clone(),
                    chat.last_was_tagged,
                    chat.incoming,
                    last_text,
                    chat.recent_arrivals.clone(),
                    chat.counter,
                    chat.mood.clone(),
                )
            };
            log::debug!(
                "reply [{channel}]: turn start, history {} line(s), incoming={incoming}",
                history.len()
            );
            // reply-to only when a mention/name was involved, or more than one message came
            // in between; otherwise a plain message (a real person doesn't "reply to" every
            // single message either)
            let mut reply_to = if last_was_tagged || waiting.len() > 1 {
                last_message.as_ref()
            } else {
                None
            };
            // the mood isn't refreshed on every message, only every few turns (it's cheap,
            // but still a call — don't burn one on every reply)
            let mood = if counter % 4 == 0 {
                let new_mood = self.determine_mood(&history).await.unwrap_or_default();
                if let Some(chat) = self.state().chats.get_mut(channel) {
                    chat.mood = new_mood.clone();
                }
                if debug && !new_mood.is_empty() {
                    trace.push(format!("mood: {new_mood}"));
                }
                new_mood
            } else {
                old_mood
            };
            // if asked to, look something up online (news, research, a link) and add
            // whatever it finds to the task
            let mut instruction = instruction.to_string();
            if !mood.is_empty() {
                instruction = format!(
                    "{instruction}\n\nŞU ANKİ RUH HALİN: {mood} — bunu ilan etme, üslubuna ve kelime seçimine yedir."
                );
            }
            if let Some(findings) = self.research(&last_text).await {
                instruction = format!(
                    "{instruction}\n\nİNTERNETTEN ŞİMDİ ÇEKTİKLERİN (istendiği için baktın; kendi ağzınla anlat, liste yapma, \"kaynak\" deme):\n{findings}"
                );
            }
            // if several different people wrote in a row, the model picks the target; the
            // reply gets tied to that person's message
            let speakers: std::collections::HashSet<&str> =
                waiting.iter().map(|(i, _)| i.as_str()).collect();
            if speakers.len() >= 2 {
                let names: Vec<String> = waiting.iter().map(|(i, _)| i.clone()).collect();
                if let Some(target) = self.pick_target(&names).await {
                    if let Some((_, id)) = waiting
                        .iter()
                        .rev()
                        .find(|(i, _)| i.eq_ignore_ascii_case(&target))
                    {
                        reply_to = Some(id);
                        instruction = format!(
                            "{instruction}\n\nBirden çok kişi yazdı; sen {target} adlı kişiye dönmeyi seçtin. Cevabın doğrudan ona seslensin."
                        );
                        log::debug!("reply [{channel}]: target picked: {target}");
                        if debug {
                            trace.push(format!("target: {target}"));
                        }
                    }
                }
            }
            // don't ask questions back-to-back: code measures the cap, the model does the writing
            if too_many_questions(&self.state(), channel) {
                instruction = format!("{instruction}\n\nBu sefer soru sorma; düz laf et ya da sus.");
                log::debug!("reply [{channel}]: question cap reached");
                if debug {
                    trace.push("question cap: no question this turn".to_string());
                }
            }
            // show the typing indicator while the model call is in flight; the stream message opens on the first delta
            match platform::discord::native_room(channel) {
                Ok(native) => { let _ = native.broadcast_typing(&ctx.http).await; }
                Err(e) => { log::error!("typing [{channel}]: {e}"); return; }
            }
            let budget = reply_budget!();
            let (reader, bot_name) = match self.generate_stream(&history, &instruction, budget, "sohbet").await
            {
                Ok(x) => x,
                Err(e) => {
                    log::error!("ai [generate_stream] [{channel}]: {e}");
                    return;
                }
            };
            let reply = match self
                .send_stream(
                    ctx,
                    channel,
                    reader,
                    StreamContext {
                        bot_name: &bot_name,
                        reply_to,
                        reaction_target: last_message.as_ref(),
                        history: &history,
                        instruction: &instruction,
                        budget,
                    },
                )
                .await
            {
                Ok(StreamResult::Sent(c)) => c,
                Ok(StreamResult::Silent) => {
                    // the model chose to stay silent: nothing is written to history, the
                    // counter, or activity. If a new message arrived meanwhile, one more turn runs anyway
                    log::debug!("reply [{channel}]: silent");
                    if debug {
                        trace.push("silent (-)".to_string());
                        self.debug_trace(ctx, channel, debug, &trace).await;
                    }
                    // the hacked prank keeps advancing even on silence: otherwise the
                    // HACK_CONTINUE instruction would get stuck forever
                    if let Some(chat) = self.state().chats.get_mut(channel) {
                        chat.hacked = chat.hacked.saturating_sub(1);
                    }
                    if new_message_arrived(&self.state(), channel, incoming) {
                        drop(_busy_guard);
                        continue;
                    }
                    return;
                }
                Ok(StreamResult::Empty) => {
                    // nothing usable came out of the stream; handle a new message if one arrived
                    if debug {
                        trace.push("stream empty → fallback generate".to_string());
                    }
                    if new_message_arrived(&self.state(), channel, incoming) {
                        // the flag is released by hand: the next turn re-inserts it at the top
                        drop(_busy_guard);
                        continue;
                    }
                    match self.generate(&history, &instruction, budget, "sohbet").await {
                        Ok(c) => {
                            // de-duplication here is also LINE-based: in the channel
                            // history the bot's lines sit one by one, so a multi-line raw
                            // blob would never match any of them
                            let mut fallback = parse_reply(strip_name(&c, &bot_name));
                            fallback.lines.retain(|s| !self.is_repeat(channel, s));
                            match self
                                .send_reply(ctx, channel, fallback, reply_to, last_message.as_ref(), None)
                                .await
                            {
                                Some(p) => p,
                                None => return,
                            }
                        }
                        Err(e) => {
                            log::error!("ai [generate fallback] [{channel}]: {e}");
                            return;
                        }
                    }
                }
                Err(e) => {
                    log::error!("ai [send_stream] [{channel}]: {e}");
                    return;
                }
            };

            if debug {
                // count what actually went out from the protocol text: the reaction line and the visible lines
                let reaction = reply
                    .lines()
                    .find_map(|l| reaction_body(l.trim()).map(|g| g.trim().to_string()));
                let line_count = reply
                    .lines()
                    .filter(|l| !l.trim().is_empty() && reaction_body(l.trim()).is_none())
                    .count();
                let mut summary = format!("{line_count} line(s) sent");
                if let Some(t) = reaction {
                    summary = format!("{summary} · reaction {t}");
                }
                trace.push(summary);
                self.debug_trace(ctx, channel, debug, &trace).await;
            }
            {
                let mut state = self.state();
                if let Some(chat) = state.chats.get_mut(channel) {
                    chat.history.push(assistant(reply));
                    chat.counter += 1;
                    chat.hacked = chat.hacked.saturating_sub(1);
                    // the reply went out: target selection starts over from scratch
                    chat.recent_arrivals.clear();
                }
                state.last_activity.insert(channel.clone(), Instant::now());
            }

            // no explicit chat close: staying quiet gets it closed by the timeout (in the
            // sleep tick). If a new message arrived while writing this reply, one more
            // turn; otherwise exit.
            let pending = new_message_arrived(&self.state(), channel, incoming);
            if !pending {
                break;
            }
        }
    }
}

// ---------- research ----------
