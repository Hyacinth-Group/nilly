// ---------- helpers ----------

/// Input: none. Output: `i64` — current time, Unix seconds (0 if the system clock is before
/// the epoch). Used throughout the crate wherever a timestamp is needed (memory writes,
/// growth/sleep/travel tracking, logging).
fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Input: `u: &User` — a serenity Discord user. Output: `String` — `global_name` if set,
/// else the account's `name`. Used by: `Handler::message`/`guild_member_addition`
/// (`handler_event.rs`), `read_history` (`cycle_memory.rs`) — anywhere a person's display
/// name is needed for memory/hafıza keys.
fn display_name(u: &User) -> String {
    u.global_name.clone().unwrap_or_else(|| u.name.clone())
}

/// Appends one line to the in-memory raw message buffer, capped at `MEMORY_SIZE`.
/// Input: `state: &mut State`; `name`/`text: &str`. Output: none (mutates
/// `state.recent_messages`). Used by: `Handler::message` (`handler_event.rs`),
/// `chat_cli.rs`.
fn remember(state: &mut State, name: &str, text: &str) {
    state.recent_messages.push_back(format!("{name}: {text}"));
    if state.recent_messages.len() > MEMORY_SIZE {
        state.recent_messages.pop_front();
    }
}

// splits long text into chunks of at most `limit` characters: prefers a sentence boundary,
// then a space, and falls back to a hard cut at the exact limit if neither is found;
// nothing is ever dropped. Walks the slice: no intermediate allocation per turn, only the
// resulting chunk becomes a String.
/// Input: `text: &str`; `limit: usize` — max characters per chunk. Output: `Vec<String>` —
/// the chunks in order, nothing dropped. Uses: `cut_point`. Used by: `code_blocks`/
/// `stream_layout` (`provider_stream_view.rs`), `parse_reply` (`text_3.rs`).
fn split(text: &str, limit: usize) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut remaining = text.trim();
    while remaining.char_indices().nth(limit).is_some() {
        let cut = cut_point(remaining, limit); // byte offset
        let head = remaining[..cut].trim();
        if !head.is_empty() {
            parts.push(head.to_string());
        }
        remaining = remaining[cut..].trim_start();
    }
    if !remaining.is_empty() {
        parts.push(remaining.to_string());
    }
    parts
}

// byte offset of the best cut point within the first `limit` characters; to avoid a tiny
// leftover chunk, a sentence/space cut is only used if it falls after the first quarter of
// the limit, otherwise it falls back to a hard cut
/// Input: `text: &str`; `limit: usize`. Output: `usize` — a byte offset into `text`. Used
/// by: `split` above, the only caller.
fn cut_point(text: &str, limit: usize) -> usize {
    let mut sentence = (0usize, 0usize); // (character index, byte offset)
    let mut space = (0usize, 0usize);
    let mut end = text.len();
    for (i, (off, c)) in text.char_indices().enumerate() {
        if i >= limit {
            end = off;
            break;
        }
        if matches!(c, '.' | '!' | '?' | '\n') {
            sentence = (i + 1, off + c.len_utf8());
        } else if c == ' ' {
            space = (i, off);
        }
    }
    let minimum = limit / 4;
    if sentence.0 > minimum {
        sentence.1
    } else if space.0 > minimum {
        space.1
    } else {
        end
    }
}

// a discord spoiler; pipe characters inside are escaped so they can't break out of it
/// Input: `text: &str`. Output: `String` — `text` wrapped in `||...||`, `|` escaped. Used
/// by: `stream_layout` (`provider_stream_view.rs`).
fn spoiler(text: &str) -> String {
    format!("||{}||", text.replace('|', "\\|"))
}

// appends a line to a channel's history and writes it to disk; survives a closed chat and a restart
/// Input: `state: &mut State`; `channel: &RoomKey`; `line: String`. Output: none.
/// Uses: `channel_notes` (single-element wrapper). Used by: `Handler::message`
/// (`handler_event.rs`), `Bot::send_reply` (`provider_send_line.rs`).
fn channel_note(state: &mut State, channel: &RoomKey, line: String) {
    channel_notes(state, channel, [line]);
}

// appends multiple lines with a SINGLE file write. Since a reply now goes out line by
// line, writing on every single line used to rewrite the whole channel history 4-5 times
// per turn.
/// Input: `state: &mut State`; `channel: &RoomKey`; `lines: impl IntoIterator<Item =
/// String>`. Output: none (updates `state.channel_history`, capped at `CHANNEL_HISTORY`,
/// and writes the whole history to `durum/kanallar/<id>.md`). Uses: `memory::write`. Used
/// by: `channel_note` above, `Bot::send_stream` (`provider_send_stream.rs`).
fn channel_notes(state: &mut State, channel: &RoomKey, lines: impl IntoIterator<Item = String>) {
    let mut lines = lines.into_iter().peekable();
    if lines.peek().is_none() {
        return;
    }
    let hist = state.channel_history.entry(channel.clone()).or_default();
    hist.extend(lines);
    while hist.len() > CHANNEL_HISTORY {
        hist.pop_front();
    }
    // no intermediate Vec: joined directly into a single String
    let mut content = String::new();
    for (i, line) in hist.iter().enumerate() {
        if i > 0 {
            content.push('\n');
        }
        content.push_str(line);
    }
    // Step 2 migrates persisted room paths atomically; terminal rooms stay memory-only.
    if let Ok(path) = memory::v1_room_path(channel) {
        memory::write(&path, &content);
    }
}

