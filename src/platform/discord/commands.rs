//! Discord owns registration, interaction tokens, deadlines and native private UI.
use super::modal;
use crate::command::parser::BorrowedActionValue;
use crate::command::schema::{ActionDefinition, OptionKind};
use crate::command::{
    self, ActionArgs, ActionError, ActionId, ActionResult, ActionTarget, Actor, AdminService,
};
use crate::platform::types::{
    ConnectionId, MessageKey, RoomKey, RoomKind, RoomRef, SpaceKey, UserKey,
};
use crate::{strings, Bot, ThinkingMode, THOUGHT_BUTTON};
use serenity::all::*;
use std::sync::LazyLock;

struct CommandDefinition {
    name: &'static str,
    description: &'static str,
    action: &'static ActionDefinition,
}
impl CommandDefinition {
    fn options(&self) -> Vec<CreateCommandOption> {
        self.action
            .options
            .iter()
            .map(|option| {
                let kind = match option.kind {
                    OptionKind::Boolean => CommandOptionType::Boolean,
                    OptionKind::Integer => CommandOptionType::Integer,
                    OptionKind::String => CommandOptionType::String,
                };
                let mut builder = CreateCommandOption::new(
                    kind,
                    strings::t(option.name_key),
                    strings::t(option.description_key),
                );
                if let Some(min) = option.min {
                    builder = builder.min_int_value(
                        u64::try_from(min).expect("Discord integer minimum must be nonnegative"),
                    );
                }
                if let Some(max) = option.max {
                    builder = builder.max_int_value(
                        u64::try_from(max).expect("Discord integer maximum must be nonnegative"),
                    );
                }
                for choice in option.choices {
                    builder = builder.add_string_choice(strings::t(choice.name_key), choice.value);
                }
                builder
            })
            .collect()
    }
    fn registration(&self) -> CreateCommand {
        CreateCommand::new(self.name)
            .description(self.description)
            .set_options(self.options())
    }
}
static DEFINITIONS: LazyLock<Vec<CommandDefinition>> = LazyLock::new(|| {
    command::schema::definitions()
        .iter()
        .map(|action| CommandDefinition {
            name: strings::t(action.name_key),
            description: strings::t(action.description_key),
            action,
        })
        .collect()
});
fn definitions() -> &'static [CommandDefinition] {
    &DEFINITIONS
}
pub(crate) fn registrations() -> Vec<CreateCommand> {
    definitions()
        .iter()
        .map(CommandDefinition::registration)
        .collect()
}

