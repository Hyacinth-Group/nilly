impl Bot {
    /// Drains a `StreamReader` to completion, live-editing Discord message(s) as text/thought
    /// arrive, then finalizes into sent messages, a reaction, and history/channel-note entries.
    /// A stream read failure propagates only if no usable content was produced.
    async fn send_stream(
        &self,
        channel: &ConversationKey,
        mut reader: StreamReader,
        context: StreamContext<'_>,
    ) -> Result<StreamResult, BotError> {
        let (transport, room) = self.transport(channel)?;

        let capabilities = transport.capabilities(&room);
        let mut text = String::new();
        let mut thought = String::new();
        let mut sent: Vec<platform::outgoing::MessageReceipt> = Vec::new();
        let mut last_write = Instant::now();
        let mut first = true;
        let mut stream_error: Option<BotError> = None;
        // the mode stays fixed for the whole reply; if it changes mid-stream, that only
        // takes effect on the next reply
        // stealth: a visible thought (spoiler, counter, button) is a bot tell, so show/hide
        // fall back to silent — the model still reasons, nothing of it reaches the channel
        let (mode, live_edits) = stream_presentation(self.state().thinking_mode, self.stealth);
        let live_edits = live_edits && capabilities.edit && capabilities.delete;
        let presentation = thought_presentation(mode);
        let start = Instant::now();
        let mut chunk_count: u32 = 0;
        let mut first_chunk_ms: Option<u128> = None;

        loop {
            match reader.next().await {
                Ok(Some(p)) => {
                    chunk_count += 1;
                    if first_chunk_ms.is_none() {
                        first_chunk_ms = Some(start.elapsed().as_millis());
                    }
                    text.push_str(&p.text);
                    if matches!(mode, ThinkingMode::Show | ThinkingMode::Hide) {
                        thought.push_str(&p.thought);
                    }
                    // stealth: people don't type by live-editing a message, so nothing goes
                    // out until the stream is done (the final layout below sends it whole)
                    if live_edits && (first || last_write.elapsed() >= capabilities.edit_cadence) {
                        // strip_name returns a slice: the whole text isn't cloned on every edit
                        let layout = transport.render_stream(
                            &room, presentation, &thought, strip_name(&text, context.bot_name), false,
                        );
                        // "first" is only spent once something real gets written: the
                        // earliest deltas are half a line, so the layout can come back
                        // empty — the message should open with the first meaningful
                        // content, not wait out the 1-2s edit interval first
                        if !layout.is_empty() {
                            first = false;
                            write_stream(&transport, &room, &mut sent, &layout, context.reply_to).await;
                            last_write = Instant::now();
                        }
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    stream_error = Some(e);
                    break;
                }
            }
        }

        // usage metrics and a stream summary
        self.add_metric(
            reader.category,
            reader.usage.reported,
            MetricObservation::Stream { completed: stream_error.is_none() },
        );
        log::debug!(
            "stream [{channel}]: chunks={chunk_count} first={first_chunk_ms:?}ms total={}ms done={}",
            start.elapsed().as_millis(),
            reader.done,
        );
        if !reader.done && stream_error.is_none() {
            log::warn!("stream [{channel}]: closed before [DONE] arrived, may be incomplete");
        }

        // the stream is treated as complete even if a new message arrived meanwhile: the
        // reply goes out as captured, and the new message gets handled on the next turn
        // (no starting over)
        let mut reply = parse_reply(strip_name(&text, context.bot_name), capabilities.output);
        // the model chose to stay silent: any messages already sent get deleted, and
        // nothing is added to history. If "tepki: 💀" and "-" arrive together, that's not
        // silence — the emoji still has to land
        if let Some(outcome) = stream_early_result(&reply) {
            if matches!(outcome, StreamResult::Silent) {
                log::debug!("stream [{channel}]: silent");
            }
            delete_messages(&transport, sent).await;
            return match stream_error {
                Some(e) => Err(e),
                None => Ok(outcome),
            };
        }
        // don't say the same thing twice: repeated lines are dropped; if nothing is left
        // and there's no reaction either, it regenerates once — silent again if that repeats too
        let repeats: Vec<String> = reply
            .lines
            .iter()
            .filter(|s| self.is_repeat(channel, s))
            .cloned()
            .collect();
        if !repeats.is_empty() {
            reply.lines.retain(|s| !repeats.contains(s));
            log::debug!("stream [{channel}]: dropped {} repeated line(s)", repeats.len());
        }
        if reply.lines.is_empty() && reply.reaction.is_none() {
            let follow_up = format!(
                "{}\n\nAz önce aynen şunu yazdın: \"{}\". Aynısını ya da benzerini yazma; başka bir açıdan gir ya da konuyu değiştir.",
                context.instruction,
                repeats.join(" / ")
            );
            let retry_reply = match self
                .generate_for(channel, context.history, &follow_up, context.budget, "sohbet")
                .await
            {
                Ok(y) => parse_reply(&y, capabilities.output),
                Err(e) => {
                    log::debug!("stream [{channel}]: regeneration after repeat failed: {e}");
                    Reply::default()
                }
            };
            // a reaction alone doesn't count as empty: an emoji is still something to send
            if (retry_reply.lines.is_empty() && retry_reply.reaction.is_none())
                || retry_reply.lines.iter().any(|s| self.is_repeat(channel, s))
            {
                delete_messages(&transport, sent).await;
                return Ok(StreamResult::Empty);
            }
            reply = retry_reply;
        }
        let layout = transport.render_final(&room, presentation, &thought, &reply.lines);
        write_stream(&transport, &room, &mut sent, &layout, context.reply_to).await;

        // Native reactions remain best-effort; unsupported reactions become one plain emoji.
        if let (Some(emoji), Some(target)) = (&reply.reaction, context.reaction_target) {
            if capabilities.react {
                let target = platform::types::MessageRef {
                    id: target.clone(), room: channel.room.clone(), author: None,
                };
                if let Err(e) = transport.react(&target, &platform::types::Emoji::Unicode(emoji.clone())).await {
                    log::warn!("couldn't add reaction ({channel}): {e}");
                    if matches!(e, platform::PlatformError::Unsupported(_)) {
                        let body = platform::outgoing::OutgoingMessage::text(emoji);
                        if let Err(error) = transport.send(&room, &body).await {
                            log::warn!("couldn't send reaction fallback ({channel}): {error}");
                        }
                    }
                }
            } else {
                let body = platform::outgoing::OutgoingMessage::text(emoji);
                if let Err(e) = transport.send(&room, &body).await {
                    log::warn!("couldn't send reaction fallback ({channel}): {e}");
                }
            }
        }

        // in hide mode the thought doesn't appear in the message; a button is attached to
        // the end of the reply, opened as an ephemeral code block on click (interaction_create handles it)
        if mode == ThinkingMode::Hide && capabilities.components && capabilities.edit {
            let collapsed_thought = single_line(&thought);
            if !collapsed_thought.is_empty() {
                if let Some(last) = sent.last() {
                    self.state().link_thought(last.message.id.clone(), collapsed_thought);
                    let mut body = platform::outgoing::OutgoingMessage::text("");
                    body.presentation = Some(platform::outgoing::PresentationPayload::ThoughtReveal);
                    if let Err(e) = transport.edit(last, &body).await {
                        log::warn!("couldn't add thought button ({channel}): {e}");
                    }
                }
            }
        }

        // what was sent goes into history; the thinking never does — the coach and critic
        // only ever see the reply. Since each line is its own message, each also becomes
        // its own history entry.
        let mut s = self.state();
        for line in &reply.lines {
            s.own_messages.push_back(line.clone());
        }
        while s.own_messages.len() > 50 {
            s.own_messages.pop_front();
        }
        let mut notes: Vec<String> = reply
            .lines
            .iter()
            .map(|s| format!("{}: {s}", context.bot_name))
            .collect();
        if let Some(emoji) = &reply.reaction {
            // seed consistency: the model should see its own protocol format in its own history
            notes.push(format!("{}: tepki: {emoji}", context.bot_name));
        }
        channel_notes(&mut s, channel, notes);
        drop(s);
        if let Some(e) = stream_error {
            log::warn!("stream [{channel}]: cut off mid-way, sent what we had: {e}");
        }
        Ok(StreamResult::Sent(reply.protocol_text()))
    }

    // shared sender for the non-streaming paths: splits the reply into lines per the
    // protocol, sends each line as its own message in order (with a small typing delay
    // between them so they don't all land at once). On silence or an empty reply, nothing
    // is sent and it returns None. The ping only attaches to the first line (the welcome
    // message tags the new member).
}

fn stream_presentation(mode: ThinkingMode, stealth: bool) -> (ThinkingMode, bool) {
    let mode = match mode {
        ThinkingMode::Show | ThinkingMode::Hide if stealth => ThinkingMode::Silent,
        mode => mode,
    };
    (mode, !stealth)
}

// The sender deletes provisional messages before returning either of these outcomes.
fn stream_early_result(reply: &Reply) -> Option<StreamResult> {
    if reply.silent && reply.lines.is_empty() && reply.reaction.is_none() {
        Some(StreamResult::Silent)
    } else if reply.is_empty() {
        Some(StreamResult::Empty)
    } else {
        None
    }
}

fn thought_presentation(mode: ThinkingMode) -> platform::outgoing::ThoughtPresentation {
    use platform::outgoing::ThoughtPresentation;
    match mode {
        ThinkingMode::Show => ThoughtPresentation::Show,
        ThinkingMode::Hide => ThoughtPresentation::Hide,
        ThinkingMode::Silent => ThoughtPresentation::Silent,
        ThinkingMode::Off => ThoughtPresentation::Off,
    }
}
