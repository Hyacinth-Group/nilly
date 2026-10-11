// A terminal chat bench: try out the reply protocol (line = message, `tepki:` emoji, `-`
// silence) without connecting to Discord. Opened with `cargo run -- chat`; every input
// line is "name: text" — without a colon, the speaker defaults to `misafir` — `!quit` or EOF
// exits. durum/ files load normally so the personality feels real, but nothing gets
// WRITTEN to disk from here: channel history and memory are kept in memory only.

use super::*;
use std::io::Write;

// the CLI's fake channel: not a real Discord channel, just the key for chat state
const CLI_CHANNEL: u64 = 1;

// appends to channel history in MEMORY ONLY; channel_note does the same thing but also
// writes to disk — the bench shouldn't pollute the real durum/kanallar files
fn append_history(state: &mut State, channel: ChannelId, line: String) {
    let hist = state.channel_history.entry(channel).or_default();
    hist.push_back(line);
    while hist.len() > CHANNEL_HISTORY {
        hist.pop_front();
    }
}

// parses "name: text"; without a colon, or with an empty side, the speaker defaults to "misafir"
fn parse_line(line: &str) -> (String, String) {
    match line.split_once(':') {
        Some((name, text)) if !name.trim().is_empty() && !text.trim().is_empty() => {
            (name.trim().to_string(), text.trim().to_string())
        }
        _ => ("misafir".to_string(), line.trim().to_string()),
    }
}

impl Bot {
    /// Runs the interactive terminal chat loop until `!quit`/EOF.
    pub async fn chat_cli(&self) {
        let channel = ChannelId::new(CLI_CHANNEL);
        {
            let mut state = self.state();
            // the ready event never fires, so the name stays empty: use the chosen name if
            // there is one, otherwise plain "bot" (strip_name and the output both depend on a name)
            if state.bot_name.is_empty() {
                state.bot_name = state
                    .growth
                    .name
                    .clone()
                    .unwrap_or_else(|| "bot".to_string());
            }
            start_chat(&mut state, channel, None);
        }
        println!("chat mode — type \"name: text\"; !quit to exit (or ctrl-d)");
        // stdin is read blocking: this mode has no background cycle, a separate reader isn't worth it
        let stdin = std::io::stdin();
        let mut raw = String::new();
        loop {
            print!("> ");
            let _ = std::io::stdout().flush();
            raw.clear();
            match stdin.read_line(&mut raw) {
                Ok(0) => break, // EOF
                Ok(_) => {}
                Err(e) => {
                    eprintln!("couldn't read input: {e}");
                    break;
                }
            }
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            if line == "!quit" {
                break;
            }
            let (name, text) = parse_line(line);

            // the incoming line: memory, channel history, and chat history (same shape as the live message handler)
            {
                let mut state = self.state();
                remember(&mut state, &name, &text);
                append_history(&mut state, channel, format!("{name}: {text}"));
                if let Some(chat) = state.chats.get_mut(&channel) {
                    chat.history.push(user(format!("{name}: {text}")));
                    if chat.history.len() > CHAT_SIZE {
                        chat.history.drain(..chat.history.len() - CHAT_SIZE);
                    }
                }
            }
            let (history, instruction) = {
                let state = self.state();
                let history = state
                    .chats
                    .get(&channel)
                    .map(|chat| chat.history.clone())
                    .unwrap_or_default();
                // same question cap as live: code measures it, the model does the writing
                let instruction = if too_many_questions(&state, channel) {
                    "Bu sefer soru sorma; düz laf et ya da sus."
                } else {
                    ""
                };
                (history, instruction)
            };

            // no streaming: the bench isn't about stream pacing, it's measuring the resulting protocol
            let generated = match self
                .generate(&history, instruction, chat_budget(), "sohbet")
                .await
            {
                Ok(c) => c,
                Err(e) => {
                    println!("(error: {e})");
                    continue;
                }
            };
            let bot_name = self.state().bot_name.clone();
            let reply = parse_reply(strip_name(&generated, &bot_name));
            if reply.lines.is_empty() && reply.reaction.is_none() {
                println!("{}", if reply.silent { "(silent)" } else { "(empty)" });
                continue;
            }
            for line in &reply.lines {
                println!("{bot_name}: {line}");
            }
            if let Some(emoji) = &reply.reaction {
                println!("[reaction {emoji}]");
            }
            // fed back into history in protocol form: the model should see its own format next turn
            {
                let mut state = self.state();
                if let Some(chat) = state.chats.get_mut(&channel) {
                    chat.history.push(assistant(reply.protocol_text()));
                    chat.counter += 1;
                }
                for line in &reply.lines {
                    append_history(&mut state, channel, format!("{bot_name}: {line}"));
                }
                if let Some(emoji) = &reply.reaction {
                    append_history(&mut state, channel, format!("{bot_name}: tepki: {emoji}"));
                }
            }
        }
        println!("exited");
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// Verifies `parse_line`'s `name: text` splitting, including the no-colon fallback to "misafir" and the edge case of a colon inside the text.
    #[test]
    fn line_parses() {
        assert_eq!(parse_line("emin: selam"), ("emin".into(), "selam".into()));
        assert_eq!(
            parse_line("Zeynep : naber"),
            ("Zeynep".into(), "naber".into())
        );
        // no colon: speaker defaults to misafir
        assert_eq!(parse_line("selam"), ("misafir".into(), "selam".into()));
        // one side of the colon empty: the whole line counts as text
        assert_eq!(parse_line("saat 3:"), ("misafir".into(), "saat 3:".into()));
    }

    /// Verifies `append_history` caps a channel's in-memory history at `CHANNEL_HISTORY`, dropping the oldest lines first.
    #[test]
    fn history_limited_in_memory() {
        let channel = ChannelId::new(CLI_CHANNEL);
        let mut state = State::default();
        for i in 0..CHANNEL_HISTORY + 5 {
            append_history(&mut state, channel, format!("emin: {i}"));
        }
        let hist = &state.channel_history[&channel];
        assert_eq!(hist.len(), CHANNEL_HISTORY);
        assert_eq!(hist.front().unwrap(), "emin: 5");
    }
}
