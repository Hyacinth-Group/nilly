/// The 6-hourly background cycle: runs the profiler/coach, queues a general observation,
/// and posts a news item (or stashes one while asleep/skips while traveling).
async fn news_cycle(bot: Arc<Bot>) {
    loop {
        sleep(NEWS_INTERVAL).await;
        if SHUTTING_DOWN.load(Ordering::SeqCst) {
            return;
        }
        if !sleep::is_awake(&bot.state()) {
            // no news posted while asleep, but it's still picked: on waking it's stashed as the "morning news"
            let stash_empty = bot.state().stashed_news.is_none();
            if stash_empty {
                match bot.news_agent().await {
                    Ok(Some(item)) => {
                        bot.state().stashed_news = Some(item);
                        log::debug!("news: stashed while asleep");
                    }
                    Ok(None) => log::debug!("news: no relevant sleep stash"),
                    Err(e) => log::debug!("news: couldn't pick a sleep stash: {e}"),
                }
            }
            continue;
        }
        if travel::now().is_some() {
            // no news posted while traveling, but the agents still run to keep learning
            if let Err(e) = bot.profiler().await { log::warn!("profiler: {e}"); }
            if let Err(e) = bot.coach().await { log::warn!("coach: {e}"); }
            continue;
        }

        bot.check_growth().await;
        if let Err(e) = bot.profiler().await { log::warn!("profiler: {e}"); }
        {
            let mut state = bot.state();
            let recent = recent_messages(&state, 300);
            let observed = state.recent_users.clone();
            // Observed identities are captured with the transcript, never reconstructed later.
            state.memory_queue.push_back((
                recent,
                "6 saatlik gözlem, bot konuşmamış olabilir".to_string(),
                "gozlem".to_string(),
                false,
                observed,
            ));
        }
        if let Err(e) = bot.coach().await { log::warn!("coach: {e}"); }

        let Some(channel) = default_channel(&bot) else {
            continue;
        };
        if bot.state().chats.contains_key(&channel) {
            continue;
        }

        // if there's news stashed from while asleep, that goes out first (the "morning news")
        let stash = bot.state().stashed_news.take();
        let result = match stash {
            Some(item) => bot.send_news(&channel, item).await.map(Some),
            None => bot.post_news(&channel).await,
        };
        if let Err(e) = result { log::warn!("news: {e}"); }
    }
}

impl Bot {
    // posts a small, made-up but believable software gripe, asking "how do I fix this"
    async fn post_problem(&self, channel: &ConversationKey) -> Result<bool, BotError> {
        self.transport(channel)?;
        let recent = recent_messages(&self.state(), 30);
        let context = self.prompt_context(channel);
        let instruction = context.render(prompts::current().problem);
        let line = self.generate_for(channel, &[user(recent)], &instruction, Some(160), "sorun").await?;
        let Some(posted) = self.send_lines(channel, &line, None, None, None, false).await? else {
            return Ok(false);
        };
        start_chat(&mut self.state(), channel, Some(posted));
        Ok(true)
    }

    /// Returns `None` for no relevant news, `Some(false)` for model silence,
    /// and `Some(true)` only after the introduction and link are delivered.
    async fn post_news(&self, channel: &ConversationKey) -> Result<Option<bool>, BotError> {
        let (adapter, room) = self.transport(channel)?;
        if !adapter.capabilities(&room).send {
            return Err(platform::PlatformError::Unsupported("send").into());
        }
        let Some(item) = self.news_agent().await? else { return Ok(None); };
        self.send_news(channel, item).await.map(Some)
    }

    // shares an already-picked news item: both this turn's pick and the one stashed while asleep go through here
    /// Returns `false` for model silence; generation and delivery failures propagate.
    async fn send_news(&self, channel: &ConversationKey, item: agents::News) -> Result<bool, BotError> {
        self.transport(channel)?;
        let link = if item.url.starts_with("https://") || item.url.starts_with("http://") {
            item.url.clone()
        } else {
            format!("https://news.ycombinator.com/item?id={}", item.id)
        };
        let context = self.prompt_context(channel);
        let instruction = context.render(prompts::current().news_intro);
        let intro = self.generate_for(
            channel,
            &[user(item.title.clone())],
            &instruction,
            Some(200),
            "haber_tanit",
        ).await?;
        // the intro goes out line by line, the link follows as its own message (that's how people post links too)
        let Some(posted) = self
            .send_lines(channel, &intro, None, None, None, false)
            .await?
        else {
            return Ok(false);
        };
        self.send(channel, &link, None, None, None).await?;

        let mut state = self.state();
        start_chat(&mut state, channel, Some(posted));
        state
            .awaiting_comment
            .insert(channel.clone(), Instant::now() + COMMENT_WINDOW);
        state.posted_news.insert(item.id);
        Ok(true)
    }

    // an image prank; if it's the hacked variant, it opens with the "hacked" bit
    async fn run_prank(&self, channel: &ConversationKey, hack: bool) -> Result<bool, BotError> {
        let (adapter, room) = self.transport(channel)?;
        if !adapter.capabilities(&room).send {
            return Err(platform::PlatformError::Unsupported("send").into());
        }
        if !adapter.capabilities(&room).send_attachments {
            return Err(platform::PlatformError::Unsupported("attachments").into());
        }
        let image = random_image().ok_or("no prank images available")?;
        let text = if hack {
            let context = self.prompt_context(channel);
            let instruction = context.render(prompts::current().hack_enter);
            self.generate_for(
                channel,
                &[user("şaka başlıyor")],
                &instruction,
                Some(150),
                "hack_giris",
            )
            .await
        } else {
            self.image_commenter(channel, &image).await
        };
        let text = text?;
        // the image goes out in a single message: only the first line is taken from the
        // protocol, decorations cleaned up. strip_name is applied here just like every
        // other path (name prefix, quotes)
        let bot_name = self.state().bot_name.clone();
        let limits = adapter.capabilities(&room).output;
        let reply = parse_reply(strip_name(&text, &bot_name), limits);
        let Some(first) = reply.lines.first().cloned() else {
            log::debug!("prank: model stayed silent, prank skipped");
            return Ok(false);
        };
        self.send(channel, &first, None, Some(&image), None)
            .await?;

        let mut state = self.state();
        let chat = start_chat(&mut state, channel, Some(first));
        if hack {
            chat.hacked = HACK_MESSAGES;
        }
        Ok(true)
    }
}
