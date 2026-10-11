// The slash command manager's body is split across src/command/*.rs (registration table /
// card commands / action commands / setting commands / shared helpers). Same reasoning
// and same method as the bot/*.rs split in main.rs: `include!` (not real `mod`) so
// `use super::*` and visibility never change — these files compile as if written inline
// in this module.
include!("command/registration.rs");
include!("command/cards.rs");
include!("command/actions.rs");
include!("command/settings.rs");
include!("command/remaining.rs");

#[cfg(test)]
mod test {
    use super::*;

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
            let _ = (k.options)();
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
                .set_options((definition.options)());
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
