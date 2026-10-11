/// One open conversation in one room. Holds:
/// - `history`: the `ChatMessage` turns sent to/from the model (capped at `CHAT_SIZE`).
/// - `counter`: how many replies the bot has sent in this chat.
/// - `hacked`: nonzero while the "hacked" bit is running; ticks down by one per reply.
/// - `last_message`/`last_was_tagged`: which message to reply to, and whether that came from
///   a mention/reply/name (drives whether a Discord reply-to is used).
/// - `incoming`: count of user messages received (detects a new one mid-reply).
/// - `recent_arrivals`: (name, message id) pairs heard since the last reply, for target selection.
/// - `mood`: this chat's current mood string, e.g. `"confusion (6)"`.
///
/// Created by `start_chat`, stored in `State.chats`, read/written throughout `chat_reply.rs`,
/// `chat_lookup.rs`, `provider_generate.rs`, `provider_send_stream.rs`, `handler_event.rs`.
#[derive(Default)]
struct Chat {
    history: Vec<ChatMessage>,
    participants: HashMap<String, UserKey>, // observed speaker labels alongside model turns
    counter: u32,
    hacked: u32, // nonzero while the "hacked" bit is running; ticks down by one per reply
    last_message: Option<MessageKey>, // the message to reply to
    last_was_tagged: bool, // whether last_message came in via a mention/reply/name (drives whether we reply-to)
    incoming: u32, // count of user messages received; used to notice a new one arrived mid-reply
    // people (name, message id) heard since the bot's last reply; feeds target selection,
    // cleared once the bot replies
    recent_arrivals: VecDeque<(String, MessageKey)>,
    mood: String, // e.g. "confusion (6)"; empty means not yet determined or intensity too low
}

// thinking-display mode; changed with !thinking, persisted in durum/dusunme.md
/// How much of the model's reasoning is requested and shown. Holds no data beyond the
/// variant itself: `Show` (spoiler + code block alongside the reply), `Hide` (live word
/// counter while thinking, a button reveals it after), `Silent` (thinks in the background,
/// no trace at all), `Off` (reasoning not requested). Stored as `State.thinking_mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ThinkingMode {
    #[default]
    Show, // reasoning is requested and shown alongside the reply, inside a spoiler
    Hide, // reasoning is requested but not shown; prints "Thinking..." while it runs
    Silent, // reasoning is requested (thinks in the background) but leaves no trace at all: no placeholder/counter/button, the reply just arrives
    Off, // reasoning is not requested; requests go out with thinking disabled
}

impl ThinkingMode {
    /// The Turkish token this mode is persisted as in `durum/dusunme.md`.
    /// Input: `self`. Output: `&'static str` (`"goster"`/`"gizle"`/`"sessiz"`/`"kapali"`).
    /// Inverse of `read`. Used by: `handler_buttons.rs`'s `setting_button`,
    /// `command/settings.rs`'s `cmd_thinking` (both write this back to disk after a change).
    fn file_value(self) -> &'static str {
        match self {
            ThinkingMode::Show => "goster",
            ThinkingMode::Hide => "gizle",
            ThinkingMode::Silent => "sessiz",
            ThinkingMode::Off => "kapali",
        }
    }

    /// Loads the persisted thinking mode from disk at startup.
    /// Input: none. Output: `Self` — parsed from `durum/dusunme.md`, defaulting to `Show` if
    /// the file is empty or unrecognized. Uses: `memory::read`. Used by: `State::load` below.
    fn read() -> Self {
        match memory::read("dusunme.md").trim() {
            "gizle" => ThinkingMode::Hide,
            "sessiz" => ThinkingMode::Silent,
            "kapali" => ThinkingMode::Off,
            _ => ThinkingMode::Show,
        }
    }

    // mode from a command argument; None if not recognized
    /// Parses a `/dusunme` command argument or a settings-panel button id suffix into a mode.
    /// Input: `arg: &str` — one of the Turkish tokens Discord users type/click (`"göster"`,
    /// `"gizle"`, `"sessiz"`, `"kapat"`, plus a few spelling/ASCII variants and `"on"`/`"off"`).
    /// Output: `Option<Self>` — `None` if `arg` isn't recognized. Used by:
    /// `command/settings.rs`'s `cmd_thinking`, `handler_buttons.rs`'s `setting_button`.
    fn from_arg(arg: &str) -> Option<Self> {
        match arg {
            "göster" | "goster" | "aç" | "ac" | "on" => Some(ThinkingMode::Show),
            "gizle" => Some(ThinkingMode::Hide),
            "sessiz" => Some(ThinkingMode::Silent),
            "kapat" | "kapalı" | "kapali" | "off" => Some(ThinkingMode::Off),
            _ => None,
        }
    }

    /// The Turkish display label shown to users (in `/durum`, `/ayarlar`, `/zihin`).
    /// Input: `self`. Output: `&'static str` (`"göster"`/`"gizli"`/`"sessiz"`/`"kapalı"`).
    /// Used by: `modal.rs`'s `mind_embeds`/`summary_modal`/`status_message`/`settings_embed`.
    fn label(self) -> &'static str {
        match self {
            ThinkingMode::Show => "göster",
            ThinkingMode::Hide => "gizli",
            ThinkingMode::Silent => "sessiz",
            ThinkingMode::Off => "kapalı",
        }
    }
}

