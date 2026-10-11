// Discord-facing UI text (slash command names/descriptions/options/choices, embed titles
// and field labels, button labels, modal field labels) lives as a flat `{"key": "value"}`
// object under langs/<dil>.json — not in Rust source (same reasoning as prompts/, see
// AGENTS.md rule 7), so a translation is a new file, not a code change. Placeholders are
// `{ad}`-style, filled with `.replace(...)` at the call site, same convention as prompts/.
// Which language's table is served is `Lang::current()` (lang.rs), fixed once for the
// process. `tr` and `en` are both filled in; a new language is a new `langs/<dil>.json`
// (every key `tr.json` has) plus one match arm in `table` below.

use crate::lang::Lang;
use std::collections::HashMap;
use std::sync::OnceLock;

mod tr {
    pub const RAW: &str = include_str!("../langs/tr.json");
}

mod en {
    pub const RAW: &str = include_str!("../langs/en.json");
}

/// Input: `lang: Lang`. Output: `&'static HashMap<String, String>` — that language's parsed
/// table, built once and reused. Panics on malformed JSON (a build-time asset, not user
/// input — should fail loudly, not degrade silently). Used by: `t` below.
fn table(lang: Lang) -> &'static HashMap<String, String> {
    static TR: OnceLock<HashMap<String, String>> = OnceLock::new();
    static EN: OnceLock<HashMap<String, String>> = OnceLock::new();
    match lang {
        Lang::Tr => TR.get_or_init(|| {
            serde_json::from_str(tr::RAW).unwrap_or_else(|e| panic!("langs/tr.json: {e}"))
        }),
        Lang::En => EN.get_or_init(|| {
            serde_json::from_str(en::RAW).unwrap_or_else(|e| panic!("langs/en.json: {e}"))
        }),
    }
}

/// Looks up a UI string by key in the active language's table. Input: `key: &'static str`.
/// Output: `&'static str` — the value, or `key` itself (logged once as an error) if the key
/// is missing — a typo'd key degrades visibly instead of panicking command registration.
/// Used by: `command/*.rs`, `modal.rs`, and every other Discord-facing text call site (was
/// a Turkish string literal before the language split).
pub fn t(key: &'static str) -> &'static str {
    match table(Lang::current()).get(key) {
        Some(v) => v.as_str(),
        None => {
            log::error!("missing localization key '{key}'");
            key
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// Verifies a handful of keys from every corner of the active table (command table, embed
    /// chrome, HELP text) actually resolve — a broad canary against a bad key rename.
    #[test]
    fn known_keys_resolve() {
        for key in [
            "cmd.durum.name",
            "cmd.dusunme.choice.goster",
            "mind.title",
            "status.title",
            "help.text",
            "settings.wake_button",
        ] {
            assert_ne!(
                t(key),
                key,
                "key '{key}' didn't resolve (langs/tr.json missing it?)"
            );
        }
    }

    /// Verifies a missing key falls back to the key itself instead of panicking.
    #[test]
    fn missing_key_falls_back_to_itself() {
        assert_eq!(t("nonexistent.key.for.test"), "nonexistent.key.for.test");
    }

    /// Verifies a multi-line value (`help.text`, several `\n`s) actually parses to real
    /// newline characters, not a literal two-character `\n`.
    #[test]
    fn multiline_value_has_real_newlines() {
        assert!(t("help.text").contains('\n'));
    }

    /// Verifies `langs/en.json` parses and resolves the same canary keys as tr — checked via
    /// `table(Lang::En)` directly (not `t`, which reads the process-wide `Lang::current()`
    /// and can't be pointed at a specific language mid-test-run).
    #[test]
    fn en_table_resolves_known_keys() {
        let en = table(Lang::En);
        for key in [
            "cmd.durum.name",
            "cmd.dusunme.choice.goster",
            "mind.title",
            "status.title",
            "help.text",
            "settings.wake_button",
        ] {
            assert!(en.contains_key(key), "en.json missing key '{key}'");
        }
        assert!(en.get("help.text").unwrap().contains('\n'));
    }
}
