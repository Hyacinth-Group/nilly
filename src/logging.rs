// Lightweight logging: `log` macros + a hand-written sink. Level comes from the
// LOG_LEVEL environment variable (default info); color is automatic on a terminal, or
// forced with LOG_COLOR=on|off. Only discord_bot's own records pass the level filter as
// configured; other crates' (serenity, reqwest, ...) internal events are shown only at
// warn/error, so they don't flood the console.

use log::{Level, LevelFilter, Log, Metadata, Record};
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// Atomic log-level scale: 0 (off) through 5 (trace).
fn filter_number(f: LevelFilter) -> u8 {
    match f {
        LevelFilter::Off => 0,
        LevelFilter::Error => 1,
        LevelFilter::Warn => 2,
        LevelFilter::Info => 3,
        LevelFilter::Debug => 4,
        LevelFilter::Trace => 5,
    }
}

/// Uses the same numeric scale as `filter_number`.
fn level_number(l: Level) -> u8 {
    match l {
        Level::Error => 1,
        Level::Warn => 2,
        Level::Info => 3,
        Level::Debug => 4,
        Level::Trace => 5,
    }
}

// 0=off 1=error 2=warn 3=info 4=debug 5=trace
static LEVEL: AtomicU8 = AtomicU8::new(3);
static COLOR: AtomicBool = AtomicBool::new(false);

const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";

/// Unset or unrecognized LOG_LEVEL values default to Info.
fn read_level() -> LevelFilter {
    match std::env::var("LOG_LEVEL")
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "error" => LevelFilter::Error,
        "warn" => LevelFilter::Warn,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        "kapali" | "off" => LevelFilter::Off,
        _ => LevelFilter::Info,
    }
}

fn level_color(l: Level) -> &'static str {
    match l {
        Level::Error => "\x1b[1;31m", // red, bold
        Level::Warn => "\x1b[33m",    // yellow
        Level::Info => "\x1b[32m",    // green
        Level::Debug | Level::Trace => DIM,
    }
}

fn is_our_target(target: &str) -> bool {
    target.starts_with("discord_bot")
}

/// The `log::Log` implementor installed as the global logger by `init`. Holds no data
/// (a unit struct); state lives in the `LEVEL`/`COLOR` statics above so it can be read
/// without a lock.
struct Sink;

impl Log for Sink {
    /// Applies the configured level to this crate; dependency logs are warn/error only.
    fn enabled(&self, metadata: &Metadata) -> bool {
        let passes = level_number(metadata.level()) <= LEVEL.load(Ordering::Relaxed);
        if is_our_target(metadata.target()) {
            passes
        } else {
            // serenity/reqwest/... internal events: only warnings and errors show
            passes && metadata.level() <= Level::Warn
        }
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let date = crate::memory::date();
        let time = crate::sleep::time();
        if COLOR.load(Ordering::Relaxed) {
            let code = level_color(record.level());
            println!(
                "{DIM}{date} {time}{RESET} {code}{:<5}{RESET} {code}{}{RESET}",
                record.level(),
                record.args()
            );
        } else {
            println!("{} {} {:<5} {}", date, time, record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

static SINK: Sink = Sink;

/// Installs `Sink` as the global logger and sets the level from `LOG_LEVEL`/color from
/// `LOG_COLOR`. Must run before any logging call.
pub fn init() {
    let level = read_level();
    LEVEL.store(filter_number(level), Ordering::Relaxed);
    log::set_max_level(level);
    let color = match std::env::var("LOG_COLOR")
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "on" | "acik" => true,
        "off" | "kapali" => false,
        _ => std::io::stdout().is_terminal(),
    };
    COLOR.store(color, Ordering::Relaxed);
    let _ = log::set_logger(&SINK);
}
