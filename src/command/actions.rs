/// `/sifirla [hepsi]`. Input: `bot: &Bot`; `ctx: &Context`; `cmd: &CommandInteraction`.
/// Output: none. Uses: `option_bool`, `bot.state()`, `reply_info`. Registered as `run` for
/// `"sifirla"` in `registration_table.rs`.
async fn cmd_reset(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    let channel = RoomKey::discord(cmd.channel_id.get());
    let all = option_bool(cmd, strings::t("cmd.sifirla.opt.hepsi.name")).unwrap_or(false);
    {
        let mut state = bot.state();
        if all {
            state.chats.clear();
            state.awaiting_comment.clear();
            state.busy.clear();
        } else {
            state.chats.remove(&channel);
            state.awaiting_comment.remove(&channel);
            state.busy.remove(&channel);
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

/// `/haber`. Input: `bot: &Bot`; `ctx: &Context`; `cmd: &CommandInteraction`. Output: none.
/// Uses: `defer`, `bot.state()`, `bot.post_news` (`cycle_news.rs`), `report_result`.
/// Registered as `run` for `"haber"` in `registration_table.rs`.
async fn cmd_news(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    defer(ctx, cmd).await;
    let channel = RoomKey::discord(cmd.channel_id.get());
    bot.state().chats.remove(&channel);
    let found = bot.post_news(ctx, &channel).await;
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

/// `/sorun`. Input: `bot: &Bot`; `ctx: &Context`; `cmd: &CommandInteraction`. Output: none.
/// Uses: `defer`, `bot.state()`, `bot.post_problem` (`cycle_news.rs`), `report_result`.
/// Registered as `run` for `"sorun"` in `registration_table.rs`.
async fn cmd_problem(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    defer(ctx, cmd).await;
    let channel = RoomKey::discord(cmd.channel_id.get());
    bot.state().chats.remove(&channel);
    bot.post_problem(ctx, &channel).await;
    report_result(ctx, cmd, strings::t("problem.title"), strings::t("common.sent")).await;
}

/// `/gez`. Input: `bot: &Bot`; `ctx: &Context`; `cmd: &CommandInteraction`. Output: none.
/// Uses: `defer`, `bot.wander` (`agenda.rs`), `report_result`. Registered as `run` for
/// `"gez"` in `registration_table.rs`.
async fn cmd_wander(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    defer(ctx, cmd).await;
    bot.wander().await;
    report_result(ctx, cmd, strings::t("wander.title"), strings::t("wander.done")).await;
}

/// `/saka`. Input: `bot: &Bot`; `ctx: &Context`; `cmd: &CommandInteraction`. Output: none.
/// Uses: `prank_shared` below (`hack=false`). Registered as `run` for `"saka"` in
/// `registration_table.rs`.
async fn cmd_prank(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    prank_shared(bot, ctx, cmd, false).await;
}

/// `/hack`. Input: `bot: &Bot`; `ctx: &Context`; `cmd: &CommandInteraction`. Output: none.
/// Uses: `prank_shared` below (`hack=true`). Registered as `run` for `"hack"` in
/// `registration_table.rs`.
async fn cmd_hack(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    prank_shared(bot, ctx, cmd, true).await;
}

/// Shared body for `cmd_prank`/`cmd_hack` above.
/// Input: `bot: &Bot`; `ctx: &Context`; `cmd: &CommandInteraction`; `hack: bool`. Output:
/// none. Uses: `defer`, `bot.state()`, `bot.run_prank` (`cycle_news.rs`), `report_result`.
async fn prank_shared(bot: &Bot, ctx: &Context, cmd: &CommandInteraction, hack: bool) {
    defer(ctx, cmd).await;
    let channel = RoomKey::discord(cmd.channel_id.get());
    bot.state().chats.remove(&channel);
    bot.run_prank(ctx, &channel, hack).await;
    let title = if hack {
        strings::t("prank.title_hack")
    } else {
        strings::t("prank.title_saka")
    };
    report_result(ctx, cmd, title, strings::t("common.sent")).await;
}

/// `/ajanlar`. Input: `bot: &Bot`; `ctx: &Context`; `cmd: &CommandInteraction`. Output:
/// none. Uses: `defer`, `bot.profiler`/`coach` (`agents.rs`), `memory::refresh_index`,
/// `report_result`. Registered as `run` for `"ajanlar"` in `registration_table.rs`.
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

/// `/uyan`. Input: `bot: &Bot`; `ctx: &Context`; `cmd: &CommandInteraction`. Output: none.
/// Uses: `defer`, `bot.wake`/`sleep_transition` (`command/remaining.rs`/`cycle_sleep.rs`),
/// `report_result`. Registered as `run` for `"uyan"` in `registration_table.rs`.
async fn cmd_wake(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    defer(ctx, cmd).await;
    bot.wake();
    bot.sleep_transition(ctx).await;
    report_result(ctx, cmd, strings::t("wake.title"), strings::t("wake.done")).await;
}

/// `/uyu [saat]`. Input: `bot: &Bot`; `ctx: &Context`; `cmd: &CommandInteraction`. Output:
/// none. Uses: `defer`, `option_int`, `bot.put_to_sleep`/`sleep_transition`
/// (`command/remaining.rs`/`cycle_sleep.rs`), `report_result`. Registered as `run` for
/// `"uyu"` in `registration_table.rs`.
async fn cmd_sleep(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    defer(ctx, cmd).await;
    let hours = option_int(cmd, strings::t("cmd.uyu.opt.saat.name")).unwrap_or(8);
    bot.put_to_sleep(hours);
    bot.sleep_transition(ctx).await;
    let description = strings::t("sleep_cmd.hours").replace("{saat}", &hours.to_string());
    report_result(ctx, cmd, strings::t("sleep_cmd.title"), &description).await;
}
