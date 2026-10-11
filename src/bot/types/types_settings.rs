// ---------- settings ----------

// providers: both are OpenAI-compatible chat/completions endpoints; whichever key is set
// in .env picks the provider (see Bot::setup)
const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
const OPENROUTER_MODEL: &str = "openai/gpt-4o-mini";
const MISTRAL_URL: &str = "https://api.mistral.ai/v1/chat/completions";
const MISTRAL_MODEL: &str = "mistral-medium-latest";
const CHAT_TIMEOUT: Duration = Duration::from_secs(30 * 60); // a chat this quiet closes itself
const CHANCE: f64 = 0.35; // no longer used: superseded by willingness scoring (kept as a fallback die roll)
const WILLINGNESS_THRESHOLD: u8 = 6; // joins the chat once the willingness score clears this
const EVALUATION_INTERVAL: Duration = Duration::from_secs(2 * 60); // how often willingness is re-scored, per channel
const COMMENT_WINDOW: Duration = Duration::from_secs(2 * 60 * 60); // waits this long for comments after posting news
const NEWS_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60); // how often it checks hacker news (agents also run on this tick)
const POKE_INTERVAL: Duration = Duration::from_secs(60 * 60); // how often it tries to speak up unprompted
const POKE_CHANCE: f64 = 0.3; // 30% odds on each attempt
const PRANK_INTERVAL: Duration = Duration::from_secs(3 * 60 * 60); // how often it tries an image/hack prank
const PRANK_CHANCE: f64 = 0.1; // 10% odds per attempt (about once every 30 hours on average)
const HACK_SHARE: f64 = 0.3; // 30% of pranks are the "hacked" bit, the rest are a plain image
const PROBLEM_SHARE: f64 = 0.25; // this share of unprompted turns go to the dev channel as a "damn bug" gripe
const HACK_MESSAGES: u32 = 3; // how many replies the hacked bit runs for (the last one snaps out of it)
const HISTORY_DAYS: i64 = 14; // how many days of messages it reads on first startup
const MEMORY_SIZE: usize = 2000; // how many recent messages it keeps in memory
const CHANNEL_HISTORY: usize = 60; // last N lines kept on disk per channel (bot's own included)
const CHAT_SEED: usize = 10; // lines pulled from channel history when a new chat opens
const CHAT_SIZE: usize = 20; // last N messages of a chat sent to the model
const MESSAGE_LIMIT: usize = 1900; // discord allows 2000; this leaves headroom
const STREAM_EDIT_INTERVAL: Duration = Duration::from_millis(1200); // stream edits never come faster than this (discord's edit rate limit)
const BURST_LIMIT: usize = 4; // max lines (= separate messages) sent per turn; the rest are dropped
const HALF_LINE_THRESHOLD: usize = 12; // while streaming, a trailing half-line shorter than this is held back
                                       // line-to-line delay on the non-streaming path: lines shouldn't all land at
                                       // once, so this staggers them to look like typing. Never measured, picked by
                                       // feel against human typing speed.
const LINE_DELAY_BASE: u64 = 300; // ms
const LINE_DELAY_PER_CHAR: u64 = 15; // ms, per character in the line
const LINE_DELAY_CAP: u64 = 1500; // ms, delay never exceeds this
const THOUGHT_BUTTON: &str = "show_thought"; // custom_id of the thought button appended in hide mode

// chat reply token budget. In release, REPLY_CAP: an ordinary chat reply lands far below
// this; it only exists to cap the cost of a runaway case like a repetition loop.
// 4096 was chosen because on reasoning-capable models the thinking tokens are also drawn
// from this budget — a tighter cap could truncate a long thought plus reply.
// debug uses a smaller Some() so a dev/test run doesn't burn tokens.
const REPLY_CAP: u32 = 4096;
/// Expands to the token budget for a chat reply.
/// Input: none. Output: `Option<u32>` — `Some(2000)` in a debug build, `Some(REPLY_CAP)` in
/// release. Uses: the `REPLY_CAP` constant above. Used directly by `generate_stream`'s caller
/// in `chat_reply.rs` and, via `chat_budget()`, by `chat_cli.rs`.
macro_rules! reply_budget {
    () => {
        if cfg!(debug_assertions) {
            Some(2000u32)
        } else {
            Some(REPLY_CAP)
        }
    };
}
// macro_rules is visible in textual order: because this macro is defined after the file's
// `mod` declarations, submodules can't call it. This is the one wrapper that hands out the
// same budget (chat_cli uses it too), so the cap isn't duplicated in two places.
/// Callable wrapper around the `reply_budget!` macro, for use from files where the macro
/// itself isn't in scope (see the textual-order note above).
/// Input: none. Output: `Option<u32>`, identical to `reply_budget!()`. Uses: `reply_budget!`.
/// Used by: `Bot::chat_cli` (src/chat_cli.rs) as the budget for `generate`.
fn chat_budget() -> Option<u32> {
    reply_budget!()
}
// http: no overall time limit (a long thinking stream shouldn't get cut off); a failed
// connection and a stalled read are capped separately instead
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15); // tcp/tls handshake
const READ_TIMEOUT: Duration = Duration::from_secs(120); // max gap between two chunks; also covers the first token
const AI_RETRIES: u32 = 2; // total extra attempts on network error / 429 / 5xx (this + the original try)

