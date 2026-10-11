use super::{ActionArgs, ActionId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKind {
    Boolean,
    Integer,
    String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionDefault {
    Boolean(bool),
    Integer(i64),
}
#[derive(Debug, Clone, Copy)]
pub struct Choice {
    pub name_key: &'static str,
    pub value: &'static str,
}
#[derive(Debug, Clone, Copy)]
pub struct OptionDefinition {
    pub key: &'static str,
    pub name_key: &'static str,
    pub description_key: &'static str,
    pub kind: OptionKind,
    pub default: Option<OptionDefault>,
    pub choices: &'static [Choice],
    pub min: Option<i64>,
    pub max: Option<i64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Administrator,
    OwnerForModelSetter,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferRequirement {
    Never,
    Always,
    MindTest,
    ModelSet,
}
#[derive(Debug, Clone, Copy)]
pub struct ActionDefinition {
    pub id: ActionId,
    pub name_key: &'static str,
    pub description_key: &'static str,
    pub options: &'static [OptionDefinition],
    pub permission: Permission,
    pub defer: DeferRequirement,
}
impl ActionDefinition {
    pub fn name(&self) -> &'static str {
        crate::strings::t(self.name_key)
    }
    pub fn description(&self) -> &'static str {
        crate::strings::t(self.description_key)
    }
    pub fn should_defer(&self, args: &ActionArgs) -> bool {
        match self.defer {
            DeferRequirement::Never => false,
            DeferRequirement::Always => true,
            DeferRequirement::MindTest => matches!(args, ActionArgs::Mind { test: true }),
            DeferRequirement::ModelSet => matches!(args, ActionArgs::Model { id: Some(_) }),
        }
    }
}
const THINKING_CHOICES: &[Choice] = &[
    Choice {
        name_key: "cmd.dusunme.choice.goster",
        value: "goster",
    },
    Choice {
        name_key: "cmd.dusunme.choice.gizle",
        value: "gizle",
    },
    Choice {
        name_key: "cmd.dusunme.choice.sessiz",
        value: "sessiz",
    },
    Choice {
        name_key: "cmd.dusunme.choice.kapat",
        value: "kapat",
    },
];
const DEBUG_CHOICES: &[Choice] = &[
    Choice {
        name_key: "cmd.debug.choice.ac",
        value: "ac",
    },
    Choice {
        name_key: "cmd.debug.choice.kapat",
        value: "kapat",
    },
];
macro_rules! option {
    ($key:literal, $base:literal, $kind:ident, $default:expr, $choices:expr, $min:expr) => {
        OptionDefinition {
            key: $key,
            name_key: concat!($base, ".name"),
            description_key: concat!($base, ".description"),
            kind: OptionKind::$kind,
            default: $default,
            choices: $choices,
            min: $min,
            max: None,
        }
    };
}
macro_rules! action {
    ($id:ident, $base:literal, $options:expr, $defer:ident, $permission:ident) => {
        ActionDefinition {
            id: ActionId::$id,
            name_key: concat!($base, ".name"),
            description_key: concat!($base, ".description"),
            options: $options,
            permission: Permission::$permission,
            defer: DeferRequirement::$defer,
        }
    };
}
const DEFINITIONS: &[ActionDefinition] = &[
    action!(Status, "cmd.durum", &[], Never, Administrator),
    action!(Help, "cmd.yardim", &[], Never, Administrator),
    action!(
        Mind,
        "cmd.zihin",
        &[option!(
            "test",
            "cmd.zihin.opt.test",
            Boolean,
            Some(OptionDefault::Boolean(false)),
            &[],
            None
        )],
        MindTest,
        Administrator
    ),
    action!(Settings, "cmd.ayarlar", &[], Never, Administrator),
    action!(
        Reset,
        "cmd.sifirla",
        &[option!(
            "all",
            "cmd.sifirla.opt.hepsi",
            Boolean,
            Some(OptionDefault::Boolean(false)),
            &[],
            None
        )],
        Never,
        Administrator
    ),
    action!(News, "cmd.haber", &[], Always, Administrator),
    action!(Problem, "cmd.sorun", &[], Always, Administrator),
    action!(Wander, "cmd.gez", &[], Always, Administrator),
    action!(Prank, "cmd.saka", &[], Always, Administrator),
    action!(Hack, "cmd.hack", &[], Always, Administrator),
    action!(Agents, "cmd.ajanlar", &[], Always, Administrator),
    action!(Wake, "cmd.uyan", &[], Always, Administrator),
    action!(
        Sleep,
        "cmd.uyu",
        &[OptionDefinition {
            max: Some(24),
            ..option!(
                "hours",
                "cmd.uyu.opt.saat",
                Integer,
                Some(OptionDefault::Integer(8)),
                &[],
                Some(1)
            )
        }],
        Always,
        Administrator
    ),
    action!(
        Thinking,
        "cmd.dusunme",
        &[option!(
            "mode",
            "cmd.dusunme.opt.kip",
            String,
            None,
            THINKING_CHOICES,
            None
        )],
        Never,
        Administrator
    ),
    action!(
        Model,
        "cmd.model",
        &[option!("id", "cmd.model.opt.id", String, None, &[], None)],
        ModelSet,
        OwnerForModelSetter
    ),
    action!(
        Debug,
        "cmd.debug",
        &[option!(
            "state",
            "cmd.debug.opt.durum",
            String,
            None,
            DEBUG_CHOICES,
            None
        )],
        Never,
        Administrator
    ),
];
/// Stack storage for text arguments is sized from the schema, not a second limit.
pub(crate) const MAX_OPTIONS: usize = {
    let mut maximum = 0;
    let mut index = 0;
    while index < DEFINITIONS.len() {
        if DEFINITIONS[index].options.len() > maximum {
            maximum = DEFINITIONS[index].options.len();
        }
        index += 1;
    }
    maximum
};
pub fn definitions() -> &'static [ActionDefinition] {
    DEFINITIONS
}
pub fn definition(id: ActionId) -> &'static ActionDefinition {
    // The one-to-one fixture below guards this invariant; no per-call allocation.
    &DEFINITIONS[id as usize]
}
pub fn lookup(name: &str) -> Option<ActionId> {
    DEFINITIONS
        .iter()
        .find(|definition| definition.id.as_str() == name || definition.name() == name)
        .map(|definition| definition.id)
}
pub fn default_sleep_hours() -> i64 {
    match definition(ActionId::Sleep).options[0].default {
        Some(OptionDefault::Integer(hours)) => hours,
        _ => unreachable!("sleep schema requires an integer default"),
    }
}
pub fn default_mind_test() -> bool {
    match definition(ActionId::Mind).options[0].default {
        Some(OptionDefault::Boolean(test)) => test,
        _ => unreachable!("mind schema requires a boolean default"),
    }
}
pub fn default_reset_all() -> bool {
    match definition(ActionId::Reset).options[0].default {
        Some(OptionDefault::Boolean(all)) => all,
        _ => unreachable!("reset schema requires a boolean default"),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema_represents_every_action_once() {
        assert_eq!(definitions().len(), 16);
        for (index, id) in ActionId::ALL.iter().enumerate() {
            assert_eq!(definitions()[index].id, *id);
            assert_eq!(
                definitions().iter().filter(|entry| entry.id == *id).count(),
                1
            );
            assert_eq!(lookup(id.as_str()), Some(*id));
        }
    }
    #[test]
    fn wire_choices_preserve_persistence_difference() {
        let values: Vec<_> = definition(ActionId::Thinking).options[0]
            .choices
            .iter()
            .map(|c| c.value)
            .collect();
        assert_eq!(values, ["goster", "gizle", "sessiz", "kapat"]);
        for value in values {
            assert!(crate::ThinkingMode::from_arg(value).is_some());
        }
        assert_eq!(super::super::ThinkingSetting::Off.file_value(), "kapali");
        assert_eq!(
            definition(ActionId::Sleep).options[0].default,
            Some(OptionDefault::Integer(8))
        );
        assert_eq!(definition(ActionId::Sleep).options[0].min, Some(1));
    }
}
