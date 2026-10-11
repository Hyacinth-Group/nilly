/// Returns the last active channel's last 40 lines only when no chat is open
/// and a profile is available.
fn idle_channel(bot: &Bot) -> Option<(ConversationKey, String)> {
    let state = bot.state();
    let channel = state.last_channel.as_ref()?;
    if state.chats.contains_key(channel) || state.profile.is_empty() {
        return None;
    }
    Some((channel.clone(), recent_messages(&state, 40)))
}

// every so often, speaks up unprompted like a familiar face would
async fn poke_cycle(bot: Arc<Bot>) {
    loop {
        sleep(POKE_INTERVAL).await;
        if SHUTTING_DOWN.load(Ordering::SeqCst) {
            return;
        }
        if !sleep::is_awake(&bot.state()) {
            continue;
        }
        // travel: gives notice a day before leaving, one message a day while on the road, nothing else
        let instruction = if let Some(trip) = travel::now() {
            if bot.state().last_road_message == travel::today() || rand::random::<f64>() > 0.25 {
                continue;
            }
            bot.state().last_road_message = travel::today();
            let _ = trip;
            prompts::current().on_the_way
        } else if let Some(trip) = travel::tomorrow() {
            if bot.state().announced_trip == trip.start {
                continue;
            }
            bot.state().announced_trip = trip.start;
            prompts::current().leaving
        } else {
            if rand::random::<f64>() > POKE_CHANCE * growth::stage(&bot.state().growth).poke {
                continue;
            }
            if rand::random::<f64>() < PROBLEM_SHARE {
                // post a small code gripe to the dev channel
                if let Some(channel) = default_channel(&bot) {
                    if !bot.state().chats.contains_key(&channel) {
                        if let Err(e) = bot.post_problem(&channel).await { log::warn!("problem: {e}"); }
                    }
                }
                continue;
            }
            prompts::current().out_of_the_blue
        };
        let Some((channel, recent)) = idle_channel(&bot) else {
            continue;
        };
        let context = bot.prompt_context(&channel);
        let instruction = context.render(instruction);

        let line = match bot.generate_for(&channel, &[user(recent)], &instruction, Some(120), "laf").await {
            Ok(l) => l,
            Err(e) => {
                log::error!("ai [poke]: {e}");
                continue;
            }
        };
        match bot
            .send_lines(&channel, &line, None, None, None, true)
            .await
        {
            Ok(Some(p)) => {
                start_chat(&mut bot.state(), &channel, Some(p));
            }
            Ok(None) => log::debug!("poke: model stayed silent, skipped"),
            Err(e) => log::warn!("poke delivery: {e}"),
        }
    }
}

// every so often, posts an image from the photos/ folder; sometimes with the hacked bit
async fn prank_cycle(bot: Arc<Bot>) {
    loop {
        sleep(PRANK_INTERVAL).await;
        if SHUTTING_DOWN.load(Ordering::SeqCst) {
            return;
        }
        if !sleep::is_awake(&bot.state()) || travel::now().is_some() {
            continue;
        }
        if rand::random::<f64>() > PRANK_CHANCE {
            continue;
        }
        let Some((channel, _)) = idle_channel(&bot) else {
            continue;
        };
        if let Err(e) = bot.run_prank(&channel, rand::random::<f64>() < HACK_SHARE).await {
            log::warn!("prank: {e}");
        }
    }
}

// browses the news every so often; the first pass is 10 min after startup, then every 4 hours
async fn wanderer_cycle(bot: Arc<Bot>) {
    let mut first = true;
    loop {
        sleep(if first {
            Duration::from_secs(600)
        } else {
            WANDERER_INTERVAL
        })
        .await;
        first = false;
        if SHUTTING_DOWN.load(Ordering::SeqCst) {
            return;
        }
        if sleep::is_awake(&bot.state()) {
            if let Err(error) = bot.wander().await {
                log::warn!("wander: deferred: {error}");
            }
        }
    }
}

// every 10 minutes: processes the queue of closed chats and observations into memory.
// Doesn't check whether the bot is asleep — things that pile up overnight get saved before morning too
/// Consumes the memory queue, including chats closed with no critic pass.
async fn memory_cycle(bot: Arc<Bot>) {
    loop {
        sleep(Duration::from_secs(10 * 60)).await;
        if SHUTTING_DOWN.load(Ordering::SeqCst) {
            return;
        }
        // messages keep being processed into memory even while asleep: a night observation
        // is queued every 2 hours
        {
            let mut state = bot.state();
            if !sleep::is_awake(&state) && now_unix() - state.last_night_observation >= 2 * 3600 {
                let recent = recent_messages(&state, 300);
                state.last_night_observation = now_unix();
                let observed = state.recent_users.clone();
                state.memory_queue.push_back((
                    recent,
                    "gece gözlemi (bot uykuda)".to_string(),
                    "gece".to_string(),
                    false,
                    observed,
                ));
            }
        }
        loop {
            let job = {
                let mut state = bot.state();
                if state.memory_queue.len() > 50 {
                    log::warn!(
                        "memory: queue overflowed ({}), dropping oldest",
                        state.memory_queue.len()
                    );
                    state.memory_queue.pop_front();
                }
                state.memory_queue.pop_front()
            };
            let Some((transcript_text, source, channel_name, run_critic, observed)) = job else {
                break;
            };
            let transcript_copy = transcript_text.clone();
            if let Err(e) = bot.diarist(transcript_text, &source, &channel_name, &observed).await {
                log::warn!(
                    "mind: diarist failed [{source}]: {}",
                    trim_error(&e.to_string())
                );
            }
            if run_critic {
                bot.critic(transcript_copy).await;
            }
        }
    }
}

// checks the sleep plan once a minute; on waking, replies to mentions received while asleep
async fn sleep_cycle(bot: Arc<Bot>) {
    loop {
        sleep(Duration::from_secs(60)).await;
        if SHUTTING_DOWN.load(Ordering::SeqCst) {
            return;
        }
        {
            let mut state = bot.state();
            sleep::update(&mut state);
        }
        if let Err(e) = bot.sleep_transition().await { log::warn!("sleep transition: {e}"); }
        bot.close_timed_out().await;
    }
}
