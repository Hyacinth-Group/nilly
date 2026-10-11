impl Bot {
    /// Builds the fixed/variable system-message pair for a chat reply.
    /// Input: `&self`; `history: &[ChatMessage]` — the chat so far (used to find who's
    /// talking); `instruction: &str` — this turn's task text. Output: `(fixed, variable,
    /// bot_name): (String, String, String)`. Uses: `memory::keywords`, `self.state()`,
    /// `memory::retrieve`, `system_text`. Used by: `generate`/`generate_stream` below —
    /// their shared setup step.
    fn chat_system(&self, history: &[ChatMessage], instruction: &str) -> (String, String, String) {
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
        let retrieved = memory::retrieve(&participants, &s.name_to_id, &keywords, &s.recent_messages, CHAT_SIZE);
        let (fixed, variable) = system_text(&s, instruction, &retrieved);
        (fixed, variable, s.bot_name.clone())
    }

    // talks with personality: chat, welcome, unprompted remarks, introducing news, pranks.
    // budget None means max_tokens isn't sent; chat replies get their budget from reply_budget!.
    // category only feeds the token metrics breakdown (!durum), it has no effect on the request.
    /// The single non-streaming personality-generation entry point (see AGENTS.md rule 6).
    /// Input: `&self`; `history: &[ChatMessage]`; `instruction: &str`; `budget: Option<u32>`;
    /// `category: &'static str`. Output: `Result<String, BotError>` — the cleaned reply text.
    /// Uses: `chat_system`, `self.ask_split`, `clean`. Used by: fallback/repeat paths in
    /// `chat_reply.rs`/`provider_send_stream.rs`, `Bot::post_problem`/`post_news`/
    /// `run_prank`/`pick_name` (`cycle_news.rs`, `cycle_growth.rs`), `poke_cycle`
    /// (`cycle_background.rs`), `handler_event.rs`'s `guild_member_addition`,
    /// `Bot::evaluate_waking` (`cycle_sleep.rs`), `Bot::chat_cli` (`chat_cli.rs`).
    async fn generate(
        &self,
        history: &[ChatMessage],
        instruction: &str,
        budget: Option<u32>,
        category: &'static str,
    ) -> Result<String, BotError> {
        let (fixed, variable, bot_name) = self.chat_system(history, instruction);
        let reply = self
            .ask_split(&fixed, &variable, history, budget, category)
            .await?;
        Ok(clean(reply, &bot_name))
    }

    // opens the chat reply as a stream; chunks are read from the reader as they arrive
    /// The streaming counterpart of `generate` (see AGENTS.md rule 6).
    /// Input: same as `generate`. Output: `Result<(StreamReader, String), BotError>` — the
    /// open reader plus the bot's current name (for `strip_name` while streaming). Uses:
    /// `chat_system`, `self.ask_raw_stream`. Used by: `Bot::reply` (`chat_reply.rs`), the
    /// only caller.
    async fn generate_stream(
        &self,
        history: &[ChatMessage],
        instruction: &str,
        budget: Option<u32>,
        category: &'static str,
    ) -> Result<(StreamReader, String), BotError> {
        let (fixed, variable, bot_name) = self.chat_system(history, instruction);
        let reader = self
            .ask_raw_stream(&fixed, &variable, history, budget, category)
            .await?;
        Ok((reader, bot_name))
    }

    // personality-free, plain analysis: used by the agents
    /// The single personality-free analysis entry point (see AGENTS.md rule 6).
    /// Input: `&self`; `text: &str` — the material to analyze; `instruction: &str` — the
    /// task; `max_tokens: u32`; `category: &'static str`. Output: `Result<String, BotError>`.
    /// Uses: `user`, `ANALYST` (`prompts.rs`), `self.ask`. Used by: `Bot::profiler`/`diarist`/
    /// `coach`/`critic`/`summarizer`/`news_agent` (`agents.rs`), `Bot::wander`
    /// (`agenda.rs`), `Bot::evaluate_waking` (`cycle_sleep.rs`).
    async fn analyze(
        &self,
        text: &str,
        instruction: &str,
        max_tokens: u32,
        category: &'static str,
    ) -> Result<String, BotError> {
        let input = user(format!("{text}\n\n---\n\n{instruction}"));
        self.ask(prompts::current().analyst, &[input], max_tokens, category)
            .await
    }

    // "do I want to join this conversation?" mini evaluation (0-10 score).
    // a mention/reply is always answered and never reaches this; None on error (the
    // fallback die roll takes over).
    // profile+index are in the fixed block (cache_control): they overlap with the same
    // content used in the main chat, and only change every 6 hours — kept separate so
    // they aren't sent at full price on every single message.
    /// Input: `&self` (reads recent messages + profile/index/name from `self.state()`).
    /// Output: `Option<(u8, String)>` — (score 0-10, reason), or `None` if there's no
    /// context yet or the call fails. Uses: `recent_messages`, `WILLINGNESS`/`ANALYST`
    /// (`prompts.rs`), `self.ask_split`, `parse_willingness` (`text_3.rs`). Used by:
    /// `Handler::message` (`handler_event.rs`), the only caller.
    async fn willingness(&self) -> Option<(u8, String)> {
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
        let fixed = format!("{}\n\nGRUP PROFİLİ\n{profile}\n\nKİŞİ DİZİNİ\n{index}", p.analyst);
        let variable = p.willingness.replace("{ad}", &bot_name);
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
    /// Input: `&self`; `waiting: &[String]` — names of people whose messages haven't been
    /// answered yet. Output: `Option<String>` — the chosen name, or `None` on failure. Uses:
    /// `recent_messages`, `TARGET_PICK`/`ANALYST` (`prompts.rs`), `self.ask_split`,
    /// `extract_target` (`text_3.rs`). Used by: `Bot::reply` (`chat_reply.rs`).
    async fn pick_target(&self, waiting: &[String]) -> Option<String> {
        let (context, bot_name) = {
            let s = self.state();
            (recent_messages(&s, 12), s.bot_name.clone())
        };
        let p = prompts::current();
        let fixed = format!("{}\n\n{}", p.analyst, p.target_pick.replace("{ad}", &bot_name));
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
    /// Input: `&self`; `history: &[ChatMessage]` — the chat so far (images stripped before
    /// sending, see comment below). Output: `Option<String>` — `"<state> (<intensity>)"`,
    /// or `None` if empty history / neutral / call failure. Uses: `MOOD`/`ANALYST`
    /// (`prompts.rs`), `self.ask_split`, `extract_mood` (`text_3.rs`). Used by: `Bot::reply`
    /// (`chat_reply.rs`).
    async fn determine_mood(&self, history: &[ChatMessage]) -> Option<String> {
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
        let variable = p.mood.replace("{ad}", &self.state().bot_name);
        match self
            .ask_split(p.analyst, &variable, &history, Some(40), "ruh_hali")
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
    /// Sends one plain Discord message and records it. Input: `&self`, `ctx: &Context`,
    /// `channel: ChannelId`, `text: &str`, `ping: Option<UserId>` (the one user allowed to
    /// be mentioned), `file: Option<&PathBuf>` (attachment), `reply_to: Option<MessageId>`
    /// (Discord reply-to target). Output: none (logs and returns early on failure). Uses:
    /// `CreateAllowedMentions`/`CreateMessage`/`CreateAttachment`, `self.state()`,
    /// `channel_note`. Used by: `send_lines`/`send_reply` (`provider_send_line.rs`), and
    /// directly wherever a single message goes out outside the line-by-line protocol.
    async fn send(
        &self,
        ctx: &Context,
        channel: ChannelId,
        text: &str,
        ping: Option<UserId>,
        file: Option<&PathBuf>,
        reply_to: Option<MessageId>, // when set, becomes a Discord reply and mentions that person
    ) {
        let mut msg = discord_message(channel, text, ping, reply_to);
        if let Some(path) = file {
            match CreateAttachment::path(path).await {
                Ok(attachment) => msg = msg.add_file(attachment),
                Err(e) => log::warn!("couldn't read image ({}): {e}", path.display()),
            }
        }
        if let Err(e) = channel.send_message(&ctx.http, msg).await {
            log::error!("send failed ({channel}): {e}");
            return;
        }
        let mut s = self.state();
        s.own_messages.push_back(text.to_string());
        if s.own_messages.len() > 50 {
            s.own_messages.pop_front();
        }
        let line = format!("{}: {}", s.bot_name, text);
        channel_note(&mut s, channel, line);
    }

    // sends a chat reply as a stream: the message appears early and is edited at intervals.
    // thinking stays in unclipped spoiler blocks; a reply over 1900 chars is split into a
    // new message. The reply-to link and the mention only go on the first message.
}

// Keep the native mention/reference shape shared by plain sends and stream openings.
fn discord_message(
    channel: ChannelId,
    text: &str,
    ping: Option<UserId>,
    reply_to: Option<MessageId>,
) -> CreateMessage {
    let mut mentions = CreateAllowedMentions::new();
    if let Some(user) = ping {
        mentions = mentions.users([user]);
    }
    if reply_to.is_some() {
        mentions = mentions.replied_user(true);
    }
    let mut message = CreateMessage::new().content(text).allowed_mentions(mentions);
    if let Some(id) = reply_to {
        message = message.reference_message((channel, id));
    }
    message
}

#[cfg(test)]
mod discord_send_fixtures {
    use super::*;

    #[test]
    fn plain_welcome_and_reply_mention_shapes() {
        let text = "selam <@42> <@&99> @everyone @here 💀";
        let channel = ChannelId::new(7);
        for (ping, reply_to, mentions, reference) in [
            (
                None,
                None,
                serde_json::json!({"parse": [], "users": [], "roles": []}),
                None,
            ),
            (
                Some(UserId::new(42)),
                None,
                serde_json::json!({"parse": [], "users": ["42"], "roles": []}),
                None,
            ),
            (
                None,
                Some(MessageId::new(81)),
                serde_json::json!({
                    "parse": [], "users": [], "roles": [], "replied_user": true
                }),
                Some(serde_json::json!({
                    "type": 0, "message_id": "81", "channel_id": "7",
                    "guild_id": null, "fail_if_not_exists": null
                })),
            ),
            (
                Some(UserId::new(42)),
                Some(MessageId::new(81)),
                serde_json::json!({
                    "parse": [], "users": ["42"], "roles": [], "replied_user": true
                }),
                Some(serde_json::json!({
                    "type": 0, "message_id": "81", "channel_id": "7",
                    "guild_id": null, "fail_if_not_exists": null
                })),
            ),
        ] {
            let actual =
                serde_json::to_value(discord_message(channel, text, ping, reply_to)).unwrap();
            assert_eq!(actual["content"].as_str().unwrap().as_bytes(), text.as_bytes());
            assert_eq!(actual["allowed_mentions"], mentions);
            assert_eq!(actual.get("message_reference"), reference.as_ref());
            assert!(actual.get("components").is_none());
            assert_eq!(actual["embeds"], serde_json::json!([]));
        }
    }
}
