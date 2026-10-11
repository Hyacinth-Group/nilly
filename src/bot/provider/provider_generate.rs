impl Bot {
    /// Builds the fixed/variable system-message pair for a chat reply.
    fn chat_system(&self, history: &[ChatMessage], instruction: &str, context: &platform::prompt_context::PromptContext) -> (String, String, String) {
        let mut participants: Vec<String> = Vec::new();
        let mut texts: Vec<String> = Vec::new();
        for m in history.iter().filter(|m| m.role == "user") {
            match m.content.split_once(": ") {
                Some((name, text)) => {
                    // no temporary String for contains: compared against the slice directly
                    if !participants.iter().any(|k| k.as_str() == name) {
                        participants.push(name.to_string());
                    }
                    texts.push(text.to_string());
                }
                None => texts.push(m.content.clone()),
            }
        }
        let keywords = memory::keywords(&texts);
        let s = self.state();
        let identities = observed_users(history).into_iter().map(|(label, person)| (label, person.id)).collect();
        let retrieved = memory::retrieve(&participants, &identities, &keywords, &s.recent_messages, CHAT_SIZE);
        let (fixed, variable) = system_text_with(&s, instruction, &retrieved, context);
        (fixed, variable, s.bot_name.clone())
    }

    // talks with personality: chat, welcome, unprompted remarks, introducing news, pranks.
    // budget None means max_tokens isn't sent; chat replies get their budget from reply_budget!.
    // category only feeds the token metrics breakdown (!durum), it has no effect on the request.
    /// Background personality generation. Render the trusted instruction template once before calling;
    /// substituted names, observations, and research remain literal data.
    async fn generate(
        &self,
        history: &[ChatMessage],
        instruction: &str,
        budget: Option<u32>,
        category: &'static str,
    ) -> Result<String, BotError> {
        let (fixed, variable, bot_name) = self.chat_system(history, instruction, &self.default_prompt_context());
        let reply = self
            .ask_split(&fixed, &variable, history, budget, category)
            .await?;
        Ok(clean(reply, &bot_name))
    }


    fn default_prompt_context(&self) -> platform::prompt_context::PromptContext {
        let state = self.state();
        let kind = state.primary_connection.as_ref().and_then(|connection| state.accounts.get(connection))
            .map(|account| account.id.platform()).unwrap_or(platform::types::PlatformKind::Discord);
        platform::prompt_context::PromptContext::for_platform(kind)
    }

    fn prompt_context(&self, channel: &ConversationKey) -> platform::prompt_context::PromptContext {
        let context = platform::prompt_context::PromptContext::for_platform(channel.room.platform());
        match self.transport(channel) {
            Ok((adapter, room)) => context.with_capabilities(&adapter.capabilities(&room)),
            Err(_) => context.with_capabilities(&platform::capabilities::Capabilities::unavailable()),
        }
    }

    /// Routed personality generation; `instruction` is already rendered and never expanded again.
    async fn generate_for(&self, channel: &ConversationKey, history: &[ChatMessage], instruction: &str,
        budget: Option<u32>, category: &'static str) -> Result<String, BotError> {
        let (fixed, variable, bot_name) = self.chat_system(history, instruction, &self.prompt_context(channel));
        let reply = self.ask_split(&fixed, &variable, history, budget, category).await?;
        Ok(clean(reply, &bot_name))
    }

    /// Routed streaming counterpart, using the same prepared-instruction contract.
    async fn generate_stream_for(&self, channel: &ConversationKey, history: &[ChatMessage], instruction: &str,
        budget: Option<u32>, category: &'static str) -> Result<(StreamReader, String), BotError> {
        let (fixed, variable, bot_name) = self.chat_system(history, instruction, &self.prompt_context(channel));
        let reader = self.ask_raw_stream(&fixed, &variable, history, budget, category).await?;
        Ok((reader, bot_name))
    }

    // personality-free, plain analysis: used by the agents
    /// Analysis with an already-rendered instruction; only the trusted analyst system template is expanded.
    async fn analyze(
        &self,
        text: &str,
        instruction: &str,
        max_tokens: u32,
        category: &'static str,
    ) -> Result<String, BotError> {
        self.analyze_with(text, instruction, max_tokens, category, &self.default_prompt_context()).await
    }

    async fn analyze_with(&self, text: &str, instruction: &str, max_tokens: u32,
        category: &'static str, context: &platform::prompt_context::PromptContext) -> Result<String, BotError> {


        let input = user(format!("{text}\n\n---\n\n{instruction}"));
        self.ask(&context.render(prompts::current().analyst), &[input], max_tokens, category).await
    }

    // "do I want to join this conversation?" mini evaluation (0-10 score).
    // a mention/reply is always answered and never reaches this; None on error (the
    // fallback die roll takes over).
    // profile+index are in the fixed block (cache_control): they overlap with the same
    // content used in the main chat, and only change every 6 hours — kept separate so
    // they aren't sent at full price on every single message.
    /// Returns a 0–10 score and reason, or `None` without context or on failure.
    async fn willingness(&self, channel: &ConversationKey) -> Option<(u8, String)> {
        let (context, profile, index, bot_name) = {
            let s = self.state();
            (
                recent_messages(&s, 12),
                s.profile.clone(),
                s.index.clone(),
                s.bot_name.clone(),
            )
        };
        if context.trim().is_empty() {
            return None;
        }
        let p = prompts::current();
        let prompt = self.prompt_context(channel);
        let fixed = format!("{}\n\nGRUP PROFİLİ\n{profile}\n\nKİŞİ DİZİNİ\n{index}", prompt.render(p.analyst));
        let variable = prompt.render_with(p.willingness, &[("{ad}", &bot_name)]);
        let input = user(format!("SON MESAJLAR\n{context}"));
        match self
            .ask_split(&fixed, &variable, &[input], Some(80), "isteklilik")
            .await
        {
            Ok(c) => parse_willingness(&c),
            Err(e) => {
                log::debug!("willingness: call failed: {e}");
                None
            }
        }
    }

    // when several different people write in a row, who should the reply go to? picks
    // among the pending names.
    // instruction is in the fixed block (cache_control): it doesn't include profile/index,
    // but at least it's constant on its own.
    /// Returns `None` if target selection fails.
    async fn pick_target(&self, channel: &ConversationKey, waiting: &[String]) -> Option<String> {
        let (context, bot_name) = {
            let s = self.state();
            (recent_messages(&s, 12), s.bot_name.clone())
        };
        let p = prompts::current();
        let prompt = self.prompt_context(channel);
        let fixed = format!("{}\n\n{}", prompt.render(p.analyst), prompt.render_with(p.target_pick, &[("{ad}", &bot_name)]));
        let variable = format!("BEKLEYENLER\n- {}", waiting.join("\n- "));
        let input = user(format!("SON MESAJLAR\n{context}"));
        match self
            .ask_split(&fixed, &variable, &[input], Some(40), "hedef_sec")
            .await
        {
            Ok(c) => extract_target(&c, waiting),
            Err(e) => {
                log::debug!("pick_target: call failed: {e}");
                None
            }
        }
    }

    // determines this chat's mood: a cheap mini call, only made by `reply` when a chat
    // opens and every few turns after (not on every message). Neutral/low intensity or an
    // error -> None, which the caller treats as "no clear mood".
    async fn determine_mood(&self, channel: &ConversationKey, history: &[ChatMessage]) -> Option<String> {
        if history.is_empty() {
            return None;
        }
        // the image never goes into this mini call: sending the full image payload for a
        // 40-token mood analysis burns tokens, and would fail the call outright on a route
        // without vision support
        let history: Vec<ChatMessage> = history
            .iter()
            .map(|m| ChatMessage {
                image: None,
                ..m.clone()
            })
            .collect();
        let p = prompts::current();
        let prompt = self.prompt_context(channel);
        let variable = prompt.render_with(p.mood, &[("{ad}", &self.state().bot_name)]).into_owned();
        match self
            .ask_split(&prompt.render(p.analyst), &variable, &history, Some(40), "ruh_hali")
            .await
        {
            Ok(c) => extract_mood(&c),
            Err(e) => {
                log::debug!("mood: call failed: {e}");
                None
            }
        }
    }

    // mentions always go out disabled: even if the model writes @everyone, nobody gets pinged.
    // everything sent lands in own_messages, which the coach and critic read from.
    /// Sends one message and records it; send failures are logged.
    /// `ping` is the only user allowed to be mentioned.
    async fn send(
        &self,
        channel: &ConversationKey,
        text: &str,
        ping: Option<&UserKey>,
        file: Option<&PathBuf>,
        reply_to: Option<&MessageKey>,
    ) {
        let (transport, room) = match self.transport(channel) {
            Ok(transport) => transport,
            Err(e) => { log::error!("send failed ({channel}): {e}"); return; }
        };

        let target = reply_to.map(|id| platform::types::MessageRef {
            id: id.clone(), room: channel.room.clone(), author: None,
        });
        let attachments = file
            .map(|path| platform::outgoing::OutgoingAttachment::LocalPath(path.as_path()));
        let mut body = platform::outgoing::OutgoingMessage::text(text);
        body.reply_to = target.as_ref();
        body.mentions = platform::outgoing::MentionPolicy {
            users: ping.map(std::slice::from_ref).unwrap_or_default(),
            reply_user: reply_to.is_some(),
            prefix_user: ping,
        };
        body.attachments = attachments.as_slice();
        let receipt = match transport.send(&room, &body).await {
            Ok(receipt) => receipt,
            Err(e) => { log::error!("send failed ({channel}): {e}"); return; }
        };
        let mut s = self.state();
        s.own_messages.push_back(receipt.last_accepted_text.clone());
        if s.own_messages.len() > 50 {
            s.own_messages.pop_front();
        }
        let line = format!("{}: {}", s.bot_name, receipt.last_accepted_text);
        channel_note(&mut s, channel, line);
    }

    // sends a chat reply as a stream: the message appears early and is edited at intervals.
    // thinking stays in unclipped spoiler blocks; a reply over 1900 chars is split into a
    // new message. The reply-to link and the mention only go on the first message.
}