// on a model that won't let reasoning be turned off (mandatory), mini-call budgets are
// raised to this floor; otherwise reasoning eats the whole budget and content comes back
// null (see reasoning_mandatory_error)
const REASONING_MANDATORY_BASE: u32 = 500;
// budget floor for the reasoning-on retry on stream-less agent calls: 2x the current
// budget, or this, whichever is larger
const REASONING_BUDGET_BASE: u32 = 1500;
/// OWNER_ID from .env: always liked and the only user who can switch /model.
/// Unset or invalid means no owner.
fn owner_id() -> Option<u64> {
    static OWNER: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *OWNER.get_or_init(|| match crate::config::runtime_value("OWNER_ID") {
        Ok(v) if !v.trim().is_empty() => match v.trim().parse() {
            Ok(id) => Some(id),
            Err(_) => {
                log::warn!("invalid OWNER_ID '{}', running without an owner", v.trim());
                None
            }
        },
        _ => None,
    })
}

fn is_owner(id: u64) -> bool {
    owner_id() == Some(id)
}
const WANDERER_INTERVAL: Duration = Duration::from_secs(4 * 60 * 60); // how often it browses the news
const IMAGE_DIR: &str = "photos"; // images available for pranks (on-disk folder name)
const STATE_DIR: &str = "durum"; // where the agents write what they've learned (on-disk folder name, kept as-is)
                                 // version string: Cargo.toml plus the commit/date build.rs pulled from git at
                                 // build time. Shown in !status and announced in-channel on restart, so it's
                                 // obvious which build is actually running.
const VERSION: &str = env!("CARGO_PKG_VERSION");
const VERSION_COMMIT: &str = env!("VERSION_COMMIT");
const VERSION_DATE: &str = env!("VERSION_DATE");

/// Formats the running build's version string, e.g. `v0.2.0 (69e2851, 2026-09-02)`.
/// Input: none. Output: `String`. Uses: `VERSION` (from `Cargo.toml` via `CARGO_PKG_VERSION`),
/// `VERSION_COMMIT`/`VERSION_DATE` (from `build.rs`, see that file's `main`).
/// Used by: `modal::status_message`, `modal::settings_embed`, `handler_event.rs`'s
/// `guild_create` (startup version announcement).
fn version_text() -> String {
    format!("v{VERSION} ({VERSION_COMMIT}, {VERSION_DATE})")
}

/// Crate-wide error type: any `std::error::Error` that's `Send + Sync`, boxed so functions
/// don't need to name a concrete error type. Held by nothing itself — it's a return-type
/// alias, used as `Result<T, BotError>` throughout `impl Bot` and the free functions in
/// `src/bot/*.rs`, `src/memory.rs`, `src/agenda.rs`, `src/agents.rs`.
type BotError = Box<dyn std::error::Error + Send + Sync>;

// shutdown signal: cycles check this at the top of each tick and stop opening new rounds;
// the watchdog stops restarting them once it's set
/// Global shutdown flag. Not a function — holds one `bool` (atomic, `false` until shutdown).
/// Set by `main`'s ctrl-c/SIGTERM handler (via `wait_for_shutdown`, `src/bot/setup.rs`);
/// read by `run_cycle` (`src/bot/cycle/cycle_memory.rs`) and each `*_cycle` loop in `src/bot/cycle/cycle_*.rs`.
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

// ---------- state ----------
