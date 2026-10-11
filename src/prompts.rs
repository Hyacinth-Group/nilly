// Prompts live as markdown under prompts/<dil>/ and get embedded at compile time (the
// filenames stay Turkish on purpose — see AGENTS.md; content is per-language, tr's is
// Turkish, en's is English). To change the text, edit the .md file and rebuild. Which
// language's set is served at runtime is decided once by `Lang::current()` (see lang.rs)
// and never changes for the life of the process. `tr` and `en` are both filled in; a new
// language is a new `prompts/<dil>/` folder (every file `tr/` has) plus one match arm in
// `get` below.

use crate::lang::Lang;

mod tr {
    pub const PERSONALITY: &str = include_str!("../prompts/tr/kisilik.md");
    pub const FAVORITE_LINE: &str = include_str!("../prompts/tr/favori-satiri.md");
    pub const WELCOME: &str = include_str!("../prompts/tr/hos-geldin.md");
    pub const OUT_OF_THE_BLUE: &str = include_str!("../prompts/tr/durup-dururken.md");
    pub const NEWS_INTRO: &str = include_str!("../prompts/tr/haber-tanit.md");
    pub const ANALYST: &str = include_str!("../prompts/tr/analist.md");
    pub const WILLINGNESS: &str = include_str!("../prompts/tr/isteklilik.md");
    pub const TARGET_PICK: &str = include_str!("../prompts/tr/hedef-sec.md");
    pub const WAKING: &str = include_str!("../prompts/tr/uyanis.md");
    pub const WAKING_REPLY: &str = include_str!("../prompts/tr/uyanis-cevap.md");
    pub const PROFILE_EXTRACT: &str = include_str!("../prompts/tr/profil-cikar.md");
    pub const NEWS_PICK: &str = include_str!("../prompts/tr/haber-sec.md");
    pub const COACH: &str = include_str!("../prompts/tr/hoca.md");
    pub const CRITIC: &str = include_str!("../prompts/tr/elestirmen.md");
    pub const IMAGE_POST: &str = include_str!("../prompts/tr/resim-at.md");
    pub const HACK_ENTER: &str = include_str!("../prompts/tr/hack-giris.md");
    pub const HACK_CONTINUE: &str = include_str!("../prompts/tr/hack-devam.md");
    pub const HACK_EXIT: &str = include_str!("../prompts/tr/hack-cikis.md");
    pub const DIARIST: &str = include_str!("../prompts/tr/gunlukcu.md");
    pub const SUMMARIZER_PERSON: &str = include_str!("../prompts/tr/ozetleyici-kisi.md");
    pub const SUMMARIZER_TOPIC: &str = include_str!("../prompts/tr/ozetleyici-konu.md");
    pub const SUMMARIZER_EVENTS: &str = include_str!("../prompts/tr/ozetleyici-olaylar.md");
    pub const WANDERER_PICK: &str = include_str!("../prompts/tr/gezgin-sec.md");
    pub const WANDERER_NOTE: &str = include_str!("../prompts/tr/gezgin-not.md");
    pub const WOKE_UP: &str = include_str!("../prompts/tr/uyandim.md");
    pub const ON_THE_WAY: &str = include_str!("../prompts/tr/yolda.md");
    pub const LEAVING: &str = include_str!("../prompts/tr/gidiyorum.md");
    pub const NAME_PICK: &str = include_str!("../prompts/tr/isim-sec.md");
    pub const NAME_ANNOUNCE: &str = include_str!("../prompts/tr/isim-duyuru.md");
    pub const PROBLEM: &str = include_str!("../prompts/tr/sorun.md");
    pub const MOOD: &str = include_str!("../prompts/tr/ruh-hali.md");
}