/// The single shared state, held behind `Bot.state: Mutex<State>`. Holds:
/// - identity: `bot_name`, `favorite_name`, `accounts`, `model`, `thinking_mode`, `debug`.
/// - agent output (mirrored on disk under `durum/`): `profile` (profiler), `temperament`
///   (coach), `corrections` (critic), `myself` (diarist), `index` (memory index), `agenda`
///   (wanderer), `growth` (stage/counters/chosen name, see `growth::Growth`).
/// - what it has observed: `recent_messages`, `own_messages`, `last_channel`,
///   `name_to_id`/`usernames` (identity mappings), `channel_history` (per-channel, also on disk).
/// - chat tracking: `chats`, `busy`, `last_activity`, `last_evaluation`, `awaiting_comment`,
///   `posted_news`, `scanned`.
/// - sleep: `sleep_start`/`sleep_start_memory_len`, `last_night_observation`, `stashed_news`,
///   `plans` (`sleep::Plan`), `asleep`, `forced_awake_until`, `pending_mentions`.
/// - memory pipeline: `memory_queue` (fed by `close_timed_out`/`news_cycle`, drained by
///   `memory_cycle` into `diarist`/`critic`).
/// - thinking-button state: `thought_store`, `thought_order`.
/// - `metrics`: running token-usage totals (`Metrics`, see `provider_types.rs`).
/// - travel: `last_road_message`, `announced_trip`.
///
/// Built by `State::load` at startup; every other field mutation happens through a
/// `Bot::state()` lock elsewhere in the crate.
#[derive(Default)]
struct State {
    bot_name: String,
    favorite_name: Option<String>,
    // what the agents produced (also mirrored on disk under durum/)
    profile: String,     // profiler
    temperament: String, // coach
    corrections: String, // critic
    myself: String,      // diarist: the bot's own current state
    index: String,       // memory index, sent with every reply
    // what it has observed
    recent_messages: VecDeque<String>, // recent server messages, "name: text"
    own_messages: VecDeque<String>,    // the bot's own recent messages
    last_channel: Option<RoomKey>,
    // chat tracking
    chats: HashMap<RoomKey, Chat>,
    busy: HashSet<RoomKey>, // rooms currently generating a reply
    last_activity: HashMap<RoomKey, Instant>, // when a chat last had activity
    last_evaluation: HashMap<RoomKey, Instant>, // last willingness call per room
    awaiting_comment: HashMap<RoomKey, Instant>,
    posted_news: HashSet<u64>,
    scanned: HashSet<SpaceKey>,
    // sleep: start time + raw memory length at sleep onset, so waking up can evaluate
    // what was written overnight
    sleep_start: i64,
    sleep_start_memory_len: usize,
    last_night_observation: i64, // last time it processed the night's messages (checks every 2h)
    stashed_news: Option<agents::News>, // news picked while asleep, posted on waking
    // agenda and sleep
    agenda: String, // wanderer: what it last read and thought
    // identity mappings: display name (lowercase) -> id, id -> username
    name_to_id: HashMap<String, UserKey>,
    usernames: HashMap<UserKey, String>,
    // queue the memory cycle works through: (transcript, source, channel name, also run the critic?)
    memory_queue: VecDeque<(String, String, String, bool)>,
    plans: Vec<sleep::Plan>,
    asleep: bool,
    forced_awake_until: i64, // after !wake, sleep plans are ignored until this unix time
    channel_history: HashMap<RoomKey, VecDeque<String>>, // recent lines per room, also kept on disk
    pending_mentions: Vec<(RoomKey, String)>, // mentions received while asleep
    growth: growth::Growth,
    accounts: HashMap<ConnectionId, UserRef>, // identity belongs to a connection, not the brain
    model: String,     // model in use; changed with !model, persisted in durum/model.md
    thinking_mode: ThinkingMode, // thinking mode; changed with !thinking, persisted in durum/dusunme.md
    debug: bool, // !debug: decisions (willingness, target, silence/reaction) get posted to the channel; durum/debug.md
    // thought behind recent replies, for the button in hide mode to show (message id -> thought)
    thought_store: HashMap<MessageKey, String>,
    thought_order: VecDeque<MessageKey>,
    metrics: Metrics,        // running total of model usage for the session
    last_road_message: i64,  // last day it posted an on-the-road message while traveling
    announced_trip: i64,     // start day of the trip it already announced as "leaving tomorrow"
}

