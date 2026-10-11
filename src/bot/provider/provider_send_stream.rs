impl Bot {
    /// Drains a `StreamReader` to completion, live-editing Discord message(s) as text/thought
    /// arrive, then finalizes into sent messages, a reaction, and history/channel-note entries.
    /// A stream read failure propagates only if no usable content was produced.
    async fn send_stream(
        &self,
        ctx: &Context,
        channel: ChannelId,
        mut reader: StreamReader,
        context: StreamContext<'_>,
    ) -> Result<StreamResult, BotError> {
        let mut text = String::new();
        let mut thought = String::new();
        let mut sent: Vec<Message> = Vec::new();
        let mut last_write = Instant::now();
        let mut first = true;
        let mut stream_error: Option<BotError> = None;
        // the mode stays fixed for the whole reply; if it changes mid-stream, that only
        // takes effect on the next reply
        // stealth: a visible thought (spoiler, counter, button) is a bot tell, so show/hide
        // fall back to silent — the model still reasons, nothing of it reaches the channel
        let mode = match self.state().thinking_mode {
            ThinkingMode::Show | ThinkingMode::Hide if self.stealth => ThinkingMode::Silent,
            m => m,
        };
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
                    if !self.stealth && (first || last_write.elapsed() >= STREAM_EDIT_INTERVAL) {
                        // strip_name returns a slice: the whole text isn't cloned on every edit
                        let layout =
                            stream_view(mode, &thought, strip_name(&text, context.bot_name), false);
                        // "first" is only spent once something real gets written: the
                        // earliest deltas are half a line, so the layout can come back
                        // empty — the message should open with the first meaningful
                        // content, not wait out the 1-2s edit interval first
                        if !layout.is_empty() {
                            first = false;
                            write_stream(ctx, channel, &mut sent, &layout, context.reply_to).await;
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
        self.add_metric(reader.category, reader.usage);
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
        let mut reply = parse_reply(strip_name(&text, context.bot_name));
        // the model chose to stay silent: any messages already sent get deleted, and
        // nothing is added to history. If "tepki: 💀" and "-" arrive together, that's not
        // silence — the emoji still has to land
        if reply.silent && reply.lines.is_empty() && reply.reaction.is_none() {
            log::debug!("stream [{channel}]: silent");
            delete_messages(ctx, sent).await;
            return match stream_error {
                Some(e) => Err(e),
                None => Ok(StreamResult::Silent),
            };
        }
        if reply.is_empty() {
            delete_messages(ctx, sent).await;
            return match stream_error {
                Some(e) => Err(e),
                None => Ok(StreamResult::Empty),
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
                .generate(context.history, &follow_up, context.budget, "sohbet")
                .await
            {
                Ok(y) => parse_reply(&y),
                Err(e) => {
                    log::debug!("stream [{channel}]: regeneration after repeat failed: {e}");
                    Reply::default()
                }
            };
            // a reaction alone doesn't count as empty: an emoji is still something to send
            if (retry_reply.lines.is_empty() && retry_reply.reaction.is_none())
                || retry_reply.lines.iter().any(|s| self.is_repeat(channel, s))
            {
                delete_messages(ctx, sent).await;
                return Ok(StreamResult::Empty);
            }
            reply = retry_reply;
        }
        let layout = stream_layout(mode, &thought, &reply.lines);
        write_stream(ctx, channel, &mut sent, &layout, context.reply_to).await;

        // emoji reaction: instead of or alongside a line of text, lands on the message
        // being replied to. An error doesn't stop the flow — the reaction is a garnish
        if let (Some(emoji), Some(target)) = (&reply.reaction, context.reaction_target) {
            if let Err(e) = ctx
                .http
                .create_reaction(channel, target, &ReactionType::Unicode(emoji.clone()))
                .await
            {
                log::warn!("couldn't add reaction ({channel}): {e}");
            }
        }

        // in hide mode the thought doesn't appear in the message; a button is attached to
        // the end of the reply, opened as an ephemeral code block on click (interaction_create handles it)
        if mode == ThinkingMode::Hide {
            let collapsed_thought = single_line(&thought);
            if !collapsed_thought.is_empty() {
                if let Some(last) = sent.last_mut() {
                    self.state().link_thought(last.id, collapsed_thought);
                    let button = CreateButton::new(THOUGHT_BUTTON)
                        .label("Düşünce Sürecini Göster")
                        .style(ButtonStyle::Secondary);
                    if let Err(e) = last
                        .edit(
                            &ctx.http,
                            EditMessage::new()
                                .components(vec![CreateActionRow::Buttons(vec![button])]),
                        )
                        .await
                    {
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
