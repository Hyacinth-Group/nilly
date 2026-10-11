/// Input: `api_url: &str`. Output: `bool` — whether `api_url` points at OpenRouter (the only
/// target `cache_control` is added for, see the file-end comment). Used by: `system_json`
/// below.
fn supports_cache(api_url: &str) -> bool {
    api_url.contains("openrouter.ai")
}

// turns the system message into an OpenAI-compatible block: plain text if variable is
// empty, otherwise a fixed+variable two-block text array; the fixed block is only marked
// with cache_control when the request is going to openrouter
/// Input: `fixed`/`variable: &str` — the two system-message parts (see `system_text`);
/// `api_url: &str` — decides whether to mark `fixed` with `cache_control`. Output:
/// `serde_json::Value` — the `{"role":"system", "content": ...}` request block. Uses:
/// `supports_cache`. Used by: `Bot::ask_split`/`ask_raw_stream` (`provider_ask.rs`).
fn system_json(fixed: &str, variable: &str, api_url: &str) -> serde_json::Value {
    if variable.is_empty() {
        return serde_json::json!({ "role": "system", "content": fixed });
    }
    let mut first = serde_json::json!({ "type": "text", "text": fixed });
    if supports_cache(api_url) {
        first["cache_control"] = serde_json::json!({ "type": "ephemeral" });
    }
    serde_json::json!({ "role": "system", "content": [
        first,
        { "type": "text", "text": variable }
    ]})
}

// every reply's system message has two parts: FIXED (personality, temperament, profile,
// index... changes when an agent runs, this is where the prompt cache pays off) and
// VARIABLE (retrieved memory, time, task)
/// Assembles the fixed/variable system-message text (see `docs/architecture.md`'s numbered list).
/// Input: `state: &State` (reads `bot_name`, `favorite_name`, `growth`, `temperament`,
/// `profile`, `index`, `agenda`, `myself`, `corrections`); `instruction: &str` — this call's
/// task; `retrieved: &str` — memory retrieved for this chat (empty for agent calls).
/// Output: `(fixed, variable): (String, String)`. Uses: `FAVORITE_LINE`/`PERSONALITY`
/// (`prompts.rs`), `growth::stage_text`, `sleep::status_text`, `travel::status_text`. Used
/// by: `Bot::chat_system` (`provider_generate.rs`), `Bot::image_commenter` (`agents.rs`).
fn system_text(state: &State, instruction: &str, retrieved: &str) -> (String, String) {
    let p = prompts::current();
    let favorite_line = match &state.favorite_name {
        Some(f) => p.favorite_line.replace("{favori}", f),
        None => String::new(),
    };
    let section = |s: &mut String, title: &str, content: &str| {
        if !content.trim().is_empty() {
            if !s.is_empty() {
                s.push_str("\n\n");
            }
            s.push_str(title);
            s.push('\n');
            s.push_str(content.trim());
        }
    };

    let mut fixed = p
        .personality
        .replace("{ad}", &state.bot_name)
        .replace("{favori_satiri}", &favorite_line);
    section(
        &mut fixed,
        "GELİŞİM EVREN",
        &growth::stage_text(&state.growth),
    );
    section(
        &mut fixed,
        "HUYUN (hocanın son notu, buna göre davran)",
        &state.temperament,
    );
    section(&mut fixed, "BU GRUP HAKKINDA BİLDİKLERİN", &state.profile);
    section(
        &mut fixed,
        "HAFIZA DİZİNİ (kimi ve neyi biliyorsun; ayrıntı gerekince getiriliyor)",
        &state.index,
    );
    section(
        &mut fixed,
        "GÜNDEM (internette gezerken okudukların ve düşündüklerin)",
        &state.agenda,
    );
    section(&mut fixed, "SENİN SON HALİN", &state.myself);
    section(
        &mut fixed,
        "KENDİNE NOTLAR (eleştirmenin son sohbetten çıkardığı dersler)",
        &state.corrections,
    );

    let mut variable = String::new();
    section(
        &mut variable,
        "BU SOHBET İÇİN HAFIZADAN GETİRİLENLER",
        retrieved,
    );
    section(
        &mut variable,
        "ŞU AN",
        &format!("{} {}", sleep::status_text(state), travel::status_text()),
    );
    section(&mut variable, "ŞU ANKİ GÖREVİN", instruction);
    (fixed, variable)
}