impl State {
    // reads from disk on restart, so it doesn't start over from nothing
    /// Builds the initial `State` at startup by reading everything persisted under `durum/`.
    /// Input: none (reads files via `memory::read` and friends). Output: `Self`, with the
    /// fields listed in the struct doc above populated and everything else left at its
    /// `Default`. Uses: `memory::read`, `memory::refresh_index`, `agenda::latest_agenda`,
    /// `growth::load`, `memory::load_channel_history`, `ThinkingMode::read`.
    /// Used by: `Bot::setup` (`setup.rs`), which wraps the result in `Mutex::new`.
    fn load() -> Self {
        State {
            profile: memory::read("profil.md"),
            temperament: memory::read("huy.md"),
            corrections: memory::read("duzeltmeler.md"),
            myself: memory::read("kendim.md"),
            index: memory::refresh_index(),
            agenda: agenda::latest_agenda(&memory::read("gundem.md")),
            growth: growth::load(),
            channel_history: memory::load_channel_history().into_iter().collect(),
            thinking_mode: ThinkingMode::read(),
            debug: memory::read("debug.md").trim() == "acik",
            // guilds already scanned before: persisted so a restart doesn't re-fetch 14
            // days of history each time (guild_create fires again on every ready)
            scanned: memory::read("taranan.md")
                .lines()
                .filter_map(|l| l.trim().parse::<u64>().ok())
                .map(SpaceKey::discord)
                .collect(),
            ..State::default()
        }
    }

    // links a thought to its reply's message id, so the button in hide mode can find it;
    // the store is capped, oldest entries drop first
    /// Records a reply's thought so the "Show Thought Process" button can retrieve it later.
    /// Input: `&mut self`; `message` — the domain message key the thought belongs to;
    /// `thought` — the collapsed thought text. Output: none (mutates `self.thought_store`/
    /// `self.thought_order`, evicting the oldest entry once there are more than 50).
    /// Used by: `Bot::send_stream` (`provider_send_stream.rs`), when in `ThinkingMode::Hide`.
    fn link_thought(&mut self, message: MessageKey, thought: String) {
        self.thought_store.insert(message.clone(), thought);
        self.thought_order.push_back(message);
        while self.thought_order.len() > 50 {
            if let Some(oldest) = self.thought_order.pop_front() {
                self.thought_store.remove(&oldest);
            }
        }
    }
}

// releases a channel's busy flag: runs on the normal path, an early return, and on panic
// via Drop — the flag can't be forgotten and leave a channel permanently locked
