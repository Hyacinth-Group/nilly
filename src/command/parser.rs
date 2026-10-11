use super::{
    schema::{self, OptionDefault, OptionKind},
    ActionArgs, ActionError, ActionId, ActionValue, DebugSetting, ThinkingSetting,
};
use std::collections::HashMap;

fn invalid(key: &str, reason: &'static str) -> ActionError {
    ActionError::InvalidArgument {
        key: key.to_string(),
        reason,
    }
}
#[derive(Debug, Clone, Copy)]
pub enum BorrowedActionValue<'a> {
    Boolean(bool),
    Integer(i64),
    String(&'a str),
}
impl<'a> From<&'a ActionValue> for BorrowedActionValue<'a> {
    fn from(value: &'a ActionValue) -> Self {
        match value {
            ActionValue::Boolean(value) => Self::Boolean(*value),
            ActionValue::Integer(value) => Self::Integer(*value),
            ActionValue::String(value) => Self::String(value),
        }
    }
}
/// Stable option keys and localized native option names both resolve through the schema.
pub fn parse_values(
    id: ActionId,
    values: &HashMap<String, ActionValue>,
) -> Result<ActionArgs, ActionError> {
    parse_native_values(
        id,
        values
            .iter()
            .map(|(key, value)| (key.as_str(), value.into())),
    )
}
/// Borrow native option payloads, allocating only a model ID that enters owned execution.
pub fn parse_native_values<'a>(
    id: ActionId,
    values: impl Clone + Iterator<Item = (&'a str, BorrowedActionValue<'a>)>,
) -> Result<ActionArgs, ActionError> {
    let definition = schema::definition(id);
    for (key, value) in values.clone() {
        let option = definition
            .options
            .iter()
            .find(|option| option.key == key || crate::strings::t(option.name_key) == key)
            .ok_or_else(|| invalid(key, "unknown option"))?;
        if values
            .clone()
            .filter(|(key, _)| *key == option.key || *key == crate::strings::t(option.name_key))
            .count()
            > 1
        {
            return Err(invalid(key, "duplicate option"));
        }
        match (option.kind, value) {
            (OptionKind::Boolean, BorrowedActionValue::Boolean(_)) => {}
            (OptionKind::Integer, BorrowedActionValue::Integer(number)) => {
                if option.min.is_some_and(|min| number < min)
                    || option.max.is_some_and(|max| number > max)
                {
                    return Err(invalid(key, "outside bounds"));
                }
            }
            (OptionKind::String, BorrowedActionValue::String(text)) => {
                if !option.choices.is_empty()
                    && !option.choices.iter().any(|choice| choice.value == text)
                {
                    return Err(invalid(key, "unknown choice"));
                }
            }
            _ => return Err(invalid(key, "incorrect type")),
        }
    }
    let value = |key: &str| {
        let option = definition.options.iter().find(|option| option.key == key)?;
        values
            .clone()
            .find(|(key, _)| *key == option.key || *key == crate::strings::t(option.name_key))
            .map(|(_, value)| value)
    };
    let boolean = |key: &str| match value(key) {
        Some(BorrowedActionValue::Boolean(value)) => value,
        _ => match definition
            .options
            .iter()
            .find(|option| option.key == key)
            .and_then(|option| option.default)
        {
            Some(OptionDefault::Boolean(value)) => value,
            _ => false,
        },
    };
    let text = |key: &str| match value(key) {
        Some(BorrowedActionValue::String(value)) => Some(value),
        _ => None,
    };
    Ok(match id {
        ActionId::Status => ActionArgs::Status {},
        ActionId::Help => ActionArgs::Help {},
        ActionId::Mind => ActionArgs::Mind {
            test: boolean("test"),
        },
        ActionId::Settings => ActionArgs::Settings {},
        ActionId::Reset => ActionArgs::Reset {
            all: boolean("all"),
        },
        ActionId::News => ActionArgs::News {},
        ActionId::Problem => ActionArgs::Problem {},
        ActionId::Wander => ActionArgs::Wander {},
        ActionId::Prank => ActionArgs::Prank {},
        ActionId::Hack => ActionArgs::Hack {},
        ActionId::Agents => ActionArgs::Agents {},
        ActionId::Wake => ActionArgs::Wake {},
        ActionId::Sleep => ActionArgs::Sleep {
            hours: match value("hours") {
                Some(BorrowedActionValue::Integer(hours)) => hours,
                _ => match definition.options[0].default {
                    Some(OptionDefault::Integer(hours)) => hours,
                    _ => return Err(invalid("hours", "missing default")),
                },
            },
        },
        ActionId::Thinking => ActionArgs::Thinking {
            mode: text("mode").map(|wire| {
                // Choice validity was checked above; map the canonical schema order to the typed setting.
                let index = definition.options[0]
                    .choices
                    .iter()
                    .position(|choice| choice.value == wire)
                    .expect("validated schema choice");
                [
                    ThinkingSetting::Show,
                    ThinkingSetting::Hide,
                    ThinkingSetting::Silent,
                    ThinkingSetting::Off,
                ][index]
            }),
        },
        ActionId::Model => ActionArgs::Model {
            id: text("id").map(str::to_string),
        },
        ActionId::Debug => ActionArgs::Debug {
            setting: text("state").map(|wire| {
                if wire == definition.options[0].choices[0].value {
                    DebugSetting::On
                } else {
                    DebugSetting::Off
                }
            }),
        },
    })
}
/// Prefixes are configured by the adapter. Discord ingress never selects one.
#[derive(Debug, Clone)]
pub enum CommandPrefix {
    Disabled,
    Explicit(String),
    Telegram { bot_name: String },
}
/// None means ordinary conversation. An unknown explicit action becomes localized help.
pub fn parse_text(text: &str, prefix: &CommandPrefix) -> Result<Option<ActionArgs>, ActionError> {
    let body = match prefix {
        CommandPrefix::Disabled => return Ok(None),
        CommandPrefix::Explicit(prefix) => {
            if prefix.is_empty() {
                return Ok(None);
            }
            let Some(rest) = text.strip_prefix(prefix) else {
                return Ok(None);
            };
            if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
                return Ok(None);
            }
            rest.trim()
        }
        CommandPrefix::Telegram { bot_name } => {
            if bot_name.is_empty() {
                return Ok(None);
            }
            let Some(rest) = text.strip_prefix('/') else {
                return Ok(None);
            };
            let command = rest.split_whitespace().next().unwrap_or("");
            if let Some((_, addressed)) = command.split_once('@') {
                if !addressed.eq_ignore_ascii_case(bot_name) {
                    return Ok(None);
                }
            }
            rest
        }
    };
    let mut words = body.split_whitespace();
    let name = words.next().unwrap_or("");
    let name = match prefix {
        CommandPrefix::Telegram { .. } => name.split('@').next().unwrap_or(""),
        _ => name,
    };
    let Some(id) = schema::lookup(name) else {
        return Ok(Some(ActionArgs::Help {}));
    };
    let definition = schema::definition(id);
    let mut values = [None; schema::MAX_OPTIONS];
    for (index, word) in words.enumerate() {
        let (key, raw) = if let Some((key, value)) = word.split_once('=') {
            (key, value)
        } else {
            (
                definition
                    .options
                    .get(index)
                    .ok_or_else(|| invalid(word, "too many arguments"))?
                    .key,
                word,
            )
        };
        let option = definition
            .options
            .iter()
            .find(|option| option.key == key || crate::strings::t(option.name_key) == key)
            .ok_or_else(|| invalid(key, "unknown option"))?;
        let parsed = match option.kind {
            OptionKind::Boolean => match raw {
                "true" => BorrowedActionValue::Boolean(true),
                "false" => BorrowedActionValue::Boolean(false),
                _ => return Err(invalid(key, "incorrect type")),
            },
            OptionKind::Integer => BorrowedActionValue::Integer(
                raw.parse().map_err(|_| invalid(key, "incorrect type"))?,
            ),
            OptionKind::String => BorrowedActionValue::String(raw),
        };
        *values
            .get_mut(index)
            .ok_or_else(|| invalid(key, "too many arguments"))? = Some((key, parsed));
    }
    parse_native_values(id, values.into_iter().flatten()).map(Some)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_action_parses_without_options_and_sleep_uses_schema_default() {
        for id in ActionId::ALL {
            assert_eq!(parse_values(id, &HashMap::new()).unwrap().id(), id);
        }
        assert_eq!(
            parse_values(ActionId::Sleep, &HashMap::new()).unwrap(),
            ActionArgs::Sleep { hours: 8 }
        );
    }
    #[test]
    fn typed_http_defaults_come_from_same_schema() {
        for (json, expected) in [
            (
                r#"{"action":"sleep"}"#,
                ActionArgs::Sleep {
                    hours: schema::default_sleep_hours(),
                },
            ),
            (
                r#"{"action":"mind"}"#,
                ActionArgs::Mind {
                    test: schema::default_mind_test(),
                },
            ),
            (
                r#"{"action":"reset"}"#,
                ActionArgs::Reset {
                    all: schema::default_reset_all(),
                },
            ),
        ] {
            assert_eq!(serde_json::from_str::<ActionArgs>(json).unwrap(), expected);
        }
        assert!(
            serde_json::from_str::<ActionArgs>(r#"{"action":"status","unknown":true}"#).is_err()
        );
    }
    #[test]
    fn borrowed_native_and_owned_values_share_validation() {
        let localized = crate::strings::t(schema::definition(ActionId::Sleep).options[0].name_key);
        assert_eq!(
            parse_native_values(
                ActionId::Sleep,
                [(localized, BorrowedActionValue::Integer(4))].into_iter()
            )
            .unwrap(),
            ActionArgs::Sleep { hours: 4 }
        );
        assert!(matches!(
            parse_native_values(
                ActionId::Sleep,
                [
                    ("hours", BorrowedActionValue::Integer(4)),
                    (localized, BorrowedActionValue::Integer(4)),
                ]
                .into_iter()
            ),
            Err(ActionError::InvalidArgument {
                reason: "duplicate option",
                ..
            })
        ));
        assert!(parse_native_values(
            ActionId::Mind,
            [("test", BorrowedActionValue::String("true"))].into_iter()
        )
        .is_err());
    }
    #[test]
    fn types_choices_bounds_and_unknown_keys_are_rejected() {
        for (id, key, value) in [
            (ActionId::Sleep, "hours", ActionValue::Integer(0)),
            (ActionId::Sleep, "hours", ActionValue::String("8".into())),
            (
                ActionId::Thinking,
                "mode",
                ActionValue::String("kapali".into()),
            ),
            (ActionId::Debug, "state", ActionValue::String("on".into())),
            (ActionId::Status, "unknown", ActionValue::Boolean(true)),
        ] {
            assert!(parse_values(id, &HashMap::from([(key.to_string(), value)])).is_err());
        }
        assert_eq!(
            parse_values(
                ActionId::Thinking,
                &HashMap::from([("mode".into(), ActionValue::String("kapat".into()))])
            )
            .unwrap(),
            ActionArgs::Thinking {
                mode: Some(ThinkingSetting::Off)
            }
        );
    }
    #[test]
    fn prefix_is_explicit_and_ordinary_text_is_never_administration() {
        let prefix = CommandPrefix::Explicit("!bot".into());
        assert_eq!(parse_text("status", &prefix).unwrap(), None);
        assert_eq!(parse_text("!botany status", &prefix).unwrap(), None);
        assert_eq!(
            parse_text("!bot status", &prefix).unwrap(),
            Some(ActionArgs::Status {})
        );
        assert_eq!(
            parse_text("!bot nonsense", &prefix).unwrap(),
            Some(ActionArgs::Help {})
        );
        assert_eq!(
            parse_text("!bot sleep", &prefix).unwrap(),
            Some(ActionArgs::Sleep { hours: 8 })
        );
        assert!(parse_text("!bot sleep 0", &prefix).is_err());
        assert_eq!(
            parse_text("!bot status", &CommandPrefix::Disabled).unwrap(),
            None
        );
        let telegram = CommandPrefix::Telegram {
            bot_name: "BotName".into(),
        };
        assert_eq!(
            parse_text("/status", &telegram).unwrap(),
            Some(ActionArgs::Status {})
        );
        assert_eq!(parse_text("/status@Other", &telegram).unwrap(), None);
        assert_eq!(
            parse_text("/status@BotName", &telegram).unwrap(),
            Some(ActionArgs::Status {})
        );
    }
}
