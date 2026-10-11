// Growth stages: the bot advances a stage based on days spent on the server and chats
// it's finished. A stage adds a "what you're like at this stage" line to the system
// message and scales the odds of joining in / poking at conversation. On entering the
// established stage, it picks its own name and changes its nickname.
// Counters live in durum/gelisim.md; on restart it picks up where it left off.

use super::*;

/// One entry of `STAGES`: the thresholds to reach it and what it changes about the bot.
/// Holds `name` (Turkish stage id, also the `durum/gelisim.md` value and the "GELİŞİM EVREN"
/// system-message label), `min_days`/`min_chats` (thresholds `earned_stage` checks against),
/// `confidence` (multiplies the willingness threshold, see `handler_event.rs`'s `message`),
/// `poke` (multiplies `POKE_CHANCE`, see `poke_cycle`), `description` (Turkish personality
/// text folded into the system message by `stage_text`).
pub struct Stage {
    pub name: &'static str,
    pub min_days: i64,   // at least this many days since birth
    pub min_chats: u32,  // at least this many finished chats
    pub confidence: f64, // multiplier on the willingness threshold (joining in)
    pub poke: f64,       // multiplier on POKE_CHANCE
    pub description: &'static str,
}

pub const NAME_STAGE: usize = 2; // picks a name on reaching this stage (established)

pub const STAGES: &[Stage] = &[
    Stage {
        name: "yeni",
        min_days: 0,
        min_chats: 0,
        confidence: 0.7,
        poke: 0.4,
        description: "Sunucuya yeni geldin. Herkesi tanımıyorsun; az konuş, çok dinle, soru sor, iç şakalara \
                   henüz girme, yanlış anlamaktan çekin. Kendine bir yer arıyorsun, biraz temkinlisin.",
    },
    Stage {
        name: "isinma",
        min_days: 3,
        min_chats: 8,
        confidence: 0.8,
        poke: 0.7,
        description: "Isınıyorsun: birkaç kişiyi tanıdın, arada laf sokmaya başladın ama sınırları hâlâ \
                   yokluyorsun. İlk iç şakaları kapıyorsun, bazen yanlış yere gülüyorsun.",
    },
    Stage {
        name: "yerlesik",
        min_days: 10,
        min_chats: 25,
        confidence: 1.0,
        poke: 1.0,
        description: "Artık buranın parçasısın: kendi kalıpların, sevdiklerin ve sevmediklerin belli. \
                   Bu evreye girerken kendine bir isim seçtin; o isimle anılıyorsun.",
    },
    Stage {
        name: "eski-toprak",
        min_days: 30,
        min_chats: 80,
        confidence: 1.0,
        poke: 1.2,
        description: "Eski toprak: geçmişe gönderme yapan, yeni gelenlere burayı anlatan, gerektiğinde \
                   susmayı bilen biri. Hikâyen var, herkes seni tanıyor, sen de onları.",
    },
];

/// The bot's persisted growth counters. Holds `birth` (first-ever run, unix seconds),
/// `chats`/`messages` (lifetime counts), `stage` (index into `STAGES`, only moves forward),
/// `name` (self-chosen once `stage >= NAME_STAGE`). Lives at `State.growth`; persisted to
/// `durum/gelisim.md` by `load`/`save` below.
#[derive(Default, Clone)]
pub struct Growth {
    pub birth: i64, // moment of the very first run (unix)
    pub chats: u32,
    pub messages: u32,
    pub stage: usize, // index into STAGES, only moves forward
    pub name: Option<String>,
}

/// Loads saved growth state, starting a fresh birth timestamp if the record is missing/empty.
pub fn load() -> Growth {
    let mut growth = Growth {
        birth: now_unix(),
        ..Default::default()
    };
    for line in memory::read("gelisim.md").lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "dogum" => growth.birth = v.parse().unwrap_or(growth.birth),
            "sohbet" => growth.chats = v.parse().unwrap_or(0),
            "mesaj" => growth.messages = v.parse().unwrap_or(0),
            "evre" => growth.stage = v.parse::<usize>().unwrap_or(0).min(STAGES.len() - 1),
            "isim" if !v.is_empty() => growth.name = Some(v.to_string()),
            _ => {}
        }
    }
    growth
}

pub fn save(growth: &Growth) {
    memory::write(
        "gelisim.md",
        &format!(
            "dogum: {}\nsohbet: {}\nmesaj: {}\nevre: {}\nisim: {}\n",
            growth.birth,
            growth.chats,
            growth.messages,
            growth.stage,
            growth.name.as_deref().unwrap_or("")
        ),
    );
}

/// Returns whole elapsed days since the bot's first run.
pub fn days(growth: &Growth) -> i64 {
    (now_unix() - growth.birth) / 86400
}

// the earned stage: the highest stage that clears both the day and chat thresholds
pub fn earned_stage(growth: &Growth) -> usize {
    let d = days(growth);
    STAGES
        .iter()
        .enumerate()
        .filter(|(_, s)| d >= s.min_days && growth.chats >= s.min_chats)
        .map(|(i, _)| i)
        .max()
        .unwrap_or(0)
}

/// Clamps a persisted stage index to the available stages.
pub fn stage(growth: &Growth) -> &'static Stage {
    &STAGES[growth.stage.min(STAGES.len() - 1)]
}

// the section that goes into the system message
pub fn stage_text(growth: &Growth) -> String {
    let s = stage(growth);
    format!(
        "{} evresi ({}. gün, {} sohbet). {}",
        s.name,
        days(growth) + 1,
        growth.chats,
        s.description
    )
}

// collapses the model's name suggestion down to a single word; None if it didn't work out
/// Keeps the first word's alphanumeric characters, capped at 20; requires at least two.
pub fn clean_name(text: &str) -> Option<String> {
    let candidate: String = text
        .split_whitespace()
        .next()?
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(20)
        .collect();
    (candidate.chars().count() >= 2).then_some(candidate)
}

#[cfg(test)]
mod test {
    use super::*;

    /// Verifies `earned_stage` requires both enough elapsed days and enough chats to advance a stage.
    #[test]
    fn stage_thresholds() {
        let mut growth = Growth {
            birth: now_unix() - 11 * 86400,
            chats: 30,
            ..Default::default()
        };
        assert_eq!(STAGES[earned_stage(&growth)].name, "yerlesik");
        growth.chats = 5;
        assert_eq!(STAGES[earned_stage(&growth)].name, "yeni");
        growth.chats = 100;
        assert_eq!(STAGES[earned_stage(&growth)].name, "yerlesik"); // not enough days
    }

    /// Verifies `clean_name` extracts a plausible single name from a free-text reply and rejects junk.
    #[test]
    fn name_gets_cleaned() {
        assert_eq!(clean_name("\"Kaju\"").as_deref(), Some("Kaju"));
        assert_eq!(
            clean_name("Bundan sonra Zeytin de").as_deref(),
            Some("Bundan")
        );
        assert!(clean_name("!").is_none());
    }
}
