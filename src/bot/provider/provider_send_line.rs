impl Bot {
    /// Parses raw model text and sends it via the non-streaming, line-by-line protocol.
    /// Input: `&self`; `ctx: &Context`; `channel: &RoomKey`; `raw: &str` — unparsed model
    /// output; `reply_to`/`reaction_target: Option<&MessageKey>`; `ping: Option<&UserKey>`.
    /// Output: `Option<String>` — `Some(protocol_text)` if anything was sent, `None` if
    /// silent/empty. Uses: `strip_name`, `parse_reply`, `send_reply`. Used by: the
    /// stream-less fallback in `chat_reply.rs`, `Bot::post_problem`/`post_news`
    /// (`cycle_news.rs`), `poke_cycle` (`cycle_background.rs`),
    /// `handler_event.rs`'s `guild_member_addition`, `Bot::sleep_transition`/`evaluate_waking`
    /// (`cycle_sleep.rs`), `Bot::pick_name` (`cycle_growth.rs`).
    async fn send_lines(
        &self,
        ctx: &Context,
        channel: &RoomKey,
        raw: &str,
        reply_to: Option<&MessageKey>,
        reaction_target: Option<&MessageKey>,
        ping: Option<&UserKey>,
    ) -> Option<String> {
        let bot_name = self.state().bot_name.clone();
        let reply = parse_reply(strip_name(raw, &bot_name));
        self.send_reply(ctx, channel, reply, reply_to, reaction_target, ping)
            .await
    }

    // send_lines's body, operating on an already-parsed Reply: callers that already have a
    // parsed (and de-duplicated) reply on hand shouldn't have to go back to text and re-parse it
    /// Sends an already-parsed `Reply` line by line, with a typing delay between lines, then
    /// the reaction (if any). Input: `&self`; `ctx`/`channel` as above; `reply: Reply`
    /// (consumed, `reaction` may be cleared if `reaction_target` is `None`);
    /// `reply_to`/`reaction_target`/`ping` as above. Output: `Option<String>` —
    /// `Some(reply.protocol_text())` if anything went out, `None` if there was nothing to
    /// send. Uses: `self.send` (per line), `ctx.http.create_reaction`, `self.state()`,
    /// `channel_note`. Used by: `send_lines` above, and directly by the fallback path in
    /// `chat_reply.rs` (which already has a de-duplicated `Reply` on hand).
    async fn send_reply(
        &self,
        ctx: &Context,
        channel: &RoomKey,
        mut reply: Reply,
        reply_to: Option<&MessageKey>,
        reaction_target: Option<&MessageKey>,
        ping: Option<&UserKey>,
    ) -> Option<String> {
        let bot_name = self.state().bot_name.clone();
        // no message for the reaction to land on (the opening paths): the reaction is
        // dropped, otherwise nothing would go to the channel at all and the chat would
        // still be reported as "opened"
        if reaction_target.is_none() {
            reply.reaction = None;
        }
        if reply.lines.is_empty() && reply.reaction.is_none() {
            log::debug!(
                "send_lines [{channel}]: nothing to send (silent={})",
                reply.silent
            );
            return None;
        }
        let native_channel = match platform::discord::native_room(channel) {
            Ok(native) => native,
            Err(e) => { log::error!("send_lines [{channel}]: {e}"); return None; }
        };
        let native_ping = match ping.map(platform::discord::native_user).transpose() {
            Ok(native) => native,
            Err(e) => { log::error!("send_lines [{channel}]: {e}"); return None; }
        };
        for (i, line) in reply.lines.iter().enumerate() {
            if i > 0 {
                let wait_ms = (LINE_DELAY_BASE + LINE_DELAY_PER_CHAR * line.chars().count() as u64)
                    .min(LINE_DELAY_CAP);
                let _ = native_channel.broadcast_typing(&ctx.http).await;
                sleep(Duration::from_millis(wait_ms)).await;
            }
            let (ping_i, reply_i) = if i == 0 { (ping, reply_to) } else { (None, None) };
            // the mention is attached AFTER the protocol is parsed: pasting it onto the
            // front of the text first made "<@id> -" stop being recognized as the silence
            // marker, and "<@id> tepki: 💀" stop counting as a reaction line
            match if i == 0 { native_ping } else { None } {
                Some(u) => {
                    self.send(ctx, channel, &format!("<@{u}> {line}"), ping_i, None, reply_i)
                        .await
                }
                None => self.send(ctx, channel, line, ping_i, None, reply_i).await,
            }
        }
        if let (Some(emoji), Some(target)) = (&reply.reaction, reaction_target) {
            let native_target = match platform::discord::native_message(target) {
                Ok(native) => native,
                Err(e) => { log::warn!("reaction [{channel}]: {e}"); return Some(reply.protocol_text()); }
            };
            if let Err(e) = ctx
                .http
                .create_reaction(native_channel, native_target, &ReactionType::Unicode(emoji.clone()))
                .await
            {
                log::warn!("couldn't add reaction ({channel}): {e}");
            }
            // send() already recorded the lines; the reaction is recorded here (in protocol form)
            let mut s = self.state();
            channel_note(&mut s, channel, format!("{bot_name}: tepki: {emoji}"));
        }
        Some(reply.protocol_text())
    }
}