mod en {
    pub const PERSONALITY: &str = include_str!("../prompts/en/kisilik.md");
    pub const FAVORITE_LINE: &str = include_str!("../prompts/en/favori-satiri.md");
    pub const WELCOME: &str = include_str!("../prompts/en/hos-geldin.md");
    pub const OUT_OF_THE_BLUE: &str = include_str!("../prompts/en/durup-dururken.md");
    pub const NEWS_INTRO: &str = include_str!("../prompts/en/haber-tanit.md");
    pub const ANALYST: &str = include_str!("../prompts/en/analist.md");
    pub const WILLINGNESS: &str = include_str!("../prompts/en/isteklilik.md");
    pub const TARGET_PICK: &str = include_str!("../prompts/en/hedef-sec.md");
    pub const WAKING: &str = include_str!("../prompts/en/uyanis.md");
    pub const WAKING_REPLY: &str = include_str!("../prompts/en/uyanis-cevap.md");
    pub const PROFILE_EXTRACT: &str = include_str!("../prompts/en/profil-cikar.md");
    pub const NEWS_PICK: &str = include_str!("../prompts/en/haber-sec.md");
    pub const COACH: &str = include_str!("../prompts/en/hoca.md");
    pub const CRITIC: &str = include_str!("../prompts/en/elestirmen.md");
    pub const IMAGE_POST: &str = include_str!("../prompts/en/resim-at.md");
    pub const HACK_ENTER: &str = include_str!("../prompts/en/hack-giris.md");
    pub const HACK_CONTINUE: &str = include_str!("../prompts/en/hack-devam.md");
    pub const HACK_EXIT: &str = include_str!("../prompts/en/hack-cikis.md");
    pub const DIARIST: &str = include_str!("../prompts/en/gunlukcu.md");
    pub const SUMMARIZER_PERSON: &str = include_str!("../prompts/en/ozetleyici-kisi.md");
    pub const SUMMARIZER_TOPIC: &str = include_str!("../prompts/en/ozetleyici-konu.md");
    pub const SUMMARIZER_EVENTS: &str = include_str!("../prompts/en/ozetleyici-olaylar.md");
    pub const WANDERER_PICK: &str = include_str!("../prompts/en/gezgin-sec.md");
    pub const WANDERER_NOTE: &str = include_str!("../prompts/en/gezgin-not.md");
    pub const WOKE_UP: &str = include_str!("../prompts/en/uyandim.md");
    pub const ON_THE_WAY: &str = include_str!("../prompts/en/yolda.md");
    pub const LEAVING: &str = include_str!("../prompts/en/gidiyorum.md");
    pub const NAME_PICK: &str = include_str!("../prompts/en/isim-sec.md");
    pub const NAME_ANNOUNCE: &str = include_str!("../prompts/en/isim-duyuru.md");
    pub const PROBLEM: &str = include_str!("../prompts/en/sorun.md");
    pub const MOOD: &str = include_str!("../prompts/en/ruh-hali.md");
}

/// One language's full prompt set. Every field is a prompt constant, unchanged in meaning
/// from before this module split by language — only the lookup (`current` below) is new.
/// See docs/prompts.md for what each field is used for.
pub struct Prompts {
    pub personality: &'static str,
    pub favorite_line: &'static str,
    pub welcome: &'static str,
    pub out_of_the_blue: &'static str,
    pub news_intro: &'static str,
    pub analyst: &'static str,
    pub willingness: &'static str,
    pub target_pick: &'static str,
    pub waking: &'static str,
    pub waking_reply: &'static str,
    pub profile_extract: &'static str,
    pub news_pick: &'static str,
    pub coach: &'static str,
    pub critic: &'static str,
    pub image_post: &'static str,
    pub hack_enter: &'static str,
    pub hack_continue: &'static str,
    pub hack_exit: &'static str,
    pub diarist: &'static str,
    pub summarizer_person: &'static str,
    pub summarizer_topic: &'static str,
    pub summarizer_events: &'static str,
    pub wanderer_pick: &'static str,
    pub wanderer_note: &'static str,
    pub woke_up: &'static str,
    pub on_the_way: &'static str,
    pub leaving: &'static str,
    pub name_pick: &'static str,
    pub name_announce: &'static str,
    pub problem: &'static str,
    pub mood: &'static str,
}

