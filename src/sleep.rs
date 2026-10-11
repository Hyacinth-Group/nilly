// Sleep schedule: local time from TZ_OFFSET. Normally asleep at 01:00, up at 09:00 (±45 min jitter).
// Rarely, a sleepless night: up from 01-06, then asleep 06-13 instead. The odds of
// insomnia scale with the bot's mood: more likely when it's hurt, on edge, or fixated.
// No writing, no joining in, no news while asleep; a mention gets a reply once it wakes up.

use super::*;

/// Parses hours from UTC into seconds, including fractional offsets.
pub fn parse_offset(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Some(0);
    }
    let hours: f64 = raw.parse().ok()?;
    (-12.0..=14.0)
        .contains(&hours)
        .then(|| (hours * 3600.0).round() as i64)
}

/// The process-wide local offset from TZ_OFFSET, defaulting to UTC.
pub fn timezone_offset() -> i64 {
    static OFFSET: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    let mut invalid = None;
    let offset = *OFFSET.get_or_init(|| {
        let raw = std::env::var("TZ_OFFSET").unwrap_or_default();
        match parse_offset(&raw) {
            Some(offset) => offset,
            None => {
                invalid = Some(raw);
                0
            }
        }
    });
    // Logging reads local time, so initialize the offset before warning.
    if let Some(raw) = invalid {
        log::warn!("invalid TZ_OFFSET '{raw}', using 0 (UTC)");
    }
    offset
}

pub const INSOMNIA_CHANCE: f64 = 0.07; // an ordinary night
pub const INSOMNIA_TENSE: f64 = 0.20; // when its mood is off

/// One night's sleep plan (unix seconds). Holds `day` (local day number the plan is for),
/// `insomnia_start` (if a sleepless night, when it stayed up from), `start`/`end` (the
/// actual sleep window). Built by `build_plan` below; stored in `State.plans`.
#[derive(Clone, Copy)]
pub struct Plan {
    pub day: i64,                    // which day's night (local day number)
    pub insomnia_start: Option<i64>, // if a sleepless night, when it stayed up from (unix)
    pub start: i64,                  // sleep start (unix)
    pub end: i64,                    // sleep end (unix)
}

/// Input: `unix: i64`. Output: `(i64, i64)` — (local day number, seconds since local
/// midnight), applying `timezone_offset()`. Used by: `time`/`time_of_day`/`time_text`/
/// `update` below.
pub fn local_time(unix: i64) -> (i64, i64) {
    let local = unix + timezone_offset();
    (local.div_euclid(86400), local.rem_euclid(86400))
}

/// Input: none. Output: `String` — current local time, `"HH:MM"`. Uses: `local_time`,
/// `now_unix`. Used by: `time_text` below, `logging.rs`'s `Sink::log`.
pub fn time() -> String {
    let (_, secs) = local_time(now_unix());
    format!("{:02}:{:02}", secs / 3600, (secs % 3600) / 60)
}

