use super::{
    read_models::{MetricsView, StatusView},
    ActionError, ActionResult, Visibility,
};
use crate::strings;

pub fn localized_description(error: &ActionError) -> String {
    match error {
        ActionError::InvalidArgument { key, reason } => {
            let reason_key = match *reason {
                "type" | "incorrect type" => "admin.error.reason.type",
                "unknown option" => "admin.error.reason.unknown_option",
                "duplicate option" => "admin.error.reason.duplicate_option",
                "outside bounds" => "admin.error.reason.bounds",
                "unknown choice" => "admin.error.reason.choice",
                "missing default" => "admin.error.reason.default",
                "too many arguments" => "admin.error.reason.arguments",
                "invalid model id" => "admin.error.reason.model_id",
                "unknown model" => "admin.error.reason.model_unknown",
                _ => "admin.error.reason.type",
            };
            strings::t("admin.error.argument")
                .replace("{key}", key)
                .replace("{reason}", strings::t(reason_key))
        }
        ActionError::UnknownAction => strings::t("admin.error.unknown").to_string(),
        ActionError::PermissionDenied => strings::t("admin.error.forbidden").to_string(),
        ActionError::MissingConnection => strings::t("admin.error.connection").to_string(),
        ActionError::MissingRoom => strings::t("admin.error.room").to_string(),
        ActionError::UnboundRoom => strings::t("admin.error.unbound").to_string(),
        ActionError::PrivateSurfaceRequired => strings::t("admin.error.private").to_string(),
        ActionError::Memory(_) => strings::t("admin.error.memory").to_string(),
        ActionError::Platform(_) => strings::t("admin.error.platform").to_string(),
        ActionError::Operation(_) => strings::t("admin.error.operation").to_string(),
        ActionError::Busy => strings::t("admin.error.busy").to_string(),
        ActionError::ModelValidationUnsupported => {
            strings::t("admin.error.model_validation_unsupported").to_string()
        }
    }
}
fn tokens(metrics: &MetricsView) -> String {
    strings::t("common.token_template")
        .replace("{calls}", &metrics.calls.to_string())
        .replace("{input}", &metrics.input_tokens.to_string())
        .replace("{cache}", &metrics.cache_tokens.to_string())
        .replace("{output}", &metrics.output_tokens.to_string())
}
fn status(view: &StatusView) -> String {
    let general = strings::t("status.general_template")
        .replace("{version}", &view.version)
        .replace("{stage}", &view.stage)
        .replace("{day}", &view.day.to_string())
        .replace("{chats}", &view.chats.to_string())
        .replace("{messages}", &view.messages.to_string())
        .replace("{model}", &view.model);
    let state = strings::t("status.state_template")
        .replace(
            "{sleep}",
            strings::t(if view.awake {
                "common.awake"
            } else {
                "common.asleep"
            }),
        )
        .replace("{thinking}", &view.thinking_label)
        .replace(
            "{debug}",
            strings::t(if view.debug {
                "common.debug_on"
            } else {
                "common.debug_off"
            }),
        )
        .replace("{travel}", &view.travel);
    format!(
        "{}\n{}\n{}\n{}",
        strings::t("status.title"),
        general,
        state,
        tokens(&view.metrics)
    )
}
/// Never turns a private card/thought into a public channel message when a transport
/// lacks a private result surface. The caller must select an actual private route.
pub fn plain(result: &ActionResult, has_private_surface: bool) -> Result<String, ActionError> {
    if result.visibility() == Visibility::Private && !has_private_surface {
        return Err(ActionError::PrivateSurfaceRequired);
    }
    Ok(match result {
        ActionResult::Ack { title, description } => format!("{title}\n{description}"),
        ActionResult::Status(view) => status(view),
        ActionResult::Settings(view) => strings::t("settings.description_template")
            .replace("{version}", &view.status.version)
            .replace("{model}", &view.status.model)
            .replace("{thinking}", &view.status.thinking_label)
            .replace(
                "{debug}",
                strings::t(if view.status.debug {
                    "common.debug_on"
                } else {
                    "common.debug_off"
                }),
            )
            .replace(
                "{sleep}",
                strings::t(if view.status.awake {
                    if view.status.forced_awake {
                        "settings.awake_forced"
                    } else {
                        "common.awake"
                    }
                } else {
                    "common.asleep"
                }),
            )
            .replace("{travel}", &view.status.travel),
        ActionResult::Help(_) => {
            use std::fmt::Write as _;
            let mut text = format!(
                "{}\n{}",
                strings::t("help.title"),
                strings::t("admin.plain.help.description")
            );
            for action in super::schema::definitions() {
                let _ = write!(text, "\n{}: {}", action.id.as_str(), action.description());
                for option in action.options {
                    let _ = write!(text, " [{}", option.key);
                    if !option.choices.is_empty() {
                        text.push('=');
                        for (index, choice) in option.choices.iter().enumerate() {
                            if index != 0 {
                                text.push('|');
                            }
                            text.push_str(choice.value);
                        }
                    }
                    text.push(']');
                }
            }
            text
        }
        ActionResult::Mind(view) => {
            use std::fmt::Write as _;
            let mut text = format!("{}\n{}", strings::t("mind.title"), status(&view.status));
            let _ = write!(
                text,
                "\n{}",
                strings::t("mind.people_field").replace("{count}", &view.people.len().to_string())
            );
            for person in &view.people {
                let _ = write!(
                    text,
                    "\n{} ({:+}) {}",
                    person.name,
                    person.score,
                    person.tags.join(", ")
                );
                if !person.note.is_empty() {
                    let _ = write!(text, "\n{}", person.note);
                }
            }
            let _ = write!(
                text,
                "\n{}",
                strings::t("mind.topics_field").replace("{count}", &view.topics.len().to_string())
            );
            for (name, note) in &view.topics {
                let _ = write!(text, "\n{name}: {note}");
            }
            let count: usize = view.events.iter().map(|(_, lines)| lines.len()).sum();
            let _ = write!(
                text,
                "\n{}",
                strings::t("mind.events_field").replace("{count}", &count.to_string())
            );
            for (month, lines) in &view.events {
                let _ = write!(text, "\n{month}\n{}", lines.join("\n"));
            }
            text
        }
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_results_never_fall_back_to_public_plain_text() {
        let result = ActionResult::Ack {
            title: "title".into(),
            description: "private".into(),
        };
        assert!(matches!(
            plain(&result, false),
            Err(ActionError::PrivateSurfaceRequired)
        ));
        assert_eq!(plain(&result, true).unwrap(), "title\nprivate");
        let help = ActionResult::Help(super::super::read_models::help());
        assert!(matches!(
            plain(&help, false),
            Err(ActionError::PrivateSurfaceRequired)
        ));
        let text = plain(&help, true).unwrap();
        for id in super::super::ActionId::ALL {
            assert!(text.contains(id.as_str()));
        }
    }
    #[test]
    fn localized_failures_do_not_expose_provider_or_storage_details() {
        for error in [
            ActionError::Operation("private-provider-response".into()),
            ActionError::Memory("private-storage-path".into()),
            ActionError::Platform(crate::platform::PlatformError::Transport(
                "private-platform-token".into(),
            )),
        ] {
            assert!(!localized_description(&error).contains("private-"));
        }
    }
}
