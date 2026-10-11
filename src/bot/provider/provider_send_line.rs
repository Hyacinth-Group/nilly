impl Bot {
    /// Automatic replies must still be approved at delivery, not just when queued.
    fn authorize_automatic_delivery(&self, channel: &ConversationKey) -> Result<(), BotError> {
        let (adapter, room) = self.transport(channel)?;
        let registry = self.adapters.lock().unwrap_or_else(|error| error.into_inner());
        if !registry.allows(adapter.connection_id(), &room) {
            return Err(platform::PlatformError::PermissionDenied.into());
        }
        Ok(())
    }

    /// Parses raw model text and sends it via the non-streaming, line-by-line protocol.
    /// Returns protocol text if anything was sent, or `None` for silence/empty output.
    async fn send_lines(
        &self,
        channel: &ConversationKey,
        raw: &str,
        reply_to: Option<&MessageKey>,
        reaction_target: Option<&MessageKey>,
        ping: Option<&UserKey>,
        approved_only: bool,
    ) -> Result<Option<String>, BotError> {
        let bot_name = self.state().bot_name.clone();
        let (transport, room) = self.transport(channel)?;
        if approved_only { self.authorize_automatic_delivery(channel)?; }
        if !transport.capabilities(&room).send {
            return Err(platform::PlatformError::Unsupported("send").into());
        }

        let reply = parse_reply(strip_name(raw, &bot_name), transport.capabilities(&room).output);
        self.send_reply(channel, reply, reply_to, reaction_target, ping, approved_only)
            .await
    }

    // send_lines's body, operating on an already-parsed Reply: callers that already have a
    // parsed (and de-duplicated) reply on hand shouldn't have to go back to text and re-parse it
    /// Sends an already-parsed `Reply` line by line, with a typing delay between lines, then
    /// the reaction (if any). Without a reaction target, the reaction is dropped.
    /// Returns protocol text if anything was sent, or `None` otherwise.
    async fn send_reply(
        &self,
        channel: &ConversationKey,
        mut reply: Reply,
        reply_to: Option<&MessageKey>,
        reaction_target: Option<&MessageKey>,
        ping: Option<&UserKey>,
        approved_only: bool,
    ) -> Result<Option<String>, BotError> {
        let bot_name = self.state().bot_name.clone();
        if approved_only { self.authorize_automatic_delivery(channel)?; }
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
            return Ok(None);
        }
        let (transport, room) = self.transport(channel)?;
        let room = transport.route_reply(room, reply_to)?;

        let capabilities = transport.capabilities(&room);
        for (i, line) in reply.lines.iter().enumerate() {
            if i > 0 {
                let wait_ms = (LINE_DELAY_BASE + LINE_DELAY_PER_CHAR * line.chars().count() as u64)
                    .min(LINE_DELAY_CAP);
                if capabilities.typing {
                    let _ = transport.typing(&room).await;
                }
                sleep(Duration::from_millis(wait_ms)).await;
            }
            let (ping_i, reply_i) = if i == 0 { (ping, reply_to) } else { (None, None) };
            // Prefix mentions are rendered only after parsing the reply protocol.
            if approved_only { self.authorize_automatic_delivery(channel)?; }
            self.send_in_room((&transport, &room), channel, line, ping_i, None, reply_i).await?;
        }
        if let (Some(emoji), Some(target)) = (&reply.reaction, reaction_target) {
            if approved_only { self.authorize_automatic_delivery(channel)?; }
            if capabilities.react {
                let target = platform::types::MessageRef {
                    id: target.clone(), room: channel.room.clone(), author: None,
                };
                match transport.react(&target, &platform::types::Emoji::Unicode(emoji.clone())).await {
                    Ok(()) => {}
                    Err(platform::PlatformError::Unsupported(_)) => self.send_in_room((&transport, &room), channel, emoji, None, None, None).await?,
                    Err(error) => return Err(error.into()),
                }
            } else {
                self.send_in_room((&transport, &room), channel, emoji, None, None, None).await?;
            }
            // send() already recorded the lines; the reaction is recorded here (in protocol form)
            let mut s = self.state();
            channel_note(&mut s, channel, format!("{bot_name}: tepki: {emoji}"));
        }
        Ok(Some(reply.protocol_text()))
    }
}