fn target(connection: &ConnectionId, channel: ChannelId, guild: Option<GuildId>) -> ActionTarget {
    ActionTarget {
        connection: Some(connection.clone()),
        room: Some(RoomRef {
            id: RoomKey::discord(channel.get()),
            space: guild.map(|id| SpaceKey::discord(id.get())),
            kind: if guild.is_some() {
                RoomKind::Channel
            } else {
                RoomKind::Direct
            },
            name: None,
            thread: None,
        }),
    }
}
fn arguments(
    definition: &CommandDefinition,
    cmd: &CommandInteraction,
) -> Result<ActionArgs, ActionError> {
    for option in &cmd.data.options {
        if !matches!(
            option.value,
            CommandDataOptionValue::Boolean(_)
                | CommandDataOptionValue::Integer(_)
                | CommandDataOptionValue::String(_)
        ) {
            return Err(ActionError::InvalidArgument {
                key: option.name.clone(),
                reason: "type",
            });
        }
    }
    let values = cmd.data.options.iter().map(|option| {
        let value = match &option.value {
            CommandDataOptionValue::Boolean(value) => BorrowedActionValue::Boolean(*value),
            CommandDataOptionValue::Integer(value) => BorrowedActionValue::Integer(*value),
            CommandDataOptionValue::String(value) => BorrowedActionValue::String(value),
            _ => unreachable!("native argument types were checked above"),
        };
        (option.name.as_str(), value)
    });
    command::parser::parse_native_values(definition.action.id, values)
}
fn response(result: &ActionResult) -> CreateInteractionResponseMessage {
    match result {
        ActionResult::Status(view) => modal::status_message_view(view),
        ActionResult::Help(_) => modal::help_message(),
        ActionResult::Mind(view) => modal::mind_message_view(view),
        ActionResult::Settings(view) => modal::settings_message_view(view, true),
        ActionResult::Ack { title, description } => CreateInteractionResponseMessage::new()
            .ephemeral(true)
            .embed(modal::info_embed(title, description)),
    }
}
fn error_response(error: &ActionError) -> CreateInteractionResponseMessage {
    CreateInteractionResponseMessage::new()
        .ephemeral(true)
        .embed(modal::info_embed(
            strings::t("admin.error.title"),
            &command::render::localized_description(error),
        ))
}
async fn send_response(
    ctx: &Context,
    cmd: &CommandInteraction,
    body: CreateInteractionResponseMessage,
) {
    if let Err(error) = cmd
        .create_response(&ctx.http, CreateInteractionResponse::Message(body))
        .await
    {
        log::warn!(
            "couldn't send command response [{}]: {error}",
            cmd.data.name
        );
    }
}
async fn slash(bot: &Bot, connection: &ConnectionId, ctx: &Context, cmd: &CommandInteraction) {
    let Some(definition) = definitions()
        .iter()
        .find(|definition| definition.name == cmd.data.name)
    else {
        log::warn!("unknown slash command: {}", cmd.data.name);
        return;
    };
    let args = match arguments(definition, cmd) {
        Ok(args) => args,
        Err(error) => {
            send_response(ctx, cmd, error_response(&error)).await;
            return;
        }
    };
    let actor = Actor::User(UserKey::discord(cmd.user.id.get()));
    let mut target = target(connection, cmd.channel_id, cmd.guild_id);
    // Model queries and forbidden setters never show a deferred response.
    let deferred = definition.action.should_defer(&args)
        && (!matches!(&args, ActionArgs::Model { id: Some(_) })
            || matches!(&actor, Actor::User(user) if crate::is_owner(user)));
    if deferred {
        if let Err(error) = cmd
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Defer(
                    CreateInteractionResponseMessage::new().ephemeral(true),
                ),
            )
            .await
        {
            log::warn!("couldn't send defer [{}]: {error}", cmd.data.name);
        }
    }
    if matches!(args, ActionArgs::Mind { test: true }) {
        if let Some(room) = &mut target.room {
            room.name = Some(
                cmd.channel_id
                    .name(ctx)
                    .await
                    .unwrap_or_else(|_| cmd.channel_id.to_string()),
            );
        }
    }
    let service = AdminService::new(bot);
    // Static async dispatch: no boxed native callback table or synthetic interaction.
    let result = match definition.action.id {
        ActionId::Status
        | ActionId::Help
        | ActionId::Mind
        | ActionId::Settings
        | ActionId::Reset
        | ActionId::News
        | ActionId::Problem
        | ActionId::Wander
        | ActionId::Prank
        | ActionId::Hack
        | ActionId::Agents
        | ActionId::Wake
        | ActionId::Sleep
        | ActionId::Thinking
        | ActionId::Model
        | ActionId::Debug => service.execute(&actor, &target, args).await,
    };
    if deferred {
        match result {
            Ok(ActionResult::Ack { title, description }) => {
                if let Err(error) = cmd
                    .edit_response(
                        &ctx.http,
                        EditInteractionResponse::new()
                            .embed(modal::info_embed(&title, &description)),
                    )
                    .await
                {
                    log::warn!("couldn't update result [{}]: {error}", cmd.data.name);
                }
            }
            Err(error) => {
                let body = EditInteractionResponse::new().embed(modal::info_embed(
                    strings::t("admin.error.title"),
                    &command::render::localized_description(&error),
                ));
                if let Err(error) = cmd.edit_response(&ctx.http, body).await {
                    log::warn!("couldn't update result [{}]: {error}", cmd.data.name);
                }
            }
            Ok(_) => unreachable!("only acknowledgement actions defer"),
        }
    } else {
        let body = match result {
            Ok(result) => response(&result),
            Err(error) => error_response(&error),
        };
        send_response(ctx, cmd, body).await;
    }
}