/// Input: `state: &State`; `n: usize`. Output: `String` — the last `n` lines of
/// `state.recent_messages`, joined with `\n`. Used by: `Bot::willingness`/`pick_target`
/// (`provider_generate.rs`), `Bot::post_problem`/`news_cycle` (`cycle_news.rs`),
/// `poke_cycle` (`cycle_background.rs`), `Bot::coach` (`agents.rs`), `idle_channel`
/// (`cycle_background.rs`).
fn recent_messages(state: &State, n: usize) -> String {
    let skip = state.recent_messages.len().saturating_sub(n);
    let mut s = String::new();
    for (i, line) in state.recent_messages.iter().skip(skip).enumerate() {
        if i > 0 {
            s.push('\n');
        }
        s.push_str(line);
    }
    s
}

// turns a chat into the plain transcript the critic/diarist/coach read. A bot reply can
// span several lines (each line went out as its own message, and a reaction line can be
// mixed in): the prefix goes on EVERY line, otherwise the second and later lines would
// read as if they belonged to someone else in the group
/// Input: `history: &[ChatMessage]`; `bot_name: &str`. Output: `String` — one `"name: text"`
/// line per line of every message, in order. Used by: `Bot::close_timed_out`
/// (`cycle_growth.rs`) when handing a finished chat to `diarist`/`critic`.
fn transcript(history: &[ChatMessage], bot_name: &str) -> String {
    let mut s = String::new();
    let mut first = true;
    for m in history {
        for line in m.content.split('\n') {
            if !first {
                s.push('\n');
            }
            first = false;
            if m.role == "assistant" {
                s.push_str(bot_name);
                s.push_str(": ");
            }
            s.push_str(line);
        }
    }
    s
}

// lowercases for comparison; also drops the combining dot that the İ→i̇ conversion adds,
// so names like "ŞAHİN"/"şahin" match
/// Input: `s: &str`. Output: `String` — `s.to_lowercase()` with the stray Turkish
/// combining-dot character (`İ`→`i̇`) stripped, for name-comparison purposes. Used by:
/// `strip_name` below, `Handler::message` (`handler_event.rs`).
fn casefold(s: &str) -> String {
    s.to_lowercase().replace('\u{0307}', "")
}

// strips a name prefix and quotes the model may have prepended; returns a slice.
// On the hot path (every edit during a stream), the whole text isn't cloned and
// lowercased: the prefix comparison only touches the leading characters, folding case the
// same way casefold does
/// Input: `text: &'a str`; `bot_name: &str`. Output: `&'a str` — a slice of `text` with a
/// leading `"bot_name:"` prefix and surrounding quotes removed, if present. Uses: `casefold`.
/// Used by: `clean` below, `stream_view` (`provider_stream_view.rs`), `send_lines`
/// (`provider_send_line.rs`), `run_prank` (`cycle_news.rs`), `Bot::reply`'s fallback
/// (`chat_reply.rs`), `chat_cli.rs`.
fn strip_name<'a>(text: &'a str, bot_name: &str) -> &'a str {
    let mut rest = text.trim();
    // the model can mimic the "name: text" pattern and put its own name at the front
    let prefix = format!("{bot_name}:");
    let char_count = prefix.chars().count();
    let head: String = rest
        .chars()
        .take(char_count)
        .flat_map(|c| c.to_lowercase())
        .filter(|&c| c != '\u{0307}')
        .collect();
    if head.starts_with(&casefold(&prefix)) {
        rest = match rest.char_indices().nth(char_count) {
            Some((off, _)) => rest[off..].trim(),
            None => "",
        };
    }
    if rest.chars().count() > 1 && rest.starts_with('"') && rest.ends_with('"') {
        rest = &rest[1..rest.len() - 1];
    }
    rest
}

// non-streaming paths are limited to a single message: strip_name + a 1900 cap.
// the streaming path splits with split() after strip_name and sends every piece, no truncation.
/// Input: `text: String`; `bot_name: &str`. Output: `String` — `strip_name(text)`, hard-cut
/// at `MESSAGE_LIMIT` characters. Uses: `strip_name`. Used by: `Bot::generate`
/// (`provider_generate.rs`), the only caller (every non-streaming personality reply).
fn clean(text: String, bot_name: &str) -> String {
    let m = strip_name(&text, bot_name);
    // find the byte offset at the limit and cut there in place: no intermediate collect
    match m.char_indices().nth(MESSAGE_LIMIT) {
        Some((off, _)) => m[..off].to_string(),
        None => m.to_string(),
    }
}

// ---------- output protocol ----------

// The model's reply is a line-based protocol: each line goes out as its own message, a
// "tepki: 💀" line becomes an emoji reaction instead of text, and a lone "-" is the
// silence marker. Everything here operates on text that's already been through
// strip_name(); it is not stripped again.
