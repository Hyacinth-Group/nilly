async fn cmd_reset(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    let all = option_bool(cmd, strings::t("cmd.sifirla.opt.hepsi.name")).unwrap_or(false);
    {
        let mut state = bot.state();
        if all {
            state.chats.clear();
            state.awaiting_comment.clear();
            state.busy.clear();
        } else {
            state.chats.remove(&cmd.channel_id);
            state.awaiting_comment.remove(&cmd.channel_id);
            state.busy.remove(&cmd.channel_id);
        }
    }
    reply_info(
        ctx,
        cmd,
        strings::t("reset.title"),
        if all {
            strings::t("reset.all")
        } else {
            strings::t("reset.channel")
        },
    )
    .await;
}

async fn cmd_news(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    defer(ctx, cmd).await;
    bot.state().chats.remove(&cmd.channel_id);
    let found = bot.post_news(ctx, cmd.channel_id).await;
    report_result(
        ctx,
        cmd,
        strings::t("news.title"),
        if found {
            strings::t("common.sent")
        } else {
            strings::t("news.not_found")
        },
    )
    .await;
}

async fn cmd_problem(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    defer(ctx, cmd).await;
    bot.state().chats.remove(&cmd.channel_id);
    bot.post_problem(ctx, cmd.channel_id).await;
    report_result(ctx, cmd, strings::t("problem.title"), strings::t("common.sent")).await;
}

async fn cmd_wander(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    defer(ctx, cmd).await;
    bot.wander().await;
    report_result(ctx, cmd, strings::t("wander.title"), strings::t("wander.done")).await;
}

async fn cmd_prank(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    prank_shared(bot, ctx, cmd, false).await;
}

async fn cmd_hack(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    prank_shared(bot, ctx, cmd, true).await;
}

/// Shared body for `cmd_prank`/`cmd_hack` above.
async fn prank_shared(bot: &Bot, ctx: &Context, cmd: &CommandInteraction, hack: bool) {
    defer(ctx, cmd).await;
    bot.state().chats.remove(&cmd.channel_id);
    bot.run_prank(ctx, cmd.channel_id, hack).await;
    let title = if hack {
        strings::t("prank.title_hack")
    } else {
        strings::t("prank.title_saka")
    };
    report_result(ctx, cmd, title, strings::t("common.sent")).await;
}

async fn cmd_agents(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    defer(ctx, cmd).await;
    bot.profiler().await;
    bot.coach().await;
    bot.state().index = memory::refresh_index();
    report_result(
        ctx,
        cmd,
        strings::t("agents_cmd.title"),
        strings::t("agents_cmd.done"),
    )
    .await;
}

async fn cmd_wake(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    defer(ctx, cmd).await;
    bot.wake();
    bot.sleep_transition(ctx).await;
    report_result(ctx, cmd, strings::t("wake.title"), strings::t("wake.done")).await;
}

async fn cmd_sleep(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    defer(ctx, cmd).await;
    let hours = option_int(cmd, strings::t("cmd.uyu.opt.saat.name")).unwrap_or(8);
    bot.put_to_sleep(hours);
    bot.sleep_transition(ctx).await;
    let description = strings::t("sleep_cmd.hours").replace("{saat}", &hours.to_string());
    report_result(ctx, cmd, strings::t("sleep_cmd.title"), &description).await;
}
