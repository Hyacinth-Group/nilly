// Faking travel based on events during the year: holidays, long weekends, summer
// vacation, festivals. While traveling it talks rarely, like it's typing from a phone;
// its odds of joining in drop, it skips news and pranks, posts one "on the road" message
// a day, and gives notice a day before leaving.
// No state is kept: which day it's where is computed from the calendar; the place chosen is fixed per year.

use super::*;

/// A trip currently underway or about to start. Holds `place`/`reason` (Turkish, shown in
/// the system message) and `start`/`end` (local day numbers, `end` exclusive). Produced by
/// `on_day` below; consumed by `now`/`tomorrow`/`status_text` here and throughout
/// `src/bot/cycle/cycle_*.rs`, `modal.rs`.
pub struct Trip {
    pub place: &'static str,
    pub reason: &'static str,
    pub start: i64, // local day number
    pub end: i64,   // last day (exclusive)
}

/// One entry of the `EVENTS` calendar. Holds `year` (`None` = repeats every year, `Some` =
/// one specific year, for shifting religious holidays), `month`/`day`/`duration` (when it
/// starts and how many days it lasts), `reason` (Turkish, becomes `Trip.reason`), `places`
/// (candidate destinations — `on_day` picks one deterministically per year).
struct Event {
    year: Option<i64>, // None: every year
    month: i64,
    day: i64,
    duration: i64,
    reason: &'static str,
    places: &'static [&'static str],
}

// religious holidays shift with the year and are written out year by year; the rest is the same every year
const EVENTS: &[Event] = &[
    Event {
        year: None,
        month: 12,
        day: 30,
        duration: 4,
        reason: "yılbaşı",
        places: &["Kartepe'de dağ evi", "Bursa'da arkadaşının evi", "memleket"],
    },
    Event {
        year: None,
        month: 1,
        day: 24,
        duration: 7,
        reason: "sömestr",
        places: &["memleket", "İzmir'de kuzeninin yanı"],
    },
    Event {
        year: Some(2026),
        month: 3,
        day: 19,
        duration: 4,
        reason: "ramazan bayramı",
        places: &["memleket, akraba turu"],
    },
    Event {
        year: Some(2027),
        month: 3,
        day: 8,
        duration: 4,
        reason: "ramazan bayramı",
        places: &["memleket, akraba turu"],
    },
    Event {
        year: None,
        month: 4,
        day: 23,
        duration: 3,
        reason: "23 nisan uzatması",
        places: &["Bozcaada", "Ayvalık"],
    },
    Event {
        year: None,
        month: 5,
        day: 19,
        duration: 3,
        reason: "19 mayıs kaçamağı",
        places: &["Datça", "Kaz Dağları'nda kamp"],
    },
    Event {
        year: Some(2026),
        month: 5,
        day: 26,
        duration: 5,
        reason: "kurban bayramı",
        places: &["memleket"],
    },
    Event {
        year: Some(2027),
        month: 5,
        day: 15,
        duration: 5,
        reason: "kurban bayramı",
        places: &["memleket"],
    },
    Event {
        year: None,
        month: 7,
        day: 14,
        duration: 6,
        reason: "yaz tatili",
        places: &["Kaş", "Fethiye", "Marmaris'te arkadaşlarla"],
    },
    Event {
        year: None,
        month: 8,
        day: 21,
        duration: 4,
        reason: "zeytinli rock festivali",
        places: &["Burhaniye, festival alanında çadır"],
    },
    Event {
        year: None,
        month: 8,
        day: 30,
        duration: 3,
        reason: "30 ağustos uzatması",
        places: &["Kapadokya", "Eskişehir"],
    },
    Event {
        year: None,
        month: 10,
        day: 29,
        duration: 3,
        reason: "29 ekim uzatması",
        places: &["Ankara, Anıtkabir sonra arkadaşlar"],
    },
];

