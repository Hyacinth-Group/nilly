// ---------- helpers ----------

/// Unix seconds, defaulting to zero if the system clock precedes the epoch.
fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}


/// Appends one line to the in-memory raw message buffer, capped at `MEMORY_SIZE`.
fn remember(state: &mut State, name: &str, text: &str) {
    state.recent_messages.push_back(format!("{name}: {text}"));
    if state.recent_messages.len() > MEMORY_SIZE {
        state.recent_messages.pop_front();
    }
}

/// Stable observed speaker handles never reuse a label for another identity.
fn observe_user(state: &mut State, person: &UserRef) -> String {
    let plain = person.display_name.to_lowercase();
    let mut label = if state.recent_users.get(&plain).is_some_and(|known| known.id != person.id) {
        format!("{} [{}]", person.display_name, person.id)
    } else {
        person.display_name.clone()
    };
    while state.recent_users.get(&label.to_lowercase()).is_some_and(|known| known.id != person.id) {
        label.push_str(&format!(" [{}]", person.id));
    }
    state.recent_users.insert(label.to_lowercase(), person.clone());
    if let Some(username) = &person.username {
        state.usernames.insert(person.id.clone(), username.clone());
    }
    label
}

/// Discord-compatible image transcript markers contain no media URL or native token.
fn incoming_text(text: &str, has_image: bool, image_analysis: bool) -> String {
    if has_image && image_analysis {
        if text.trim().is_empty() { "[resim attı]".to_string() }
        else { format!("[resim] {text}") }
    } else {
        text.to_string()
    }
}

// splits long text into chunks of at most `limit` characters: prefers a sentence boundary,
// then a space, and falls back to a hard cut at the exact limit if neither is found;
// nothing is ever dropped. Walks the slice: no intermediate allocation per turn, only the
// resulting chunk becomes a String.
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
fn spoiler(text: &str) -> String {
    format!("||{}||", text.replace('|', "\\|"))
}

// appends a line to a channel's history and writes it to disk; survives a closed chat and a restart
fn channel_note(state: &mut State, channel: &ConversationKey, line: String) {
    channel_notes(state, channel, [line]);
}

// appends multiple lines with a SINGLE file write. Since a reply now goes out line by
// line, writing on every single line used to rewrite the whole channel history 4-5 times
// per turn.
/// Caps history at `CHANNEL_HISTORY` and persists it under `kanallar/<id>.md`.
fn channel_notes(state: &mut State, channel: &ConversationKey, lines: impl IntoIterator<Item = String>) {
    channel_notes_observed(state, channel, lines.into_iter().map(|line| (line, None)));
}

fn channel_notes_observed(state: &mut State, channel: &ConversationKey, lines: impl IntoIterator<Item = (String, Option<UserRef>)>) {
    let mut lines = lines.into_iter().peekable();
    if lines.peek().is_none() {
        return;
    }
    let hist = state.channel_history.entry(channel.clone()).or_default();
    let observations = state.channel_observations.entry(channel.clone()).or_default();
    observations.resize(hist.len(), None);
    for (line, person) in lines {
        hist.push_back(line);
        observations.push_back(person);
    }
    while hist.len() > CHANNEL_HISTORY {
        hist.pop_front();
        observations.pop_front();
    }
    // no intermediate Vec: joined directly into a single String
    let mut content = String::new();
    for (i, line) in hist.iter().enumerate() {
        if i > 0 {
            content.push('\n');
        }
        content.push_str(line);
    }
    memory::write_channel_history(channel, &content, observations);
}

fn channel_observe(state: &mut State, channel: &ConversationKey, line: String, person: &UserRef) {
    channel_notes_observed(state, channel, [(line, Some(person.clone()))]);
}

fn channel_observed_users(state: &State, channel: &ConversationKey, limit: usize) -> HashMap<String, UserRef> {
    let Some(lines) = state.channel_history.get(channel) else { return HashMap::new(); };
    let Some(people) = state.channel_observations.get(channel) else { return HashMap::new(); };
    lines.iter().zip(people).skip(lines.len().saturating_sub(limit)).filter_map(|(line, person)| {
        let person = person.as_ref()?;
        let (label, _) = line.split_once(": ")?;
        Some((label.to_lowercase(), person.clone()))
    }).collect()
}

/// Returns the last `n` raw message lines in chronological order.
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
/// Removes the combining dot produced when lowercasing Turkish İ.
fn casefold(s: &str) -> String {
    s.to_lowercase().replace('\u{0307}', "")
}

// strips a name prefix and quotes the model may have prepended; returns a slice.
// On the hot path (every edit during a stream), the whole text isn't cloned and
// lowercased: the prefix comparison only touches the leading characters, folding case the
// same way casefold does
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
