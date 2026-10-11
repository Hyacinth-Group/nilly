/// Parses raw model output into a `Reply` per the line-based protocol (see AGENTS.md rule 2
/// and `docs/flows.md`'s "Output protocol").
/// Expects the bot-name prefix to have been stripped already.
fn parse_reply(text: &str) -> Reply {
    let mut reply = Reply::default();
    let mut lines: Vec<String> = Vec::new();
    let mut skipped = 0usize;
    // a number prefix is only stripped when it's a REAL list: two or more numbered lines
    // means the model wrote a list. A single line's "3. sınıftayım" ("I'm in 3rd grade")
    // is an ordinal and is left alone.
    let is_list = text
        .split('\n')
        .filter(|s| number_prefix(s.trim()).is_some())
        .count()
        >= 2;
    for raw in text.split('\n') {
        let mut line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if silence_marker(line) {
            reply.silent = true;
            continue;
        }
        if let Some(body) = reaction_body(line) {
            // the first reaction wins; if the emoji can't be parsed, the line still
            // doesn't go out as a message
            if reply.reaction.is_none() {
                reply.reaction = extract_emoji(body);
            }
            continue;
        }
        // don't let a leftover scrap of the previous message ("'cım" etc.) through
        if line.starts_with('\'') {
            continue;
        }
        if is_list {
            if let Some(stripped) = number_prefix(line) {
                line = stripped;
            }
        }
        let cleaned = clean_slop(line);
        if cleaned.is_empty() {
            continue;
        }
        // the exact same line shouldn't go out twice in one turn ("he\nhe")
        if lines.contains(&cleaned) {
            continue;
        }
        if lines.len() >= BURST_LIMIT {
            skipped += 1;
            continue;
        }
        lines.push(cleaned);
    }
    if skipped > 0 {
        log::debug!("reply: burst limit exceeded, dropped {skipped} line(s)");
    }
    // a line over 1900 chars doesn't fit one message: split and flatten
    reply.lines = lines.iter().flat_map(|s| split(s, MESSAGE_LIMIT)).collect();
    reply
}

// if two of the last 4 bot lines (reaction lines don't count) ended in a question mark,
// it's been asking questions back-to-back; `reply` turns this into an instruction
fn too_many_questions(state: &State, channel: ChannelId) -> bool {
    let prefix = format!("{}: ", state.bot_name);
    state
        .channel_history
        .get(&channel)
        .map(|hist| {
            hist.iter()
                .rev()
                .filter_map(|l| l.strip_prefix(&prefix))
                .filter(|l| reaction_body(l).is_none())
                .take(4)
                .filter(|l| l.trim_end().ends_with('?'))
                .count()
        })
        .unwrap_or(0)
        >= 2
}

// status codes treated as transient: worth backing off and retrying (429 rate limit, 5xx
// server-side); a permanent error like 401/404 is not retried
fn status_retryable(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504)
}

// collapses an error body down to a single line
/// Caps the collapsed error at 300 characters.
fn trim_error(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(300)
        .collect()
}

// did the provider return a 400 saying "reasoning can't be disabled" (as some GLM reasoning variants do)?
fn reasoning_mandatory_error(body_text: &str) -> bool {
    let lower = body_text.to_lowercase();
    lower.contains("reasoning") && (lower.contains("mandatory") || lower.contains("cannot be disabled"))
}

// pulls the JSON out from inside decoration like ```json ... ```
/// Takes the first opening through last closing brace; leaves text unchanged without a pair.
fn extract_json(text: &str) -> &str {
    match (text.find('{'), text.rfind('}')) {
        (Some(open), Some(close)) if close > open => &text[open..=close],
        _ => text,
    }
}

// parses the 0-10 score and reason out of a willingness reply; None if malformed (the reason is for the debug line)
/// `puan`/`sebep` are protocol keys matching the willingness prompt's JSON.
fn parse_willingness(reply: &str) -> Option<(u8, String)> {
    #[derive(Deserialize)]
    struct Score {
        #[serde(default)]
        puan: i32,
        #[serde(default)]
        sebep: String,
    }
    let parsed: Score = serde_json::from_str(extract_json(reply)).ok()?;
    Some((parsed.puan.clamp(0, 10) as u8, parsed.sebep.trim().to_string()))
}

/// Test-only score projection of `parse_willingness`.
#[cfg(test)]
fn willingness_score(reply: &str) -> Option<u8> {
    parse_willingness(reply).map(|(score, _)| score)
}

// parses the person's name out of a target-pick reply: JSON first, falling back to the first line/word
/// Matches a known name case-insensitively, otherwise returns the raw candidate.
/// `hedef` is a protocol key matching the target-pick prompt's JSON.
fn extract_target(reply: &str, known: &[String]) -> Option<String> {
    #[derive(Deserialize)]
    struct Target {
        #[serde(default)]
        hedef: String,
    }
    let candidate = serde_json::from_str::<Target>(extract_json(reply))
        .map(|h| h.hedef.trim().to_string())
        .unwrap_or_else(|_| {
            reply
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .to_string()
        });
    if candidate.is_empty() {
        return None;
    }
    // if it resembles one of the known names, use that instead (the model may have dressed it up)
    known
        .iter()
        .find(|b| b.eq_ignore_ascii_case(&candidate))
        .cloned()
        .or(Some(candidate))
}

// parses the mood state + intensity out of a mood reply; low intensity (neutral) isn't worth reflecting at all
/// Returns `<state> (<intensity>)`; rejects malformed/empty states or intensity below 3.
/// `durum`/`yogunluk` are protocol keys matching the mood prompt's JSON.
fn extract_mood(reply: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Mood {
        #[serde(default)]
        durum: String,
        #[serde(default)]
        yogunluk: i32,
    }
    let r: Mood = serde_json::from_str(extract_json(reply)).ok()?;
    let state = r.durum.trim();
    if state.is_empty() {
        return None;
    }
    let intensity = r.yogunluk.clamp(1, 10);
    if intensity < 3 {
        return None; // neutral/unclear: not worth adding to the instruction, let plain personality talk
    }
    Some(format!("{state} ({intensity})"))
}
