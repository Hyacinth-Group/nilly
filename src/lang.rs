// Which language the bot runs in: picks the compiled-in prompt set (prompts/<dil>/, see
// prompts.rs) and UI string table (langs/<dil>.json, see strings.rs). One process, one
// language, fixed for the whole run (like PROVIDER/MODEL), defaulting to English.
// Read from `BOT_LANG` in `.env`. Named BOT_LANG rather than LANG: most shells
// already export LANG for the OS locale, and reading that by accident would silently pick
// up whatever the host's locale happens to be instead of the operator's actual choice.
//
// Adding a language: add a variant here, a `prompts/<dil>/` folder (every file
// `prompts/tr/` has), a `langs/<dil>.json` (every key `langs/tr.json` has), one match arm in
// `prompts::get` and one in `strings::table`.

use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Tr,
    En,
}

static CURRENT: OnceLock<Lang> = OnceLock::new();

impl Lang {
    /// Accepts en/tr case-insensitively; empty or unknown values use English,
    /// with a warning for unknown values.
    fn parse(raw: &str) -> Lang {
        match raw.trim().to_lowercase().as_str() {
            "tr" => Lang::Tr,
            "" | "en" => Lang::En,
            other => {
                log::warn!("unknown BOT_LANG '{other}', falling back to en");
                Lang::En
            }
        }
    }

    /// The process-wide language, resolved once (from `BOT_LANG`) and reused for every
    /// later call — first caller wins, matching every other `.env` setting's fixed-at-startup
    /// behavior.
    pub fn current() -> Lang {
        *CURRENT.get_or_init(|| {
            Self::parse(&crate::config::runtime_value("BOT_LANG").unwrap_or_default())
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// Verifies `parse` accepts "tr"/"en" case-insensitively and falls back to en for anything else.
    #[test]
    fn parse_recognizes_known_languages_falls_back_to_en() {
        assert_eq!(Lang::parse(""), Lang::En);
        assert_eq!(Lang::parse("tr"), Lang::Tr);
        assert_eq!(Lang::parse("TR"), Lang::Tr);
        assert_eq!(Lang::parse("  tr  "), Lang::Tr);
        assert_eq!(Lang::parse("en"), Lang::En);
        assert_eq!(Lang::parse("EN"), Lang::En);
        assert_eq!(Lang::parse("  en  "), Lang::En);
        assert_eq!(Lang::parse("de"), Lang::En);
    }
}
