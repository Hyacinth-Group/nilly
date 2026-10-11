/// `/dusunme [kip]`. Input: `bot: &Bot`; `ctx: &Context`; `cmd: &CommandInteraction`.
/// Output: none. Uses: `option_text`, `ThinkingMode::from_arg`/`file_value`/`label`,
/// `bot.state()`, `memory::write`, `reply_info`. Registered as `run` for `"dusunme"` in
/// `registration_table.rs`.
async fn cmd_thinking(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    match option_text(cmd, strings::t("cmd.dusunme.opt.kip.name")).and_then(ThinkingMode::from_arg)
    {
        Some(new_mode) => {
            bot.state().thinking_mode = new_mode;
            memory::write("dusunme.md", new_mode.file_value());
            let description = strings::t("thinking.set").replace("{mode}", new_mode.label());
            reply_info(ctx, cmd, strings::t("thinking.title"), &description).await;
        }
        None => {
            let mode = bot.state().thinking_mode;
            let description = strings::t("thinking.current").replace("{mode}", mode.label());
            reply_info(ctx, cmd, strings::t("thinking.title"), &description).await;
        }
    }
}

/// `/model [id]`. Input: `bot: &Bot`; `ctx: &Context`; `cmd: &CommandInteraction`. Output:
/// none. Uses: `option_text`, `bot.state()`, `is_owner`, `defer`, `bot.model_exists`
/// (`command/remaining.rs`), `memory::write`, `reply_info`/`report_result`. Registered as
/// `run` for `"model"` in `registration_table.rs`.
async fn cmd_model(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    match option_text(cmd, strings::t("cmd.model.opt.id.name")) {
        None => {
            let m = bot.state().model.clone();
            let description = strings::t("model_cmd.current").replace("{model}", &m);
            reply_info(ctx, cmd, strings::t("model_cmd.title"), &description).await;
        }
        Some(id) if !is_owner(&UserKey::discord(cmd.user.id.get())) => {
            let _ = id;
            reply_info(
                ctx,
                cmd,
                strings::t("model_cmd.title"),
                strings::t("model_cmd.forbidden"),
            )
            .await;
        }
        Some(id) => {
            let id = id.to_string();
            defer(ctx, cmd).await;
            // model_exists only queries openrouter's catalog; a custom router (API_URL)
            // or mistral's native api has no equivalent lookup, so the id is trusted as-is
            if bot.api_url.contains("openrouter") && !bot.model_exists(&id).await {
                report_result(
                    ctx,
                    cmd,
                    strings::t("model_cmd.title"),
                    strings::t("model_cmd.unknown"),
                )
                .await;
            } else {
                bot.state().model = id.clone();
                memory::write("model.md", &id);
                let description = strings::t("model_cmd.set").replace("{model}", &id);
                report_result(ctx, cmd, strings::t("model_cmd.title"), &description).await;
            }
        }
    }
}

/// `/debug [durum]`. Input: `bot: &Bot`; `ctx: &Context`; `cmd: &CommandInteraction`.
/// Output: none. Uses: `option_text`, `bot.set_debug` (`command/remaining.rs`),
/// `reply_info`. Registered as `run` for `"debug"` in `registration_table.rs`.
async fn cmd_debug(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    let arg = option_text(cmd, strings::t("cmd.debug.opt.durum.name")).unwrap_or("");
    let enabled = bot.set_debug(arg);
    reply_info(
        ctx,
        cmd,
        strings::t("debug_cmd.title"),
        if enabled {
            strings::t("debug_cmd.on")
        } else {
            strings::t("debug_cmd.off")
        },
    )
    .await;
}
