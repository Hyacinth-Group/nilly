fn options_reset() -> Vec<CreateCommandOption> {
    vec![CreateCommandOption::new(
        CommandOptionType::Boolean,
        strings::t("cmd.sifirla.opt.hepsi.name"),
        strings::t("cmd.sifirla.opt.hepsi.description"),
    )]
}

fn options_mind() -> Vec<CreateCommandOption> {
    vec![CreateCommandOption::new(
        CommandOptionType::Boolean,
        strings::t("cmd.zihin.opt.test.name"),
        strings::t("cmd.zihin.opt.test.description"),
    )]
}

fn options_sleep() -> Vec<CreateCommandOption> {
    vec![CreateCommandOption::new(
        CommandOptionType::Integer,
        strings::t("cmd.uyu.opt.saat.name"),
        strings::t("cmd.uyu.opt.saat.description"),
    )
    .min_int_value(1)]
}

fn options_thinking() -> Vec<CreateCommandOption> {
    // choice values ("goster"/"gizle"/"sessiz"/"kapat") are a wire protocol matched by
    // ThinkingMode::from_arg — never translated, see langs/tr.json's file-top note. Only the
    // display labels (first argument) come from strings::t.
    vec![CreateCommandOption::new(
        CommandOptionType::String,
        strings::t("cmd.dusunme.opt.kip.name"),
        strings::t("cmd.dusunme.opt.kip.description"),
    )
    .add_string_choice(strings::t("cmd.dusunme.choice.goster"), "goster")
    .add_string_choice(strings::t("cmd.dusunme.choice.gizle"), "gizle")
    .add_string_choice(strings::t("cmd.dusunme.choice.sessiz"), "sessiz")
    .add_string_choice(strings::t("cmd.dusunme.choice.kapat"), "kapat")]
}

fn options_model() -> Vec<CreateCommandOption> {
    vec![CreateCommandOption::new(
        CommandOptionType::String,
        strings::t("cmd.model.opt.id.name"),
        strings::t("cmd.model.opt.id.description"),
    )]
}

fn options_debug() -> Vec<CreateCommandOption> {
    vec![CreateCommandOption::new(
        CommandOptionType::String,
        strings::t("cmd.debug.opt.durum.name"),
        strings::t("cmd.debug.opt.durum.description"),
    )
    .add_string_choice(strings::t("cmd.debug.choice.ac"), "ac")
    .add_string_choice(strings::t("cmd.debug.choice.kapat"), "kapat")]
}

// ---------- option-reading helpers ----------

fn option_text<'a>(cmd: &'a CommandInteraction, name: &str) -> Option<&'a str> {
    cmd.data
        .options()
        .into_iter()
        .find(|o| o.name == name)
        .and_then(|o| match o.value {
            ResolvedValue::String(s) => Some(s),
            _ => None,
        })
}

fn option_int(cmd: &CommandInteraction, name: &str) -> Option<i64> {
    cmd.data
        .options()
        .into_iter()
        .find(|o| o.name == name)
        .and_then(|o| match o.value {
            ResolvedValue::Integer(n) => Some(n),
            _ => None,
        })
}

fn option_bool(cmd: &CommandInteraction, name: &str) -> Option<bool> {
    cmd.data
        .options()
        .into_iter()
        .find(|o| o.name == name)
        .and_then(|o| match o.value {
            ResolvedValue::Boolean(b) => Some(b),
            _ => None,
        })
}

// ---------- response helpers ----------

// a ready-made `CreateInteractionResponseMessage` (durum/yardim/zihin/ayarlar) goes out as-is
/// Logs response failures without propagating them.
async fn send_response(
    ctx: &Context,
    cmd: &CommandInteraction,
    response: CreateInteractionResponseMessage,
) {
    if let Err(e) = cmd
        .create_response(&ctx.http, CreateInteractionResponse::Message(response))
        .await
    {
        log::warn!("couldn't send command response [{}]: {e}", cmd.data.name);
    }
}

// a short info/acknowledgment embed; most commands use this
async fn reply_info(ctx: &Context, cmd: &CommandInteraction, title: &str, description: &str) {
    send_response(
        ctx,
        cmd,
        CreateInteractionResponseMessage::new()
            .ephemeral(true)
            .embed(modal::info_embed(title, description)),
    )
    .await;
}

// commands that will make a network/model call call this first (so they don't miss the 3s limit)
async fn defer(ctx: &Context, cmd: &CommandInteraction) {
    let msg = CreateInteractionResponseMessage::new().ephemeral(true);
    if let Err(e) = cmd
        .create_response(&ctx.http, CreateInteractionResponse::Defer(msg))
        .await
    {
        log::warn!("couldn't send defer [{}]: {e}", cmd.data.name);
    }
}

// writes the actual result after defer()
async fn report_result(ctx: &Context, cmd: &CommandInteraction, title: &str, description: &str) {
    let body = EditInteractionResponse::new().embed(modal::info_embed(title, description));
    if let Err(e) = cmd.edit_response(&ctx.http, body).await {
        log::warn!("couldn't update result [{}]: {e}", cmd.data.name);
    }
}

// ---------- commands ----------