// ---------- chat mechanics ----------

/// Opens (or returns the existing) `Chat` for a channel, seeding fresh history from the
/// channel's recent lines and, if given, an already-sent opening message.
/// Input: `state: &mut State`; `channel: &RoomKey`; `opening: Option<String>` — an opening
/// reply already sent line-by-line, to fold into history as one assistant turn without
/// duplication (see comment below). Output: `&mut Chat` — the (new or existing) chat.
/// Uses: `state.channel_history`, `assistant`/`user`. Used by: `Handler::message`
/// (`handler_event.rs`), `Bot::post_problem`/`send_news`/`post_news`/`run_prank`
/// (`cycle_news.rs`), `poke_cycle` (`cycle_background.rs`), `guild_member_addition`,
/// `sleep_transition`/`evaluate_waking` (`cycle_sleep.rs`), `pick_name` (`cycle_growth.rs`),
/// `chat_cli.rs`.
fn start_chat<'a>(state: &'a mut State, channel: &RoomKey, opening: Option<String>) -> &'a mut Chat {
    let mut chat = Chat::default();
    // start from the channel's recent lines, so it knows what's already been discussed
    let prefix = format!("{}: ", state.bot_name);
    if let Some(hist) = state.channel_history.get(channel) {
        let skip = hist.len().saturating_sub(CHAT_SEED);
        for line in hist.iter().skip(skip) {
            match line.strip_prefix(&prefix) {
                Some(m) => chat.history.push(assistant(m)),
                None => {
                    if let Some((name, _)) = line.split_once(": ") {
                        let label = name.to_lowercase();
                        if let Some(id) = state.name_to_id.get(&label) {
                            chat.participants.entry(label).or_insert_with(|| id.clone());
                        }
                    }
                    chat.history.push(user(line.clone()));
                }
            }
        }
    }
    if let Some(a) = opening {
        // the opening was already sent and already landed in the channel history LINE BY
        // LINE (each line its own message); those lines are stripped from the bot block at
        // the end of the seed, otherwise the model would see its own opening twice — once
        // split into lines and once combined. Another bot message (like a news link) may
        // have landed in between, so the whole block is scanned.
        let pieces: Vec<&str> = a.split('\n').map(str::trim).collect();
        let mut i = chat.history.len();
        while i > 0 && chat.history[i - 1].role == "assistant" {
            i -= 1;
            if pieces.contains(&chat.history[i].content.trim()) {
                chat.history.remove(i);
            }
        }
        chat.history.push(assistant(a));
        chat.counter = 1;
    }
    state.last_activity.insert(channel.clone(), Instant::now());
    state.chats.entry(channel.clone()).or_insert(chat)
}

/// Input: `state: &mut State`; `channel: &RoomKey`. Output: `Option<Chat>` — the removed
/// chat, if one existed (also clears `state.awaiting_comment` for the channel). Used by:
/// `Bot::close_timed_out` (`cycle_growth.rs`), `Handler::message`'s expired-news-window path
/// (`handler_event.rs`).
fn end_chat(state: &mut State, channel: &RoomKey) -> Option<Chat> {
    state.awaiting_comment.remove(channel);
    state.chats.remove(channel)
}

/// Input: `state: &State`; `channel: &RoomKey`; `generated_incoming: u32` — the
/// `Chat.incoming` count captured when a reply started generating. Output: `bool` — whether
/// a newer user message has arrived since then. Used by: `Bot::reply` (`chat_reply.rs`), to
/// decide whether to loop for another turn instead of exiting.
fn new_message_arrived(state: &State, channel: &RoomKey, generated_incoming: u32) -> bool {
    state
        .chats
        .get(channel)
        .is_some_and(|s| s.incoming > generated_incoming)
}
