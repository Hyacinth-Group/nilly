//! Explicit transport context for model prompts; Discord's default wording is preserved.
use super::{
    capabilities::{Capabilities, HistoryAccess},
    types::PlatformKind,
};
use crate::{lang::Lang, prompts};
use std::borrow::Cow;

#[derive(Debug, Clone, Copy)]
pub struct PromptContext {
    platform: PlatformKind,
    reactions: bool,
    legacy_history: bool,
}

impl PromptContext {
    pub fn for_platform(platform: PlatformKind) -> Self {
        Self {
            platform,
            reactions: platform == PlatformKind::Discord,
            legacy_history: platform == PlatformKind::Discord,
        }
    }

    pub fn with_reactions(mut self, available: bool) -> Self {
        self.reactions = available;
        self
    }

    /// Enable the historical two-week claim only for the actual legacy Discord scan.
    pub fn with_legacy_history(mut self, supplied: bool) -> Self {
        self.legacy_history = supplied && self.platform == PlatformKind::Discord;
        self
    }

    pub fn with_capabilities(self, capabilities: &Capabilities) -> Self {
        self.with_reactions(capabilities.react)
            .with_legacy_history(self.legacy_history && capabilities.history != HistoryAccess::None)
    }

    pub fn render<'a>(&self, template: &'a str) -> Cow<'a, str> {
        self.render_with(template, &[])
    }

    /// Replace template tokens once. Replacement values are literal: a name containing
    /// another token cannot alter the prompt. No allocation occurs without a substitution.
    pub fn render_with<'a>(&self, template: &'a str, values: &[(&str, &str)]) -> Cow<'a, str> {
        let mut output: Option<String> = None;
        let mut copied = 0;
        let mut cursor = 0;
        while let Some((start, end)) = next_token(template, cursor) {
            cursor = end;
            let token = &template[start..end];
            let value = values.iter().find(|(key, _)| *key == token);
            if value.is_some_and(|(_, replacement)| *replacement == token) {
                continue;
            }
            let fragment = if value.is_none() {
                self.fragment(token)
            } else {
                None
            };
            if value.is_none() && fragment.is_none() {
                continue;
            }
            let out = output.get_or_insert_with(|| String::with_capacity(template.len()));
            out.push_str(&template[copied..start]);
            if let Some((_, value)) = value {
                out.push_str(value);
            } else if let Some(fragment) = fragment {
                append_values(out, fragment, values);
            }
            copied = end;
        }
        match output {
            Some(mut output) => {
                output.push_str(&template[copied..]);
                Cow::Owned(output)
            }
            None => Cow::Borrowed(template),
        }
    }

    fn fragment(&self, token: &str) -> Option<&'static str> {
        let (lang, key) = if let Some(key) = token.strip_prefix("{platform_en_") {
            (Lang::En, key)
        } else {
            (Lang::Tr, token.strip_prefix("{platform_tr_")?)
        };
        prompts::platform_fragment(
            lang,
            key.strip_suffix('}')?,
            self.platform,
            self.reactions,
            self.legacy_history,
        )
    }
}

fn next_token(text: &str, cursor: usize) -> Option<(usize, usize)> {
    let start = cursor + text[cursor..].find('{')?;
    let end = start + text[start..].find('}')? + 1;
    Some((start, end))
}

