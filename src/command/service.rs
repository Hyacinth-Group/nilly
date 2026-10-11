use super::{
    read_models::{self, PersonView},
    ActionArgs, ActionError, ActionResult, ActionTarget, Actor, DebugSetting,
};
use crate::platform::types::{MessageKey, PlatformKind, UserKey};
use crate::{memory, strings, Bot};

pub(crate) struct AdminService<'a> {
    bot: &'a Bot,
    owned: Option<std::sync::Arc<Bot>>,
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
        Self { bot, owned: None }
    }
    pub(crate) fn new_owned(bot: &'a std::sync::Arc<Bot>) -> Self {
        Self {
            bot,
            owned: Some(bot.clone()),
        }
    }
    pub(crate) fn model_validation_supported(&self) -> bool {
        self.bot.model_catalogue_url().is_some()
    }

    /// Only owned ingress can move synchronous database work off the async executor.
    async fn synchronous<T, F>(&self, operation: F) -> Result<T, ActionError>
    where
        T: Send + 'static,
        F: FnOnce(&Bot) -> Result<T, ActionError> + Send + 'static,
    {
        static WORKERS: std::sync::LazyLock<std::sync::Arc<tokio::sync::Semaphore>> =
            std::sync::LazyLock::new(|| std::sync::Arc::new(tokio::sync::Semaphore::new(2)));
        let Some(bot) = self.owned.clone() else {
            return operation(self.bot);
        };
        let permit = WORKERS
            .clone()
            .acquire_owned()
            .await
            .map_err(|error| ActionError::Operation(error.into()))?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            operation(&bot)
        })
        .await
        .map_err(|error| ActionError::Operation(error.into()))?
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
        let mut registry = self
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
            if let Some(connection) = &target.connection {
                if room.id.platform() == PlatformKind::Discord {
                    // Actual native command targets are trusted independently of learning policy.
                    registry
                        .bind(connection, room.clone())
                        .map_err(ActionError::Platform)?;
                } else if !registry.allows(connection, room) || !registry.is_bound(connection, room)
                {
                    return Err(ActionError::UnboundRoom);
                }
            } else if !matches!(actor, Actor::LocalOperator) {
                return Err(ActionError::MissingConnection);
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
                return Err(ActionError::PermissionDenied);
            }
            super::schema::Permission::Administrator
            | super::schema::Permission::OwnerForModelSetter => {}
        }
        let bot = self.bot;
        let _busy_guard = if matches!(
            args,
            ActionArgs::News {}
                | ActionArgs::Problem {}
                | ActionArgs::Prank {}
                | ActionArgs::Hack {}
        ) {
            let channel = target
                .room
                .as_ref()
                .ok_or(ActionError::MissingRoom)?
                .conversation();
            if !bot.state().busy.insert(channel.clone()) {
                return Err(ActionError::Busy);
            }
            Some(crate::BusyGuard {
                state: &bot.state,
                channel,
            })
        } else {
            None
        };
        Ok(match args {
            ActionArgs::Status {} => {
                self.synchronous(|bot| Ok(ActionResult::Status(read_models::status(bot))))
                    .await?
            }
            ActionArgs::Help {} => ActionResult::Help(read_models::help()),
            ActionArgs::Settings {} => {
                self.synchronous(|bot| Ok(ActionResult::Settings(read_models::settings(bot))))
                    .await?
            }
            ActionArgs::Mind { test: false } => {
                self.synchronous(|bot| read_models::mind(bot).map(ActionResult::Mind))
                    .await?
            }
            ActionArgs::Mind { test: true } => {
                let room = target.room.as_ref().ok_or(ActionError::MissingRoom)?;
                let mut lines: Vec<String> = {
                    let state = bot.state();
                    state
                        .channel_history
                        .get(&room.conversation())
                        .map(|history| history.iter().rev().take(30).cloned().collect())
                        .unwrap_or_default()
                };
                if lines.is_empty() {
                    ack("mind_test.title", strings::t("mind_test.no_lines"))
                } else {
                    lines.reverse();
                    let fallback = room.id.to_string();
                    let observed = {
                        let state = bot.state();
                        crate::channel_observed_users(&state, &room.conversation(), 30)
                    };
                    let channel_name = room.name.as_deref().unwrap_or(&fallback);
                    let result = bot
                        .diarist(
                            lines.join("\n"),
                            strings::t("admin.mind_test.label"),
                            channel_name,
                            &observed,
                        )
                        .await;
                    let summary = result.map_err(ActionError::Operation)?;
                    let description = strings::t("mind_test.result")
                        .replace("{people}", &summary.people.to_string())
                        .replace("{topics}", &summary.topics.to_string())
                        .replace("{events}", &summary.events.to_string())
                        .replace("{chars}", &summary.output_chars.to_string());
                    ack("mind_test.title", description)
                }
            }
            ActionArgs::Reset { all } => {
                let mut state = bot.state();
                if all {
                    if !state.busy.is_empty() {
                        return Err(ActionError::Busy);
                    }
                    state.chats.clear();
                    state.awaiting_comment.clear();
                } else {
                    let room = target.room.as_ref().ok_or(ActionError::MissingRoom)?;
                    if state.busy.contains(&room.conversation()) {
                        return Err(ActionError::Busy);
                    }
                    state.chats.remove(&room.conversation());
                    state.awaiting_comment.remove(&room.conversation());
                }
                ack(
                    "reset.title",
                    strings::t(if all { "reset.all" } else { "reset.channel" }),
                )
            }
            ActionArgs::News {} => {
                let room = target.room.as_ref().ok_or(ActionError::MissingRoom)?;
                let outcome = bot
                    .post_news(&room.conversation())
                    .await
                    .map_err(ActionError::Operation)?;
                ack(
                    "news.title",
                    strings::t(match outcome {
                        Some(true) => "common.sent",
                        Some(false) => "admin.action.silent",
                        None => "news.not_found",
                    }),
                )
            }
            ActionArgs::Problem {} => {
                let room = target.room.as_ref().ok_or(ActionError::MissingRoom)?;
                let delivered = bot
                    .post_problem(&room.conversation())
                    .await
                    .map_err(ActionError::Operation)?;
                ack(
                    "problem.title",
                    strings::t(if delivered {
                        "common.sent"
                    } else {
                        "admin.action.silent"
                    }),
                )
            }
            ActionArgs::Wander {} => {
                let changed = bot.wander().await.map_err(ActionError::Operation)?;
                ack(
                    "wander.title",
                    strings::t(if changed {
                        "wander.done"
                    } else {
                        "admin.action.no_input"
                    }),
                )
            }
            ActionArgs::Prank {} | ActionArgs::Hack {} => {
                let hack = matches!(args, ActionArgs::Hack {});
                let room = target.room.as_ref().ok_or(ActionError::MissingRoom)?;
                let delivered = bot
                    .run_prank(&room.conversation(), hack)
                    .await
                    .map_err(ActionError::Operation)?;
                ack(
                    if hack {
                        "prank.title_hack"
                    } else {
                        "prank.title_saka"
                    },
                    strings::t(if delivered {
                        "common.sent"
                    } else {
                        "admin.action.silent"
                    }),
                )
            }
            ActionArgs::Agents {} => {
                let profiled = bot.profiler().await.map_err(ActionError::Operation)?;
                let coached = bot.coach().await.map_err(ActionError::Operation)?;
                ack(
                    "agents_cmd.title",
                    strings::t(if profiled || coached {
                        "agents_cmd.done"
                    } else {
                        "admin.action.no_input"
                    }),
                )
            }
            ActionArgs::Wake {} => {
                bot.wake();
                let (changed, delivered) = bot
                    .sleep_transition()
                    .await
                    .map_err(ActionError::Operation)?;
                ack(
                    "wake.title",
                    strings::t(if delivered {
                        "admin.action.woke_delivered"
                    } else if changed {
                        "admin.action.woke_without_message"
                    } else {
                        "admin.action.no_transition"
                    }),
                )
            }
            ActionArgs::Sleep { hours } => {
                bot.put_to_sleep(hours);
                let (changed, _) = bot
                    .sleep_transition()
                    .await
                    .map_err(ActionError::Operation)?;
                ack(
                    "sleep_cmd.title",
                    if changed {
                        strings::t("sleep_cmd.hours").replace("{saat}", &hours.to_string())
                    } else {
                        strings::t("admin.action.sleep_extended")
                            .replace("{hours}", &hours.to_string())
                    },
                )
            }
            ActionArgs::Thinking { mode } => match mode {
                Some(mode) => {
                    self.synchronous(move |bot| {
                        memory::set_thinking_mode(bot, mode.core()).map_err(ActionError::Memory)
                    })
                    .await?;
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
                    self.validate_model(&id).await?;
                    self.synchronous(move |bot| {
                        memory::set_model_override(bot, &id, true).map_err(ActionError::Memory)?;
                        Ok(ack(
                            "model_cmd.title",
                            strings::t("model_cmd.set").replace("{model}", &id),
                        ))
                    })
                    .await?
                }
            },
            ActionArgs::Debug { setting } => {
                let requested = match setting {
                    Some(DebugSetting::On) => Some(true),
                    Some(DebugSetting::Off) => Some(false),
                    None => None,
                };
                let enabled = self
                    .synchronous(move |bot| {
                        memory::set_debug_mode(bot, requested).map_err(ActionError::Memory)
                    })
                    .await?;
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
    async fn validate_model(&self, id: &str) -> Result<(), ActionError> {
        if id.trim().is_empty() || id != id.trim() {
            return Err(ActionError::InvalidArgument {
                key: "id".into(),
                reason: "invalid model id",
            });
        }
        if !self.bot.model_exists(id).await? {
            return Err(ActionError::InvalidArgument {
                key: "id".into(),
                reason: "unknown model",
            });
        }
        Ok(())
    }

    /// Restores the configured default, not a synthetic catalogue entry.
    pub(crate) async fn use_default_model(
        &self,
        actor: &Actor,
    ) -> Result<ActionResult, ActionError> {
        if !model_owner(actor, crate::owner_id()) {
            return Err(ActionError::PermissionDenied);
        }
        let configured = crate::config::runtime_value("MODEL").unwrap_or_default();
        let model = if configured.trim().is_empty() {
            let provider = crate::config::runtime_value("PROVIDER").unwrap_or_default();
            if provider.eq_ignore_ascii_case("mistral")
                || (crate::config::runtime_value("OPENROUTER_KEY")
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                    && !crate::config::runtime_value("MISTRAL_KEY")
                        .unwrap_or_default()
                        .trim()
                        .is_empty())
            {
                crate::MISTRAL_MODEL
            } else {
                crate::OPENROUTER_MODEL
            }
        } else {
            configured.trim()
        };
        self.validate_model(model).await?;
        let model = model.to_string();
        self.synchronous(move |bot| {
            memory::set_model_override(bot, &model, false).map_err(ActionError::Memory)?;
            Ok(ack(
                "model_cmd.title",
                strings::t("admin.action.model_default").replace("{model}", &model),
            ))
        })
        .await
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
            cycles_started: std::sync::atomic::AtomicBool::new(false),
            announcement_sent: std::sync::atomic::AtomicBool::new(false),
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
        let first = RoomRef::from_key(RoomKey::discord(7001)).conversation();
        let second = RoomRef::from_key(RoomKey::discord(7002)).conversation();
        {
            let mut state = bot.state();
            for room in [&first, &second] {
                state.chats.insert(room.clone(), crate::Chat::default());
                state
                    .awaiting_comment
                    .insert(room.clone(), std::time::Instant::now());
                state.channel_history.insert(
                    room.clone(),
                    std::collections::VecDeque::from(["retained".into()]),
                );
            }
        }
        let service = AdminService::new(&bot);
        let target = ActionTarget {
            connection: None,
            room: Some(RoomRef::from_key(first.room.clone())),
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
    async fn custom_provider_validation_is_unsupported_and_thinking_off_persists() {
        memory::test::ensure_test_db();
        let bot = fixture();
        let service = AdminService::new(&bot);
        let actor = Actor::LocalOperator;
        let target = ActionTarget::default();
        let id = "custom/provider-model".to_string();
        let result = service
            .execute(
                &actor,
                &target,
                ActionArgs::Model {
                    id: Some(id.clone()),
                },
            )
            .await;
        assert!(matches!(
            result,
            Err(ActionError::ModelValidationUnsupported)
        ));
        assert!(bot.state().model.is_empty());
        let query = service
            .execute(&actor, &target, ActionArgs::Model { id: None })
            .await
            .unwrap();
        assert!(matches!(query, ActionResult::Ack { description, .. }
            if description == strings::t("model_cmd.current").replace("{model}", "")));
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
        assert!(matches!(
            service.use_default_model(&actor).await,
            Err(ActionError::ModelValidationUnsupported)
        ));
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
    #[tokio::test]
    async fn reset_can_address_a_topic_without_erasing_base_room_or_other_topics() {
        let bot = fixture();
        let base = crate::platform::types::RoomRef::from_key(
            crate::platform::types::RoomKey::discord(87230),
        );
        let mut first = base.clone();
        first.thread = Some(crate::platform::types::ThreadKey::discord(87231));
        let mut second = base.clone();
        second.thread = Some(crate::platform::types::ThreadKey::discord(87232));
        for room in [&base, &first, &second] {
            bot.state()
                .chats
                .insert(room.conversation(), crate::Chat::default());
        }
        AdminService::new(&bot)
            .execute(
                &Actor::LocalOperator,
                &ActionTarget {
                    connection: None,
                    room: Some(first.clone()),
                },
                ActionArgs::Reset { all: false },
            )
            .await
            .unwrap();
        let state = bot.state();
        assert!(!state.chats.contains_key(&first.conversation()));
        assert!(state.chats.contains_key(&base.conversation()));
        assert!(state.chats.contains_key(&second.conversation()));
    }

    #[test]
    fn legacy_command_binding_preserves_actual_topic_without_approving_learning() {
        let bot = fixture();
        let connection = crate::platform::types::ConnectionId::new("legacy-command-route");
        let account = crate::platform::types::UserRef {
            id: UserKey::discord(87233),
            display_name: "Bot".into(),
            username: None,
            automation: crate::platform::types::Automation::Bot,
            stability: crate::platform::types::IdentityStability::Persistent,
        };
        let mut room = crate::platform::types::RoomRef::from_key(
            crate::platform::types::RoomKey::discord(87234),
        );
        room.thread = Some(crate::platform::types::ThreadKey::discord(87235));
        room.name = Some("actual command topic".into());
        {
            let mut registry = bot
                .adapters
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            registry.register(crate::platform::discord::test_adapter(
                connection.clone(),
                account,
            ));
            registry
                .set_room_policy(
                    &connection,
                    crate::platform::registry::RoomPolicy::Approved(
                        std::collections::HashSet::new(),
                    ),
                )
                .unwrap();
        }
        let target = ActionTarget {
            connection: Some(connection.clone()),
            room: Some(room.clone()),
        };
        AdminService::new(&bot)
            .authorize(
                &Actor::User(UserKey::discord(87236)),
                &target,
                &ActionArgs::Reset { all: false },
            )
            .unwrap();
        let registry = bot
            .adapters
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert!(registry.is_bound(&connection, &room));
        assert!(!registry.allows(&connection, &room));
        let actual = registry.room(&room.conversation()).unwrap();
        assert_eq!(actual.thread, room.thread);
        assert_eq!(actual.name, room.name);
    }
    #[tokio::test]
    async fn delivery_actions_reject_missing_or_unbound_routes_without_sent_ack() {
        let bot = fixture();
        let service = AdminService::new(&bot);
        let actions = [
            ActionArgs::News {},
            ActionArgs::Problem {},
            ActionArgs::Prank {},
            ActionArgs::Hack {},
        ];
        for action in actions {
            assert!(matches!(
                service
                    .execute(
                        &Actor::LocalOperator,
                        &ActionTarget::default(),
                        action.clone()
                    )
                    .await,
                Err(ActionError::MissingRoom)
            ));
            let target = ActionTarget {
                connection: None,
                room: Some(crate::platform::types::RoomRef::from_key(
                    crate::platform::types::RoomKey::discord(88001),
                )),
            };
            assert!(matches!(
                service
                    .execute(&Actor::LocalOperator, &target, action)
                    .await,
                Err(ActionError::Operation(_))
            ));
        }
        assert!(bot.state().busy.is_empty());
        assert!(bot.state().own_messages.is_empty());
        assert_eq!(bot.state().metrics.calls, 0);
    }

    #[tokio::test]
    async fn unavailable_send_capability_is_an_error_even_for_silent_model_text() {
        let bot = fixture();
        let connection = crate::platform::types::ConnectionId::new("unavailable-send");
        let account = crate::platform::types::UserRef {
            id: UserKey::discord(88002),
            display_name: "Bot".into(),
            username: None,
            automation: crate::platform::types::Automation::Bot,
            stability: crate::platform::types::IdentityStability::Persistent,
        };
        let room = crate::platform::types::RoomRef::from_key(
            crate::platform::types::RoomKey::discord(88003),
        );
        {
            let mut registry = bot
                .adapters
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            registry.register(crate::platform::discord::test_adapter(
                connection.clone(),
                account,
            ));
            registry.bind(&connection, room.clone()).unwrap();
        }
        assert!(bot
            .send(&room.conversation(), "not delivered", None, None, None)
            .await
            .is_err());
        assert!(bot
            .send_lines(&room.conversation(), "sessiz", None, None, None, false)
            .await
            .is_err());
        for action in [
            ActionArgs::News {},
            ActionArgs::Problem {},
            ActionArgs::Prank {},
            ActionArgs::Hack {},
        ] {
            assert!(matches!(
                AdminService::new(&bot)
                    .execute(
                        &Actor::LocalOperator,
                        &ActionTarget {
                            connection: Some(connection.clone()),
                            room: Some(room.clone())
                        },
                        action,
                    )
                    .await,
                Err(ActionError::Operation(_))
            ));
        }
        assert!(bot.state().own_messages.is_empty());
        assert!(bot.state().chats.is_empty());
        assert!(bot.state().busy.is_empty());
        assert_eq!(bot.state().metrics.calls, 0);
        bot.adapters
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .set_room_policy(
                &connection,
                crate::platform::registry::RoomPolicy::Approved(std::collections::HashSet::new()),
            )
            .unwrap();
        assert!(bot
            .authorize_automatic_delivery(&room.conversation())
            .is_err());
        {
            let mut state = bot.state();
            state.asleep = true;
            state.pending_mentions.push((
                room.conversation(),
                "Alice: mention".into(),
                crate::platform::types::UserRef {
                    id: UserKey::discord(88008),
                    display_name: "Alice".into(),
                    username: None,
                    automation: crate::platform::types::Automation::Human,
                    stability: crate::platform::types::IdentityStability::Persistent,
                },
            ));
        }
        assert!(matches!(
            AdminService::new(&bot)
                .execute(
                    &Actor::LocalOperator,
                    &ActionTarget::default(),
                    ActionArgs::Wake {},
                )
                .await,
            Err(ActionError::Operation(_))
        ));
        assert_eq!(bot.state().pending_mentions.len(), 1);
        assert!(bot.state().own_messages.is_empty());
    }

    #[tokio::test]
    async fn busy_reset_rejects_without_releasing_another_generations_flag() {
        let bot = fixture();
        let room = crate::platform::types::RoomRef::from_key(
            crate::platform::types::RoomKey::discord(88004),
        );
        let route = room.conversation();
        bot.state().busy.insert(route.clone());
        bot.state()
            .chats
            .insert(route.clone(), crate::Chat::default());
        let service = AdminService::new(&bot);
        for all in [false, true] {
            assert!(matches!(
                service
                    .execute(
                        &Actor::LocalOperator,
                        &ActionTarget {
                            connection: None,
                            room: Some(room.clone())
                        },
                        ActionArgs::Reset { all },
                    )
                    .await,
                Err(ActionError::Busy)
            ));
            assert!(bot.state().busy.contains(&route));
            assert!(bot.state().chats.contains_key(&route));
        }
    }

    #[tokio::test]
    async fn sleep_bounds_and_no_transition_ack_are_honest() {
        let bot = fixture();
        let service = AdminService::new(&bot);
        for hours in [-1, 0, 25, i64::MAX] {
            assert!(matches!(
                service
                    .execute(
                        &Actor::LocalOperator,
                        &ActionTarget::default(),
                        ActionArgs::Sleep { hours }
                    )
                    .await,
                Err(ActionError::InvalidArgument { .. })
            ));
        }
        assert!(bot.state().plans.is_empty());
        for hours in [1, 24] {
            service
                .execute(
                    &Actor::LocalOperator,
                    &ActionTarget::default(),
                    ActionArgs::Sleep { hours },
                )
                .await
                .unwrap();
            assert!(bot.state().asleep);
            let state = bot.state();
            let plan = state.plans.last().unwrap();
            assert_eq!(plan.end - plan.start, hours * 3600);
        }
        service
            .execute(
                &Actor::LocalOperator,
                &ActionTarget::default(),
                ActionArgs::Wake {},
            )
            .await
            .unwrap();
        let result = service
            .execute(
                &Actor::LocalOperator,
                &ActionTarget::default(),
                ActionArgs::Wake {},
            )
            .await
            .unwrap();
        assert!(
            matches!(result, ActionResult::Ack { description, .. } if description == strings::t("admin.action.no_transition"))
        );
        assert!(bot.state().own_messages.is_empty());
    }

    #[tokio::test]
    async fn default_restore_preserves_owner_policy() {
        let bot = fixture();
        let candidate = UserKey::discord(88005);
        let user = if crate::owner_id() == Some(&candidate) {
            UserKey::discord(88006)
        } else {
            candidate
        };
        assert!(matches!(
            AdminService::new(&bot)
                .use_default_model(&Actor::User(user.clone()))
                .await,
            Err(ActionError::PermissionDenied)
        ));
        assert!(!model_owner(&Actor::User(user), None));
        assert!(bot.state().model.is_empty());
    }

    #[tokio::test]
    async fn official_catalogue_transport_failure_cannot_set_or_restore_a_model() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new().fallback(|| async { axum::http::StatusCode::FORBIDDEN }),
            )
            .await
            .unwrap();
        });
        let mut bot = fixture();
        bot.api_url = crate::OPENROUTER_URL.into();
        bot.http = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(format!("http://{address}")).unwrap())
            .build()
            .unwrap();
        bot.state().model = "retained".into();
        let service = AdminService::new(&bot);
        assert!(matches!(
            service
                .execute(
                    &Actor::LocalOperator,
                    &ActionTarget::default(),
                    ActionArgs::Model {
                        id: Some("openai/gpt-4o-mini".into())
                    }
                )
                .await,
            Err(ActionError::Operation(_))
        ));
        assert!(matches!(
            service.use_default_model(&Actor::LocalOperator).await,
            Err(ActionError::Operation(_))
        ));
        assert_eq!(bot.state().model, "retained");
        server.abort();
    }
    #[tokio::test]
    async fn model_analysis_failure_is_an_error_not_a_raw_error_ack() {
        memory::test::ensure_test_db();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new().fallback(|| async {
                    (
                        axum::http::StatusCode::BAD_REQUEST,
                        "private-provider-response",
                    )
                }),
            )
            .await
            .unwrap();
        });
        let mut bot = fixture();
        bot.api_url = format!("http://{address}/chat/completions");
        let room = crate::platform::types::RoomRef::from_key(
            crate::platform::types::RoomKey::discord(88007),
        );
        bot.state().channel_history.insert(
            room.conversation(),
            std::collections::VecDeque::from(["Alice: transcript".into()]),
        );
        let result = AdminService::new(&bot)
            .execute(
                &Actor::LocalOperator,
                &ActionTarget {
                    connection: None,
                    room: Some(room),
                },
                ActionArgs::Mind { test: true },
            )
            .await;
        let error = result.unwrap_err();
        assert!(matches!(error, ActionError::Operation(_)));
        assert!(!error.to_string().contains("private-provider-response"));
        server.abort();
    }
}
