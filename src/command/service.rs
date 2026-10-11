use super::{
    read_models::{self, PersonView},
    ActionArgs, ActionError, ActionResult, ActionTarget, Actor, DebugSetting,
};
use crate::platform::types::{MessageKey, PlatformKind, UserKey};
use crate::{memory, strings, Bot};

pub(crate) struct AdminService<'a> {
    bot: &'a Bot,
}
fn model_owner(actor: &Actor, owner: Option<&UserKey>) -> bool {
    match actor {
        Actor::LocalOperator => true,
        Actor::User(user) => owner == Some(user),
    }
}
fn ack(title_key: &'static str, description: impl Into<String>) -> ActionResult {
    ActionResult::Ack {
        title: strings::t(title_key).to_string(),
        description: description.into(),
    }
}
impl<'a> AdminService<'a> {
    pub(crate) fn new(bot: &'a Bot) -> Self {
        Self { bot }
    }
    /// Authorizes the ingress identity and route without touching any platform SDK/cache.
    pub(crate) fn authorize(
        &self,
        actor: &Actor,
        target: &ActionTarget,
        args: &ActionArgs,
    ) -> Result<(), ActionError> {
        if args.requires_room() && target.room.is_none() {
            return Err(ActionError::MissingRoom);
        }
        if args.requires_room()
            && target
                .room
                .as_ref()
                .is_some_and(|room| room.thread.is_some())
        {
            return Err(ActionError::UnsupportedRoute);
        }
        if let ActionArgs::Sleep { hours } = args {
            let option = &super::schema::definition(super::ActionId::Sleep).options[0];
            if option.min.is_some_and(|min| *hours < min)
                || option.max.is_some_and(|max| *hours > max)
                || hours
                    .checked_mul(3600)
                    .and_then(|duration| crate::now_unix().checked_add(duration))
                    .is_none()
            {
                return Err(ActionError::InvalidArgument {
                    key: option.key.to_string(),
                    reason: "outside bounds",
                });
            }
        }
        let registry = self
            .bot
            .adapters
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Actor::User(user) = actor {
            let connection = target
                .connection
                .as_ref()
                .ok_or(ActionError::MissingConnection)?;
            registry
                .authorize(connection, user)
                .map_err(|error| match error {
                    crate::platform::PlatformError::PermissionDenied => {
                        ActionError::PermissionDenied
                    }
                    other => ActionError::Platform(other),
                })?;
        }
        if let Some(room) = &target.room {
            if let Some(connection) = &target.connection {
                let platform =
                    registry
                        .connection_platform(connection)
                        .ok_or(ActionError::Platform(
                            crate::platform::PlatformError::Disconnected,
                        ))?;
                if platform != room.id.platform() {
                    return Err(ActionError::UnboundRoom);
                }
            }
            // Native Discord slash actions historically ignore message-ingress allowlists.
            // Other transports must route only explicitly bound conversations.
            if room.id.platform() != PlatformKind::Discord {
                let connection = target
                    .connection
                    .as_ref()
                    .ok_or(ActionError::MissingConnection)?;
                if !registry.is_bound(connection, room) {
                    return Err(ActionError::UnboundRoom);
                }
            }
        }
        Ok(())
    }
    pub(crate) async fn execute(
        &self,
        actor: &Actor,
        target: &ActionTarget,
        args: ActionArgs,
    ) -> Result<ActionResult, ActionError> {
        self.authorize(actor, target, &args)?;
        match super::schema::definition(args.id()).permission {
            super::schema::Permission::OwnerForModelSetter
                if matches!(&args, ActionArgs::Model { id: Some(_) })
                    && !model_owner(actor, crate::owner_id()) =>
            {
                return Ok(ack("model_cmd.title", strings::t("model_cmd.forbidden")))
            }
            super::schema::Permission::Administrator
            | super::schema::Permission::OwnerForModelSetter => {}
        }
        let bot = self.bot;
        Ok(match args {
            ActionArgs::Status {} => ActionResult::Status(read_models::status(bot)),
            ActionArgs::Help {} => ActionResult::Help(read_models::help()),
            ActionArgs::Settings {} => ActionResult::Settings(read_models::settings(bot)),
            ActionArgs::Mind { test: false } => ActionResult::Mind(read_models::mind(bot)?),
            ActionArgs::Mind { test: true } => {
                let room = target.room.as_ref().ok_or(ActionError::MissingRoom)?;
                let mut lines: Vec<String> = {
                    let state = bot.state();
                    state
                        .channel_history
                        .get(&room.id)
                        .map(|history| history.iter().rev().take(30).cloned().collect())
                        .unwrap_or_default()
                };
                if lines.is_empty() {
                    ack("mind_test.title", strings::t("mind_test.no_lines"))
                } else {
                    lines.reverse();
                    let fallback = room.id.to_string();
                    let channel_name = room.name.as_deref().unwrap_or(&fallback);
                    let result = bot
                        .diarist(
                            lines.join("\n"),
                            strings::t("admin.mind_test.label"),
                            channel_name,
                        )
                        .await;
                    let description = match result {
                        Ok(summary) => strings::t("mind_test.result")
                            .replace("{people}", &summary.people.to_string())
                            .replace("{topics}", &summary.topics.to_string())
                            .replace("{events}", &summary.events.to_string())
                            .replace("{chars}", &summary.output_chars.to_string()),
                        Err(error) => strings::t("mind_test.failed")
                            .replace("{error}", &memory::trim(&error.to_string(), 300)),
                    };
                    ack("mind_test.title", description)
                }
            }
            ActionArgs::Reset { all } => {
                let mut state = bot.state();
                if all {
                    state.chats.clear();
                    state.awaiting_comment.clear();
                    state.busy.clear();
                } else {
                    let room = target.room.as_ref().ok_or(ActionError::MissingRoom)?;
                    state.chats.remove(&room.id);
                    state.awaiting_comment.remove(&room.id);
                    state.busy.remove(&room.id);
                }
                ack(
                    "reset.title",
                    strings::t(if all { "reset.all" } else { "reset.channel" }),
                )
            }
            ActionArgs::News {} => {
                let room = target.room.as_ref().ok_or(ActionError::MissingRoom)?;
                bot.state().chats.remove(&room.id);
                let found = bot.post_news(&room.id).await;
                ack(
                    "news.title",
                    strings::t(if found {
                        "common.sent"
                    } else {
                        "news.not_found"
                    }),
                )
            }
            ActionArgs::Problem {} => {
                let room = target.room.as_ref().ok_or(ActionError::MissingRoom)?;
                bot.state().chats.remove(&room.id);
                bot.post_problem(&room.id).await;
                ack("problem.title", strings::t("common.sent"))
            }
            ActionArgs::Wander {} => {
                bot.wander().await;
                ack("wander.title", strings::t("wander.done"))
            }
            ActionArgs::Prank {} | ActionArgs::Hack {} => {
                let hack = matches!(args, ActionArgs::Hack {});
                let room = target.room.as_ref().ok_or(ActionError::MissingRoom)?;
                bot.state().chats.remove(&room.id);
                bot.run_prank(&room.id, hack).await;
                ack(
                    if hack {
                        "prank.title_hack"
                    } else {
                        "prank.title_saka"
                    },
                    strings::t("common.sent"),
                )
            }
            ActionArgs::Agents {} => {
                bot.profiler().await;
                bot.coach().await;
                let index = memory::refresh_index_result().map_err(ActionError::Memory)?;
                bot.state().index = index;
                ack("agents_cmd.title", strings::t("agents_cmd.done"))
            }
            ActionArgs::Wake {} => {
                bot.wake();
                bot.sleep_transition().await;
                ack("wake.title", strings::t("wake.done"))
            }
            ActionArgs::Sleep { hours } => {
                bot.put_to_sleep(hours);
                bot.sleep_transition().await;
                ack(
                    "sleep_cmd.title",
                    strings::t("sleep_cmd.hours").replace("{saat}", &hours.to_string()),
                )
            }
            ActionArgs::Thinking { mode } => match mode {
                Some(mode) => {
                    bot.state().thinking_mode = mode.core();
                    memory::write_result("dusunme.md", mode.file_value())
                        .map_err(ActionError::Memory)?;
                    ack(
                        "thinking.title",
                        strings::t("thinking.set").replace("{mode}", mode.label()),
                    )
                }
                None => {
                    let mode = bot.state().thinking_mode;
                    ack(
                        "thinking.title",
                        strings::t("thinking.current").replace("{mode}", mode.label()),
                    )
                }
            },
            ActionArgs::Model { id } => match id {
                None => {
                    let model = bot.state().model.clone();
                    ack(
                        "model_cmd.title",
                        strings::t("model_cmd.current").replace("{model}", &model),
                    )
                }
                Some(id) => {
                    // Preserve catalog quirks: only OpenRouter checked, transport failure fails open,
                    // malformed catalog rejects, custom/native Mistral IDs trusted unchanged.
                    if bot.api_url.contains("openrouter") && !bot.model_exists(&id).await {
                        ack("model_cmd.title", strings::t("model_cmd.unknown"))
                    } else {
                        bot.state().model = id.clone();
                        memory::write_result("model.md", &id).map_err(ActionError::Memory)?;
                        ack(
                            "model_cmd.title",
                            strings::t("model_cmd.set").replace("{model}", &id),
                        )
                    }
                }
            },
            ActionArgs::Debug { setting } => {
                let choices = super::schema::definition(super::ActionId::Debug).options[0].choices;
                let arg = match setting {
                    Some(DebugSetting::On) => choices[0].value,
                    Some(DebugSetting::Off) => choices[1].value,
                    None => "",
                };
                let enabled = bot.set_debug(arg).map_err(ActionError::Memory)?;
                ack(
                    "debug_cmd.title",
                    strings::t(if enabled {
                        "debug_cmd.on"
                    } else {
                        "debug_cmd.off"
                    }),
                )
            }
        })
    }
    pub(crate) fn person(&self, id: &UserKey) -> Result<PersonView, ActionError> {
        memory::read_person_result(id)
            .map(PersonView::from)
            .map_err(ActionError::Memory)
    }
    pub(crate) fn topics(&self) -> Result<Vec<(String, String)>, ActionError> {
        memory::topic_summaries_result().map_err(ActionError::Memory)
    }
    pub(crate) fn events(&self) -> Result<Vec<(String, Vec<String>)>, ActionError> {
        memory::event_months_result(3).map_err(ActionError::Memory)
    }
    pub(crate) fn thought(&self, message: &MessageKey) -> Option<String> {
        self.bot.state().thought_store.get(message).cloned()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Bot {
        Bot {
            state: std::sync::Mutex::new(crate::State::default()),
            adapters: std::sync::Mutex::new(crate::platform::ConnectionRegistry::default()),
            http: reqwest::Client::new(),
            api_url: "https://custom.invalid/chat/completions".into(),
            key: String::new(),
            news_channel: None,
            firecrawl: None,
            guild_id: None,
            allowed_channels: None,
            debug_channel: None,
            image_analysis: false,
            stealth: false,
            reasoning_mandatory_models: std::sync::Mutex::new(std::collections::HashSet::new()),
        }
    }
    #[test]
    fn trusted_operator_has_no_room_requirement_for_shared_brain_reads() {
        let bot = fixture();
        let service = AdminService::new(&bot);
        let target = ActionTarget::default();
        assert!(service
            .authorize(&Actor::LocalOperator, &target, &ActionArgs::Status {})
            .is_ok());
        assert!(service
            .authorize(
                &Actor::LocalOperator,
                &target,
                &ActionArgs::Reset { all: true }
            )
            .is_ok());
        assert!(matches!(
            service.authorize(
                &Actor::LocalOperator,
                &target,
                &ActionArgs::Reset { all: false }
            ),
            Err(ActionError::MissingRoom)
        ));
        assert!(matches!(
            service.authorize(
                &Actor::User(UserKey::discord(1)),
                &target,
                &ActionArgs::Status {}
            ),
            Err(ActionError::MissingConnection)
        ));
        assert!(matches!(
            service.authorize(
                &Actor::LocalOperator,
                &target,
                &ActionArgs::Sleep { hours: 0 }
            ),
            Err(ActionError::InvalidArgument { .. })
        ));
        assert!(matches!(
            service.authorize(
                &Actor::LocalOperator,
                &target,
                &ActionArgs::Sleep { hours: i64::MAX }
            ),
            Err(ActionError::InvalidArgument { .. })
        ));
    }
    #[tokio::test]
    async fn reset_scopes_preserve_history_and_other_room_until_all() {
        use crate::platform::types::{RoomKey, RoomRef};
        let bot = fixture();
        let first = RoomKey::discord(7001);
        let second = RoomKey::discord(7002);
        {
            let mut state = bot.state();
            for room in [&first, &second] {
                state.chats.insert(room.clone(), crate::Chat::default());
                state
                    .awaiting_comment
                    .insert(room.clone(), std::time::Instant::now());
                state.busy.insert(room.clone());
                state.channel_history.insert(
                    room.clone(),
                    std::collections::VecDeque::from(["retained".into()]),
                );
            }
        }
        let service = AdminService::new(&bot);
        let target = ActionTarget {
            connection: None,
            room: Some(RoomRef::from_key(first.clone())),
        };
        let result = service
            .execute(
                &Actor::LocalOperator,
                &target,
                ActionArgs::Reset { all: false },
            )
            .await
            .unwrap();
        assert!(
            matches!(result, ActionResult::Ack { description, .. } if description == strings::t("reset.channel"))
        );
        {
            let state = bot.state();
            assert!(!state.chats.contains_key(&first));
            assert!(!state.awaiting_comment.contains_key(&first));
            assert!(!state.busy.contains(&first));
            assert!(state.chats.contains_key(&second));
            assert!(state.awaiting_comment.contains_key(&second));
            assert!(state.busy.contains(&second));
            assert_eq!(state.channel_history.len(), 2);
        }
        service
            .execute(
                &Actor::LocalOperator,
                &ActionTarget::default(),
                ActionArgs::Reset { all: true },
            )
            .await
            .unwrap();
        let state = bot.state();
        assert!(state.chats.is_empty());
        assert!(state.awaiting_comment.is_empty());
        assert!(state.busy.is_empty());
        assert_eq!(state.channel_history.len(), 2);
    }
    #[tokio::test]
    async fn diagnostic_without_history_returns_original_ack_without_model_call() {
        let bot = fixture();
        let target = ActionTarget {
            connection: None,
            room: Some(crate::platform::types::RoomRef::from_key(
                crate::platform::types::RoomKey::discord(7010),
            )),
        };
        let result = AdminService::new(&bot)
            .execute(
                &Actor::LocalOperator,
                &target,
                ActionArgs::Mind { test: true },
            )
            .await
            .unwrap();
        assert!(matches!(result, ActionResult::Ack { title, description }
            if title == strings::t("mind_test.title") && description == strings::t("mind_test.no_lines")));
        assert_eq!(bot.state().metrics.calls, 0);
    }
    #[tokio::test]
    async fn settings_queries_and_toggles_are_typed_without_model_calls() {
        memory::test::ensure_test_db();
        let bot = fixture();
        let service = AdminService::new(&bot);
        let actor = Actor::LocalOperator;
        let target = ActionTarget::default();
        let result = service
            .execute(&actor, &target, ActionArgs::Thinking { mode: None })
            .await
            .unwrap();
        assert!(matches!(result, ActionResult::Ack { description, .. }
            if description == strings::t("thinking.current").replace("{mode}", crate::ThinkingMode::Show.label())));
        for setting in [None, Some(DebugSetting::Off), Some(DebugSetting::On), None] {
            service
                .execute(&actor, &target, ActionArgs::Debug { setting })
                .await
                .unwrap();
            assert_eq!(
                memory::read_result("debug.md").unwrap().unwrap(),
                if bot.state().debug { "acik" } else { "kapali" }
            );
        }
        assert!(!bot.state().debug);
        assert_eq!(bot.state().metrics.calls, 0);
    }
    #[tokio::test]
    async fn custom_provider_model_ids_are_trusted_verbatim_and_thinking_off_persists() {
        memory::test::ensure_test_db();
        let bot = fixture();
        let service = AdminService::new(&bot);
        let actor = Actor::LocalOperator;
        let target = ActionTarget::default();
        let id = " custom/provider-model ".to_string();
        let result = service
            .execute(
                &actor,
                &target,
                ActionArgs::Model {
                    id: Some(id.clone()),
                },
            )
            .await
            .unwrap();
        assert!(matches!(result, ActionResult::Ack { description, .. }
            if description == strings::t("model_cmd.set").replace("{model}", &id)));
        assert_eq!(bot.state().model, id);
        assert_eq!(memory::read_result("model.md").unwrap(), Some(id.clone()));
        let query = service
            .execute(&actor, &target, ActionArgs::Model { id: None })
            .await
            .unwrap();
        assert!(matches!(query, ActionResult::Ack { description, .. }
            if description == strings::t("model_cmd.current").replace("{model}", &id)));
        service
            .execute(
                &actor,
                &target,
                ActionArgs::Thinking {
                    mode: Some(super::super::ThinkingSetting::Off),
                },
            )
            .await
            .unwrap();
        assert_eq!(bot.state().thinking_mode, crate::ThinkingMode::Off);
        assert_eq!(
            memory::read_result("dusunme.md").unwrap(),
            Some("kapali".into())
        );
        assert_eq!(bot.state().metrics.calls, 0);
    }
    #[test]
    fn ownership_is_qualified_and_local_operator_is_explicit() {
        let owner = UserKey::discord(42);
        let same_number = UserKey::new(PlatformKind::Telegram, &["42"]).unwrap();
        assert!(model_owner(&Actor::LocalOperator, None));
        assert!(model_owner(&Actor::User(owner.clone()), Some(&owner)));
        assert!(!model_owner(&Actor::User(same_number), Some(&owner)));
        assert!(!model_owner(
            &Actor::User(UserKey::discord(9)),
            Some(&owner)
        ));
        assert!(!model_owner(&Actor::User(owner), None));
    }
}