/// Fragment prose may contain ordinary name/favorite slots. Fill those directly into
/// the same buffer, never rescanning user-supplied replacement text or allocating a copy.
fn append_values(output: &mut String, fragment: &str, values: &[(&str, &str)]) {
    if values.is_empty() {
        output.push_str(fragment);
        return;
    }
    let mut cursor = 0;
    let mut copied = 0;
    while let Some((start, end)) = next_token(fragment, cursor) {
        cursor = end;
        if let Some((_, value)) = values.iter().find(|(key, _)| *key == &fragment[start..end]) {
            output.push_str(&fragment[copied..start]);
            output.push_str(value);
            copied = end;
        }
    }
    output.push_str(&fragment[copied..]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn templates(p: &prompts::Prompts) -> [(&str, &str); 31] {
        [
            ("kisilik", p.personality),
            ("favori-satiri", p.favorite_line),
            ("hos-geldin", p.welcome),
            ("durup-dururken", p.out_of_the_blue),
            ("haber-tanit", p.news_intro),
            ("analist", p.analyst),
            ("isteklilik", p.willingness),
            ("hedef-sec", p.target_pick),
            ("uyanis", p.waking),
            ("uyanis-cevap", p.waking_reply),
            ("profil-cikar", p.profile_extract),
            ("haber-sec", p.news_pick),
            ("hoca", p.coach),
            ("elestirmen", p.critic),
            ("resim-at", p.image_post),
            ("hack-giris", p.hack_enter),
            ("hack-devam", p.hack_continue),
            ("hack-cikis", p.hack_exit),
            ("gunlukcu", p.diarist),
            ("ozetleyici-kisi", p.summarizer_person),
            ("ozetleyici-konu", p.summarizer_topic),
            ("ozetleyici-olaylar", p.summarizer_events),
            ("gezgin-sec", p.wanderer_pick),
            ("gezgin-not", p.wanderer_note),
            ("uyandim", p.woke_up),
            ("yolda", p.on_the_way),
            ("gidiyorum", p.leaving),
            ("isim-sec", p.name_pick),
            ("isim-duyuru", p.name_announce),
            ("sorun", p.problem),
            ("ruh-hali", p.mood),
        ]
    }

    #[test]
    fn every_discord_prompt_matches_complete_original_body_in_both_languages() {
        let context = PromptContext::for_platform(PlatformKind::Discord);
        for (lang, fixture) in [
            (
                Lang::En,
                include_str!("../../prompts/en/discord-golden.json"),
            ),
            (
                Lang::Tr,
                include_str!("../../prompts/tr/discord-golden.json"),
            ),
        ] {
            let originals: BTreeMap<String, String> = serde_json::from_str(fixture).unwrap();
            assert_eq!(originals.len(), 31);
            for (name, template) in templates(prompts::get(lang)) {
                assert_eq!(context.render(template), originals[name], "{name}");
                let values = [
                    ("{ad}", "Ada"),
                    ("{favori_satiri}", "favorite"),
                    ("{favori}", "Emin"),
                ];
                let expected = originals[name]
                    .replace("{ad}", "Ada")
                    .replace("{favori_satiri}", "favorite")
                    .replace("{favori}", "Emin");
                assert_eq!(context.render_with(template, &values), expected, "{name}");
            }
        }
    }

    #[test]
    fn non_discord_full_bodies_match_localized_capability_fixtures() {
        let context = PromptContext::for_platform(PlatformKind::Slack);
        for (lang, fixture) in [
            (
                Lang::En,
                include_str!("../../prompts/en/platform-golden.json"),
            ),
            (
                Lang::Tr,
                include_str!("../../prompts/tr/platform-golden.json"),
            ),
        ] {
            let expected: BTreeMap<String, String> = serde_json::from_str(fixture).unwrap();
            assert_eq!(expected.len(), 5);
            for (name, template) in templates(prompts::get(lang)) {
                if let Some(body) = expected.get(name) {
                    assert_eq!(context.render(template), body.as_str(), "{name}");
                }
            }
        }
    }

    #[test]
    fn native_labels_identity_and_history_limits_are_localized() {
        for platform in [
            PlatformKind::Telegram,
            PlatformKind::Slack,
            PlatformKind::Matrix,
            PlatformKind::Terminal,
        ] {
            let context = PromptContext::for_platform(platform);
            for lang in [Lang::En, Lang::Tr] {
                let p = prompts::get(lang);
                for (_, template) in templates(p) {
                    let rendered = context.render(template);
                    assert!(!rendered.contains("{platform_"));
                    assert!(!rendered.to_lowercase().contains("discord"));
                    assert!(!rendered.contains("server"));
                    assert!(!rendered.contains("sunucu"));
                }
                let personality = context.render(p.personality);
                let history = context.render(p.profile_extract);
                match lang {
                    Lang::En => {
                        assert!(personality.contains("You are an AI"));
                        assert!(personality.contains("never deny being AI"));
                        assert!(personality.contains("source-qualified identities"));
                        assert!(history.contains("limited or only prospective"));
                    }
                    Lang::Tr => {
                        assert!(personality.contains("Sen bir yapay zeka"));
                        assert!(personality.contains("yapay zeka olduğunu asla inkar etme"));
                        assert!(personality.contains("kaynakla\nnitelenmiş kimliklerle"));
                        assert!(history.contains("Geçmiş sınırlı olabilir"));
                    }
                }
                let label = match platform {
                    PlatformKind::Telegram => "Telegram",
                    PlatformKind::Slack => "Slack",
                    PlatformKind::Matrix => "Matrix",
                    PlatformKind::Terminal => "terminal",
                    PlatformKind::Discord => unreachable!(),
                };
                assert!(personality.contains(label));
                assert!(history.contains(label));
            }
        }
    }

    #[test]
    fn absent_reaction_capability_removes_requests_and_critic_expectations() {
        for lang in [Lang::En, Lang::Tr] {
            let p = prompts::get(lang);
            let context = PromptContext::for_platform(PlatformKind::Discord)
                .with_capabilities(&Capabilities::unavailable());
            assert!(!context.render(p.personality).contains("tepki:"));
            assert!(!context.render(p.critic).contains("tepki:"));
            assert!(!context.render(p.profile_extract).contains("last two weeks"));
            assert!(!context
                .render(p.profile_extract)
                .contains("son iki haftası"));
        }
    }

    #[test]
    fn replacements_are_literal_and_unmodified_inputs_are_borrowed() {
        let context = PromptContext::for_platform(PlatformKind::Discord);
        assert!(matches!(context.render("plain prose"), Cow::Borrowed(_)));
        assert!(matches!(context.render("unknown {slot}"), Cow::Borrowed(_)));
        assert!(matches!(
            context.render_with("{ad}", &[("{ad}", "{ad}")]),
            Cow::Borrowed(_)
        ));
        assert_eq!(
            context.render_with(
                "{ad} {favori}",
                &[("{ad}", "{favori}"), ("{favori}", "Emin")]
            ),
            "{favori} Emin"
        );
        assert!(context
            .render_with(prompts::get(Lang::En).personality, &[("{ad}", "{favori}")])
            .starts_with(
                "# Personality (system message on every reply)\n\nYour name is {favori}."
            ));
    }
}
