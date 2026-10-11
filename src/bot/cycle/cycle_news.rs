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
                    Ok(item) => {
                        bot.state().stashed_news = Some(item);
                        log::debug!("news: stashed while asleep");
                    }
                    Err(e) => log::debug!("news: couldn't pick a sleep stash: {e}"),
                }
            }
            continue;
        }
        if travel::now().is_some() {
            // no news posted while traveling, but the agents still run to keep learning
            bot.profiler().await;
            bot.coach().await;
            continue;
        }

        bot.check_growth().await;
        bot.profiler().await;
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
        bot.coach().await;

        let Some(channel) = default_channel(&bot) else {
            continue;
        };
        if bot.state().chats.contains_key(&channel) {
            continue;
        }

        // if there's news stashed from while asleep, that goes out first (the "morning news")
        let stash = bot.state().stashed_news.take();
        match stash {
            Some(item) => {
                bot.send_news(&channel, item).await;
            }
            None => {
                bot.post_news(&channel).await;
            }
        }
    }
}

impl Bot {
    // posts a small, made-up but believable software gripe, asking "how do I fix this"
    async fn post_problem(&self, channel: &ConversationKey) {
        let recent = recent_messages(&self.state(), 30);
        let context = self.prompt_context(channel);
        let instruction = context.render(prompts::current().problem);
        match self
            .generate_for(channel, &[user(recent)], &instruction, Some(160), "sorun")
            .await
        {
            Ok(line) => match self
                .send_lines(channel, &line, None, None, None)
                .await
            {
                Some(p) => {
                    start_chat(&mut self.state(), channel, Some(p));
                }
                None => log::debug!("problem: model stayed silent, skipped"),
            },
            Err(e) => log::error!("ai [post_problem]: {e}"),
        }
    }

    // picks a news item and posts it to the channel, opening a chat that waits for comments
    /// Returns whether a news item was posted.
    async fn post_news(&self, channel: &ConversationKey) -> bool {
        let item = match self.news_agent().await {
            Ok(item) => item,
            Err(e) => {
                log::warn!("news_agent: {e}");
                return false;
            }
        };
        self.send_news(channel, item).await
    }

    // shares an already-picked news item: both this turn's pick and the one stashed while asleep go through here
    /// Returns `false` if the intro call fails or the model stays silent.
    async fn send_news(&self, channel: &ConversationKey, item: agents::News) -> bool {
        let link = if item.url.starts_with("https://") || item.url.starts_with("http://") {
            item.url.clone()
        } else {
            format!("https://news.ycombinator.com/item?id={}", item.id)
        };
        let context = self.prompt_context(channel);
        let instruction = context.render(prompts::current().news_intro);
        let intro = match self
            .generate_for(
                channel,
                &[user(item.title.clone())],
                &instruction,
                Some(200),
                "haber_tanit",
            )
            .await
        {
            Ok(g) => g,
            Err(e) => {
                log::error!("ai [haber_tanit]: {e}");
                return false;
            }
        };
        // the intro goes out line by line, the link follows as its own message (that's how people post links too)
        let Some(posted) = self
            .send_lines(channel, &intro, None, None, None)
            .await
        else {
            log::debug!("news: model stayed silent for the intro, news item skipped");
            return false;
        };
        self.send(channel, &link, None, None, None).await;

        let mut state = self.state();
        start_chat(&mut state, channel, Some(posted));
        state
            .awaiting_comment
            .insert(channel.clone(), Instant::now() + COMMENT_WINDOW);
        state.posted_news.insert(item.id);
        true
    }

    // an image prank; if it's the hacked variant, it opens with the "hacked" bit
    async fn run_prank(&self, channel: &ConversationKey, hack: bool) {
        let Some(image) = random_image() else {
            if self.stealth {
                log::warn!("prank: no images in photos/, skipped");
                return;
            }
            self.send_card(
                channel,
                strings::t("prank.title"),
                strings::t("prank.no_images"),
            )
            .await;
            return;
        };
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
        let text = match text {
            Ok(m) => m,
            Err(e) => {
                log::error!("ai [prank]: {e}");
                return;
            }
        };
        // the image goes out in a single message: only the first line is taken from the
        // protocol, decorations cleaned up. strip_name is applied here just like every
        // other path (name prefix, quotes)
        let bot_name = self.state().bot_name.clone();
        let limits = match self.transport(channel) {
            Ok((adapter, room)) => adapter.capabilities(&room).output,
            Err(error) => {
                log::warn!("prank [{channel}]: {error}");
                return;
            }
        };
        let reply = parse_reply(strip_name(&text, &bot_name), limits);
        let Some(first) = reply.lines.first().cloned() else {
            log::debug!("prank: model stayed silent, prank skipped");
            return;
        };
        self.send(channel, &first, None, Some(&image), None)
            .await;

        let mut state = self.state();
        let chat = start_chat(&mut state, channel, Some(first));
        if hack {
            chat.hacked = HACK_MESSAGES;
        }
    }
}