/// Input: none. Output: `String` — current local time, `"HH:MM:SS"`. Uses: `local_time`,
/// `now_unix`. Used by: `memory::date_time`, the only caller.
pub fn time_of_day() -> String {
    let (_, secs) = local_time(now_unix());
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// Input: none. Output: `String` — `"YYYY-MM-DD <Turkish weekday> HH:MM"`. Uses:
/// `local_time`, `memory::date_from_unix`, `time`. Used by: `status_text` below.
pub fn time_text() -> String {
    let (day, _) = local_time(now_unix());
    let names = [
        "perşembe",
        "cuma",
        "cumartesi",
        "pazar",
        "pazartesi",
        "salı",
        "çarşamba",
    ];
    format!(
        "{} {} {}",
        memory::date_from_unix(now_unix() + timezone_offset()),
        names[day.rem_euclid(7) as usize],
        time()
    )
}

/// Input: none. Output: `i64` — a random offset in `[-2700, 2700)` seconds (±45 min). Used
/// by: `build_plan` below, for both the start and end of a sleep window.
fn jitter() -> i64 {
    (rand::random::<u32>() % 5400) as i64 - 2700 // ±45 min
}

/// Input: `state: &State`. Output: `bool` — whether `state.myself`/`temperament` contain a
/// tension-signaling Turkish word (hurt, on-edge, fixated, ...). Used by: `update` below,
/// to pick the insomnia chance.
fn is_tense(state: &State) -> bool {
    let lower = format!("{} {}", state.myself, state.temperament).to_lowercase();
    [
        "kırgın",
        "sinir",
        "gergin",
        "takıntı",
        "uyku",
        "kafayı",
        "bunalt",
    ]
    .iter()
    .any(|k| lower.contains(k))
}

/// Input: `day: i64` — the local day to plan for; `tense: bool` — from `is_tense`. Output:
/// `Plan` — either a normal night (01:00±45→09:00±45) or, with `INSOMNIA_TENSE`/
/// `INSOMNIA_CHANCE` odds, a sleepless one (up 01:00-06:00, asleep 06:00±45→13:00±45).
/// Uses: `jitter`. Used by: `update` below, the only caller.
fn build_plan(day: i64, tense: bool) -> Plan {
    let night = (day + 1) * 86400 - timezone_offset(); // next day's 00:00, unix
    let chance = if tense {
        INSOMNIA_TENSE
    } else {
        INSOMNIA_CHANCE
    };
    if rand::random::<f64>() < chance {
        Plan {
            day,
            insomnia_start: Some(night + 3600),
            start: night + 6 * 3600 + jitter(),
            end: night + 13 * 3600 + jitter(),
        }
    } else {
        Plan {
            day,
            insomnia_start: None,
            start: night + 3600 + jitter(),
            end: night + 9 * 3600 + jitter(),
        }
    }
}

// builds a plan for tonight and last night if missing, drops ones that are over
/// Input: `state: &mut State`. Output: none (adds today's/yesterday's `Plan` if missing,
/// drops expired ones from `state.plans`). Uses: `local_time`, `build_plan`, `is_tense`,
/// `memory::date_from_unix`. Used by: `Bot::setup` (`setup.rs`), `sleep_cycle`
/// (`cycle_background.rs`), the only callers.
pub fn update(state: &mut State) {
    let now = now_unix();
    let (today, _) = local_time(now);
    for day in [today - 1, today] {
        if !state.plans.iter().any(|p| p.day == day) {
            let plan = build_plan(day, is_tense(state));
            if plan.insomnia_start.is_some() {
                log::info!(
                    "sleep: night of {} will be sleepless",
                    memory::date_from_unix(day * 86400)
                );
            }
            state.plans.push(plan);
        }
    }
    state.plans.retain(|plan| plan.end > now);
}

/// Input: `state: &State`. Output: `bool` — `true` if `state.forced_awake_until` covers now
/// (`!uyan` was used) or no plan's window contains now. Used throughout the crate wherever
/// asleep/awake behavior branches (`Handler::message`, the background cycles, `modal.rs`).
pub fn is_awake(state: &State) -> bool {
    let now = now_unix();
    if now < state.forced_awake_until {
        return true; // !uyan was used
    }
    !state
        .plans
        .iter()
        .any(|plan| plan.start <= now && now < plan.end)
}

// the "right now" line that goes into the system message
/// Input: `state: &State` (currently unused — kept for a symmetric signature with
/// `travel::status_text`). Output: `String` — the "ŞU AN" system-message line. Uses:
/// `time_text`. Used by: `system_text` (`provider_system.rs`), `modal.rs`'s embed builders.
pub fn status_text(state: &State) -> String {
    let _ = state;
    time_text()
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parse_offset_bounds() {
        assert_eq!(parse_offset(""), Some(0));
        assert_eq!(parse_offset("3"), Some(10800));
        assert_eq!(parse_offset("-5"), Some(-18000));
        assert_eq!(parse_offset("5.5"), Some(19800));
        assert_eq!(parse_offset("15"), None);
        assert_eq!(parse_offset("abc"), None);
    }
}