async fn setting_button(
    bot: &Bot,
    connection: &ConnectionId,
    ctx: &Context,
    component: ComponentInteraction,
) {
    let id = &component.data.custom_id;
    let args = if let Some(mode) = id.strip_prefix(modal::SETTING_THINKING) {
        ThinkingMode::from_arg(mode).map(|mode| ActionArgs::Thinking {
            mode: Some(mode.into()),
        })
    } else if id == modal::SETTING_DEBUG {
        Some(ActionArgs::Debug { setting: None })
    } else if id == modal::SETTING_WAKE {
        Some(ActionArgs::Wake {})
    } else if id == modal::SETTING_SLEEP {
        Some(ActionArgs::Sleep {
            hours: command::schema::default_sleep_hours(),
        })
    } else {
        return;
    };
    let actor = Actor::User(UserKey::discord(component.user.id.get()));
    let target = target(connection, component.channel_id, component.guild_id);
    let deferred = args.as_ref().is_some_and(|args| {
        command::schema::definitions()
            .iter()
            .find(|definition| definition.id == args.id())
            .is_some_and(|definition| definition.should_defer(args))
    });
    if deferred {
        if let Err(error) = component
            .create_response(&ctx.http, CreateInteractionResponse::Acknowledge)
            .await
        {
            log::warn!("couldn't defer settings panel: {error}");
        }
    }
    let service = AdminService::new(bot);
    if let Some(args) = args {
        if let Err(error) = service.execute(&actor, &target, args).await {
            log::warn!("couldn't apply setting: {error}");
            if deferred {
                let _ = component
                    .edit_response(
                        &ctx.http,
                        EditInteractionResponse::new().embed(modal::info_embed(
                            strings::t("admin.error.title"),
                            &command::render::localized_description(&error),
                        )),
                    )
                    .await;
            } else {
                let _ = component
                    .create_response(
                        &ctx.http,
                        CreateInteractionResponse::Message(error_response(&error)),
                    )
                    .await;
            }
            return;
        }
    }
    log::info!("setting [{}]: {id}", component.user.id);
    let view = match service
        .execute(&actor, &target, ActionArgs::Settings {})
        .await
    {
        Ok(ActionResult::Settings(view)) => view,
        Err(error) => {
            log::warn!("couldn't read settings panel: {error}");
            return;
        }
        Ok(_) => unreachable!("settings action returns a settings view"),
    };
    if deferred {
        if let Err(error) = component
            .edit_response(
                &ctx.http,
                EditInteractionResponse::new()
                    .embed(modal::settings_embed(&view))
                    .components(modal::settings_components(&view)),
            )
            .await
        {
            log::warn!("couldn't refresh settings panel: {error}");
        }
    } else if let Err(error) = component
        .create_response(
            &ctx.http,
            CreateInteractionResponse::UpdateMessage(modal::settings_message_view(&view, false)),
        )
        .await
    {
        log::warn!("couldn't refresh settings panel: {error}");
    }
}

async fn thought_button(bot: &Bot, ctx: &Context, component: ComponentInteraction) {
    let message = MessageKey::discord(component.message.id.get());
    let content = AdminService::new(bot)
        .thought(&message)
        .map(|thought| super::render::thought_display(&thought))
        .unwrap_or_else(|| strings::t("thought.not_found").to_string());
    if let Err(error) = component
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .ephemeral(true)
                    .content(content),
            ),
        )
        .await
    {
        log::warn!("couldn't send thought ephemeral reply: {error}");
    }
}

