/// `/durum`. Input: `bot: &Bot`; `ctx: &Context`; `cmd: &CommandInteraction`. Output: none.
/// Uses: `modal::status_message`, `send_response`. Registered as `run` for `"durum"` in
/// `registration_table.rs`.
async fn cmd_status(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    let response = modal::status_message(&bot.state());
    send_response(ctx, cmd, response).await;
}

/// `/yardim`. Input: `_bot: &Bot` (unused); `ctx: &Context`; `cmd: &CommandInteraction`.
/// Output: none. Uses: `modal::help_message`, `send_response`. Registered as `run` for
/// `"yardim"` in `registration_table.rs`.
async fn cmd_help(_bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    send_response(ctx, cmd, modal::help_message()).await;
}

/// `/ayarlar`. Input: `bot: &Bot`; `ctx: &Context`; `cmd: &CommandInteraction`. Output:
/// none. Uses: `modal::settings_message`, `send_response`. Registered as `run` for
/// `"ayarlar"` in `registration_table.rs`.
async fn cmd_settings(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    let response = modal::settings_message(&bot.state(), true);
    send_response(ctx, cmd, response).await;
}

/// `/zihin [test]`. Input: `bot: &Bot`; `ctx: &Context`; `cmd: &CommandInteraction` (its
/// `test` option selects the diagnostic path). Output: none. Uses: `option_bool`, `defer`,
/// `bot.state()`, `bot.diarist` (`agents.rs`), `memory::trim`, `report_result`,
/// `modal::mind_message`, `send_response`. Registered as `run` for `"zihin"` in
/// `registration_table.rs`.
async fn cmd_mind(bot: &Bot, ctx: &Context, cmd: &CommandInteraction) {
    if option_bool(cmd, strings::t("cmd.zihin.opt.test.name")).unwrap_or(false) {
        defer(ctx, cmd).await;
        let channel = RoomKey::discord(cmd.channel_id.get());
        // diagnostic: feed this channel's recent lines straight to the diarist, so
        // whether the mind pipeline works can be seen without waiting 40 minutes (it was
        // coming back empty on a reasoning model, and without a live log there was no way to tell)
        let mut lines: Vec<String> = {
            let state = bot.state();
            state
                .channel_history
                .get(&channel)
                .map(|hist| hist.iter().rev().take(30).cloned().collect())
                .unwrap_or_default()
        };
        if lines.is_empty() {
            report_result(
                ctx,
                cmd,
                strings::t("mind_test.title"),
                strings::t("mind_test.no_lines"),
            )
            .await;
            return;
        }
        lines.reverse();
        let channel_name = cmd
            .channel_id
            .name(ctx)
            .await
            .unwrap_or_else(|_| cmd.channel_id.to_string());
        let result = bot
            .diarist(lines.join("\n"), "zihin testi", &channel_name)
            .await;
        let description = match result {
            Ok(summary) => strings::t("mind_test.result")
                .replace("{people}", &summary.people.to_string())
                .replace("{topics}", &summary.topics.to_string())
                .replace("{events}", &summary.events.to_string())
                .replace("{chars}", &summary.output_chars.to_string()),
            Err(e) => strings::t("mind_test.failed")
                .replace("{error}", &memory::trim(&e.to_string(), 300)),
        };
        report_result(ctx, cmd, strings::t("mind_test.title"), &description).await;
        return;
    }
    let response = modal::mind_message(&bot.state());
    send_response(ctx, cmd, response).await;
}
