fn supports_cache(api_url: &str) -> bool {
    api_url.contains("openrouter.ai")
}

// turns the system message into an OpenAI-compatible block: plain text if variable is
// empty, otherwise a fixed+variable two-block text array; the fixed block is only marked
// with cache_control when the request is going to openrouter
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

/// Removes the chat and clears its awaiting-comment flag.
fn end_chat(state: &mut State, channel: &RoomKey) -> Option<Chat> {
    state.awaiting_comment.remove(channel);
    state.chats.remove(channel)
}

/// Detects a user message arriving after reply generation captured `Chat.incoming`.
fn new_message_arrived(state: &State, channel: &RoomKey, generated_incoming: u32) -> bool {
    state
        .chats
        .get(channel)
        .is_some_and(|s| s.incoming > generated_incoming)
}