pub(crate) async fn interaction_create(
    bot: &Bot,
    connection: &ConnectionId,
    ctx: &Context,
    interaction: Interaction,
) {
    // No registered controls in stealth: stale interactions remain unanswered.
    if bot.stealth {
        return;
    }
    match interaction {
        Interaction::Command(cmd) => slash(bot, connection, ctx, &cmd).await,
        Interaction::Modal(interaction) => {
            let _ = interaction
                .create_response(
                    &ctx.http,
                    CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new()
                            .ephemeral(true)
                            .content(strings::t("modal.display_only")),
                    ),
                )
                .await;
        }
        Interaction::Component(component) => {
            if component.data.custom_id == THOUGHT_BUTTON {
                thought_button(bot, ctx, component).await;
                return;
            }
            if component.data.custom_id.starts_with("setting_") {
                setting_button(bot, connection, ctx, component).await;
                return;
            }
            let service = AdminService::new(bot);
            let detail = match component.data.custom_id.as_str() {
                modal::MIND_TOPICS => service
                    .topics()
                    .map(|topics| modal::topics_modal(&topics))
                    .map(Some),
                modal::MIND_EVENTS => service
                    .events()
                    .map(|events| modal::events_modal(&events))
                    .map(Some),
                modal::MIND_SUMMARY => {
                    let actor = Actor::User(UserKey::discord(component.user.id.get()));
                    match service
                        .execute(
                            &actor,
                            &target(connection, component.channel_id, component.guild_id),
                            ActionArgs::Status {},
                        )
                        .await
                    {
                        Ok(ActionResult::Status(view)) => Ok(Some(modal::summary_modal(&view))),
                        Err(error) => Err(error),
                        Ok(_) => unreachable!("status action returns a status view"),
                    }
                }
                modal::MIND_PERSON_PICK => {
                    let ComponentInteractionDataKind::StringSelect { values } =
                        &component.data.kind
                    else {
                        return;
                    };
                    let Some(id) = values
                        .first()
                        .and_then(|value| value.parse::<UserKey>().ok())
                    else {
                        return;
                    };
                    service
                        .person(&id)
                        .map(|person| Some(modal::person_modal(&person)))
                }
                _ => Ok(None),
            };
            let response = match detail {
                Ok(Some(modal)) => CreateInteractionResponse::Modal(modal),
                Ok(None) => return,
                Err(error) => CreateInteractionResponse::Message(error_response(&error)),
            };
            if let Err(error) = component.create_response(&ctx.http, response).await {
                log::warn!("couldn't send detail modal: {error}");
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn native_command_target_uses_its_actual_connection_and_room_scope() {
        let connection = ConnectionId::new("second-discord");
        let target = target(&connection, ChannelId::new(7), Some(GuildId::new(9)));
        assert_eq!(target.connection, Some(connection));
        let room = target.room.unwrap();
        assert_eq!(room.id, RoomKey::discord(7));
        assert_eq!(room.space, Some(SpaceKey::discord(9)));
        assert_eq!(room.kind, RoomKind::Channel);
        assert!(room.thread.is_none());
    }

    /// Verifies `definitions()` has no duplicate command names (Discord would reject a duplicate registration).
    #[test]
    fn table_names_are_unique() {
        let mut names: Vec<&str> = definitions().iter().map(|k| k.name).collect();
        let before = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), before, "duplicate name in the command table");
    }

    /// Verifies every command's `options` builder in `definitions()` runs without panicking.
    #[test]
    fn options_dont_panic() {
        for k in definitions() {
            let _ = k.options();
        }
    }

    /// Verifies `/dusunme`'s slash-option choice values exactly match what `ThinkingMode::from_arg` recognizes.
    #[test]
    fn thinking_mode_options_match_from_arg() {
        // the slash option values (goster/gizle/sessiz/kapat — Discord-facing, stay
        // Turkish) must exactly match the strings ThinkingMode::from_arg recognizes,
        // or the selected mode is silently never applied
        for value in ["goster", "gizle", "sessiz", "kapat"] {
            assert!(
                ThinkingMode::from_arg(value).is_some(),
                "from_arg does not recognize {value}"
            );
        }
    }

    // Serialized builders capture Discord defaults too: optional arguments, no
    // autocomplete, and no registration-time owner/DM permission restrictions.
    #[test]
    fn discord_registration_wire_shape() {
        use crate::lang::Lang;
        use serde_json::{json, Value};

        let (names, option_names) = match Lang::current() {
            Lang::En => (
                [
                    "status", "help", "mind", "settings", "reset", "news", "problem", "wander",
                    "prank", "hack", "agents", "wake", "sleep", "thinking", "model", "debug",
                ],
                ["test", "all", "hours", "mode", "id", "state"],
            ),
            Lang::Tr => (
                [
                    "durum", "yardim", "zihin", "ayarlar", "sifirla", "haber", "sorun", "gez",
                    "saka", "hack", "ajanlar", "uyan", "uyu", "dusunme", "model", "debug",
                ],
                ["test", "hepsi", "saat", "kip", "id", "durum"],
            ),
        };
        let description_keys = [
            "cmd.durum.description",
            "cmd.yardim.description",
            "cmd.zihin.description",
            "cmd.ayarlar.description",
            "cmd.sifirla.description",
            "cmd.haber.description",
            "cmd.sorun.description",
            "cmd.gez.description",
            "cmd.saka.description",
            "cmd.hack.description",
            "cmd.ajanlar.description",
            "cmd.uyan.description",
            "cmd.uyu.description",
            "cmd.dusunme.description",
            "cmd.model.description",
            "cmd.debug.description",
        ];
        let option = |kind, name, description| {
            json!({
                "type": kind,
                "name": name,
                "description": description,
                "required": false,
                "choices": [],
                "options": [],
                "channel_types": [],
                "min_value": null,
                "max_value": null,
                "min_length": null,
                "max_length": null,
                "autocomplete": false,
            })
        };
        let mut expected_options: Vec<Value> = vec![json!([]); 16];
        expected_options[2] = json!([option(
            5,
            option_names[0],
            strings::t("cmd.zihin.opt.test.description"),
        )]);
        expected_options[4] = json!([option(
            5,
            option_names[1],
            strings::t("cmd.sifirla.opt.hepsi.description"),
        )]);
        let mut sleep = option(
            4,
            option_names[2],
            strings::t("cmd.uyu.opt.saat.description"),
        );
        sleep["min_value"] = json!(1);
        sleep["max_value"] = json!(24);
        expected_options[12] = json!([sleep]);
        let mut thinking = option(
            3,
            option_names[3],
            strings::t("cmd.dusunme.opt.kip.description"),
        );
        thinking["choices"] = json!([
            {"name": strings::t("cmd.dusunme.choice.goster"), "value": "goster"},
            {"name": strings::t("cmd.dusunme.choice.gizle"), "value": "gizle"},
            {"name": strings::t("cmd.dusunme.choice.sessiz"), "value": "sessiz"},
            {"name": strings::t("cmd.dusunme.choice.kapat"), "value": "kapat"},
        ]);
        expected_options[13] = json!([thinking]);
        expected_options[14] = json!([option(
            3,
            option_names[4],
            strings::t("cmd.model.opt.id.description"),
        )]);
        let mut debug = option(
            3,
            option_names[5],
            strings::t("cmd.debug.opt.durum.description"),
        );
        debug["choices"] = json!([
            {"name": strings::t("cmd.debug.choice.ac"), "value": "ac"},
            {"name": strings::t("cmd.debug.choice.kapat"), "value": "kapat"},
        ]);
        expected_options[15] = json!([debug]);

        assert_eq!(definitions().len(), 16);
        for (index, definition) in definitions().iter().enumerate() {
            let registration = CreateCommand::new(definition.name)
                .description(definition.description)
                .set_options(definition.options());
            assert_eq!(
                serde_json::to_value(registration).unwrap(),
                json!({
                    "name": names[index],
                    "description": strings::t(description_keys[index]),
                    "name_localizations": {},
                    "description_localizations": {},
                    "options": expected_options[index],
                    "nsfw": false,
                }),
                "registration at index {index}",
            );
        }
    }
}