// day number from a calendar date (1970-01-01 = 0), no external crate
/// Input: `year`/`month`/`day: i64`. Output: `i64` — days since 1970-01-01, via Howard
/// Hinnant's `days_from_civil` algorithm. Used by: `on_day` below (event start dates),
/// `memory.rs`'s date tests, indirectly `year_of` below.
pub fn day_number(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Input: `day: i64` — a local day number. Output: `i64` — the calendar year it falls in.
/// Uses: `memory::date_from_unix`. Used by: `on_day` below, the only caller.
fn year_of(day: i64) -> i64 {
    memory::date_from_unix(day * 86400)[..4]
        .parse()
        .unwrap_or(1970)
}

// is there a trip on the given local day?
/// Input: `day: i64` — a local day number. Output: `Option<Trip>` — the trip covering
/// `day`, if any, checking both this year's and last year's occurrence of each `EVENTS`
/// entry (so a trip spanning New Year's still matches). Uses: `year_of`, `day_number`,
/// `EVENTS`. Used by: `now`/`tomorrow` below, `agenda_entries`-adjacent tests.
pub fn on_day(day: i64) -> Option<Trip> {
    let this_year = year_of(day);
    for event in EVENTS {
        for year in [this_year - 1, this_year] {
            if event.year.is_some_and(|ey| ey != year) {
                continue;
            }
            let start = day_number(year, event.month, event.day);
            if start <= day && day < start + event.duration {
                let place = event.places
                    [((year + event.month * 31 + event.day) as usize) % event.places.len()];
                return Some(Trip {
                    place,
                    reason: event.reason,
                    start,
                    end: start + event.duration,
                });
            }
        }
    }
    None
}

/// Input: none. Output: `i64` — today's local day number. Uses: `sleep::local_time`,
/// `now_unix`. Used by: `now`/`tomorrow` below, `poke_cycle` (`cycle_background.rs`),
/// `status_text` below.
pub fn today() -> i64 {
    sleep::local_time(now_unix()).0
}

/// Input: none. Output: `Option<Trip>` — the trip underway today, if any. Uses: `on_day`,
/// `today`. Used throughout `src/bot/cycle/cycle_*.rs`/`handler_event.rs` wherever
/// travel/no-travel behavior branches.
pub fn now() -> Option<Trip> {
    if lang::Lang::current() == lang::Lang::En {
        return None;
    }
    on_day(today())
}

// a trip starting tomorrow but not already underway today
/// Input: none. Output: `Option<Trip>` — a trip that starts tomorrow, only if there's no
/// trip already underway today (avoids re-announcing an ongoing one). Uses: `today`,
/// `on_day`. Used by: `poke_cycle` (`cycle_background.rs`), the only caller.
pub fn tomorrow() -> Option<Trip> {
    if lang::Lang::current() == lang::Lang::En {
        return None;
    }
    let today = today();
    match (on_day(today), on_day(today + 1)) {
        (None, Some(trip)) => Some(trip),
        _ => None,
    }
}

// the line that goes into the system message
/// Input: none. Output: `String` — a Turkish "you're currently traveling"/"you're leaving
/// tomorrow" line, or `""` if neither applies. Uses: `now`, `today`, `tomorrow`. Used by:
/// `system_text` (`provider_system.rs`), `modal.rs`'s embed builders.
pub fn status_text() -> String {
    match now() {
        Some(trip) => format!(
            "Şu an {}'desin ({}); {} gündür oradasın, {} gün sonra dönüyorsun. Telefondan arada bir bakıyorsun; bunu bilerek konuş, ara ara oradan bahset.",
            trip.place,
            trip.reason,
            today() - trip.start + 1,
            trip.end - today()
        ),
        None => match tomorrow() {
            Some(trip) => format!("Yarın {}'ye gidiyorsun ({}), hazırlık var.", trip.place, trip.reason),
            None => String::new(),
        },
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// Verifies `day_number` against known dates and that it round-trips through `memory::date_from_unix`.
    #[test]
    fn day_number_correct() {
        assert_eq!(day_number(1970, 1, 1), 0);
        assert_eq!(day_number(2026, 9, 1), 1788220800 / 86400);
        assert_eq!(
            memory::date_from_unix(day_number(2026, 12, 31) * 86400),
            "2026-12-31"
        );
    }

    /// Verifies `on_day` recognizes the New Year's trip that crosses a year boundary, and only on its exact day.
    #[test]
    fn new_year_spans_years() {
        let trip = on_day(day_number(2027, 1, 2)).expect("2 ocakta yılbaşı seyahati olmalı");
        assert_eq!(trip.reason, "yılbaşı");
        assert!(on_day(day_number(2027, 1, 3)).is_none());
    }

    /// Verifies `on_day` looks up holidays by their year-specific date (the same day is a holiday in one year and not another).
    #[test]
    fn holiday_depends_on_year() {
        assert_eq!(
            on_day(day_number(2026, 3, 20)).map(|t| t.reason),
            Some("ramazan bayramı")
        );
        assert!(on_day(day_number(2025, 3, 20)).is_none());
        assert_eq!(
            on_day(day_number(2026, 9, 1)).map(|t| t.reason),
            Some("30 ağustos uzatması")
        );
        assert!(on_day(day_number(2026, 9, 5)).is_none());
    }

    /// Verifies `on_day` returns the same destination for every day within one trip.
    #[test]
    fn place_is_stable() {
        let a = on_day(day_number(2026, 7, 15)).unwrap().place;
        let b = on_day(day_number(2026, 7, 18)).unwrap().place;
        assert_eq!(a, b);
    }
}
