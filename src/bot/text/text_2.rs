/// The parsed form of one model reply, per the line-based output protocol. Holds `lines`
/// (text lines to send, one Discord message each), `reaction` (an emoji to react with
/// instead of/alongside text), `silent` (the model asked to say nothing). Produced by
/// `parse_reply` (`text_3.rs`); consumed by `Bot::send_stream`/`send_reply`
/// (`provider_send_stream.rs`/`provider_send_line.rs`).
#[derive(Default, Debug, PartialEq, Eq)]
struct Reply {
    lines: Vec<String>,
    reaction: Option<String>,
    silent: bool,
}

impl Reply {
    // did nothing usable come out of the reply (no words, no reaction, no silence decision)?
    fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.reaction.is_none() && !self.silent
    }

    // the form that goes into chat history and the channel notes: the model sees its own
    // protocol reflected back, and writes in the same form next turn
    fn protocol_text(&self) -> String {
        let mut s = self.lines.join("\n");
        if let Some(t) = &self.reaction {
            if !s.is_empty() {
                s.push('\n');
            }
            s.push_str("tepki: ");
            s.push_str(t);
        }
        s
    }
}

// does the line start with "tepki:" (case-insensitive, "tepki :" also counts)?
// if so, returns what's after the colon
/// Matches "tepki" case-insensitively, allowing whitespace before the colon.
fn reaction_body(line: &str) -> Option<&str> {
    let (head, rest) = line.split_once(':')?;
    (casefold(head.trim()) == "tepki").then_some(rest)
}

// is this character actually an emoji? "not a letter" isn't a good enough test: "—", "…",
// "→", and typographic quotes all pass that test too, and sending one to Discord as a
// Unicode reaction gets the request rejected with a 400. Known emoji blocks are counted,
// nothing else qualifies as a reaction.
fn emoji_start(c: char) -> bool {
    matches!(c as u32,
        0xA9 | 0xAE | 0x2122 | 0x3030 | 0x303D | 0x3297 | 0x3299
        | 0x2600..=0x27BF   // misc symbols + dingbats
        | 0x2B00..=0x2BFF   // arrow/star/square symbols (like ⭐)
        | 0x1F000..=0x1FAFF // the actual emoji blocks (flags, skin tone included)
    )
}

// characters that can trail an emoji sequence: variation selector, ZWJ, keycap
fn emoji_continues(c: char) -> bool {
    emoji_start(c) || matches!(c as u32, 0xFE0F | 0xFE0E | 0x200D | 0x20E3)
}

// the first emoji sequence in the text; a trailing variation selector/ZWJ is included too.
// None for a custom emoji form like ":kekw:" or when there's no emoji at all.
/// Caps the first emoji sequence at eight characters.
fn extract_emoji(text: &str) -> Option<String> {
    let (head, _) = text.char_indices().find(|(_, c)| emoji_start(*c))?;
    Some(
        text[head..]
            .chars()
            .take_while(|c| emoji_continues(*c))
            .take(8)
            .collect(),
    )
}

// is the line, on its own, the silence marker?
/// Recognizes a dash (optionally quoted), [sus], or (sus); bracketed forms ignore case.
fn silence_marker(line: &str) -> bool {
    matches!(line, "-" | "\"-\"" | "'-'") || matches!(casefold(line).as_str(), "[sus]" | "(sus)")
}

// what follows a "1. " / "2) "-style number prefix (digits are single-byte, so the slice
// is safe). Not dropped on its own: "3. sınıftayım" ("I'm in 3rd grade"), "2. el araba"
// ("2nd-hand car") are ordinals in Turkish, not list markers.
fn number_prefix(s: &str) -> Option<&str> {
    let digits = s.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let rest = &s[digits..];
    rest.strip_prefix(". ")
        .or_else(|| rest.strip_prefix(") "))
        .map(str::trim_start)
}

// strips "written by AI" tells: a leading bullet marker and bold/underline markdown.
// The number prefix is NOT stripped here but in parse_reply (only looking at the whole
// reply makes it clear whether it's really a list or an ordinal number). The INSIDE of a
// backtick is preserved: something like `__init__` is a code fragment carrying real information.
fn clean_slop(line: &str) -> String {
    let mut s = line.trim();
    for prefix in ["- ", "* ", "• "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim_start();
            break;
        }
    }
    let mut output = String::with_capacity(s.len());
    // splitting on backtick means the odd-indexed parts are inside code and are left alone
    for (i, part) in s.split('`').enumerate() {
        if i > 0 {
            output.push('`');
        }
        if i % 2 == 0 {
            output.push_str(&part.replace("**", "").replace("__", ""));
        } else {
            output.push_str(part);
        }
    }
    output.trim().to_string()
}

// Parses the model's reply according to the protocol. A short line isn't dropped: "he"
// ("yeah"), "yok" ("nope"), "la" (a casual interjection) are all natural replies too.