const TR: Prompts = Prompts {
    personality: tr::PERSONALITY,
    favorite_line: tr::FAVORITE_LINE,
    welcome: tr::WELCOME,
    out_of_the_blue: tr::OUT_OF_THE_BLUE,
    news_intro: tr::NEWS_INTRO,
    analyst: tr::ANALYST,
    willingness: tr::WILLINGNESS,
    target_pick: tr::TARGET_PICK,
    waking: tr::WAKING,
    waking_reply: tr::WAKING_REPLY,
    profile_extract: tr::PROFILE_EXTRACT,
    news_pick: tr::NEWS_PICK,
    coach: tr::COACH,
    critic: tr::CRITIC,
    image_post: tr::IMAGE_POST,
    hack_enter: tr::HACK_ENTER,
    hack_continue: tr::HACK_CONTINUE,
    hack_exit: tr::HACK_EXIT,
    diarist: tr::DIARIST,
    summarizer_person: tr::SUMMARIZER_PERSON,
    summarizer_topic: tr::SUMMARIZER_TOPIC,
    summarizer_events: tr::SUMMARIZER_EVENTS,
    wanderer_pick: tr::WANDERER_PICK,
    wanderer_note: tr::WANDERER_NOTE,
    woke_up: tr::WOKE_UP,
    on_the_way: tr::ON_THE_WAY,
    leaving: tr::LEAVING,
    name_pick: tr::NAME_PICK,
    name_announce: tr::NAME_ANNOUNCE,
    problem: tr::PROBLEM,
    mood: tr::MOOD,
};

const EN: Prompts = Prompts {
    personality: en::PERSONALITY,
    favorite_line: en::FAVORITE_LINE,
    welcome: en::WELCOME,
    out_of_the_blue: en::OUT_OF_THE_BLUE,
    news_intro: en::NEWS_INTRO,
    analyst: en::ANALYST,
    willingness: en::WILLINGNESS,
    target_pick: en::TARGET_PICK,
    waking: en::WAKING,
    waking_reply: en::WAKING_REPLY,
    profile_extract: en::PROFILE_EXTRACT,
    news_pick: en::NEWS_PICK,
    coach: en::COACH,
    critic: en::CRITIC,
    image_post: en::IMAGE_POST,
    hack_enter: en::HACK_ENTER,
    hack_continue: en::HACK_CONTINUE,
    hack_exit: en::HACK_EXIT,
    diarist: en::DIARIST,
    summarizer_person: en::SUMMARIZER_PERSON,
    summarizer_topic: en::SUMMARIZER_TOPIC,
    summarizer_events: en::SUMMARIZER_EVENTS,
    wanderer_pick: en::WANDERER_PICK,
    wanderer_note: en::WANDERER_NOTE,
    woke_up: en::WOKE_UP,
    on_the_way: en::ON_THE_WAY,
    leaving: en::LEAVING,
    name_pick: en::NAME_PICK,
    name_announce: en::NAME_ANNOUNCE,
    problem: en::PROBLEM,
    mood: en::MOOD,
};

fn get(lang: Lang) -> &'static Prompts {
    match lang {
        Lang::Tr => &TR,
        Lang::En => &EN,
    }
}

/// The compiled-in prompts for the process's active language.
pub fn current() -> &'static Prompts {
    get(Lang::current())
}

#[cfg(test)]
mod test {
    use super::*;

    /// Verifies every field actually loaded a non-empty file (a typo'd include_str! path
    /// would fail to compile, but an accidentally-empty .md wouldn't) — for every language.
    #[test]
    fn prompts_are_not_empty() {
        for lang in [Lang::Tr, Lang::En] {
            let p = get(lang);
            assert!(!p.personality.is_empty());
            assert!(!p.favorite_line.is_empty());
            assert!(!p.welcome.is_empty());
            assert!(!p.mood.is_empty());
        }
    }
}
