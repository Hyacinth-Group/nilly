/// Parses a single SSE line.
/// Ignores non-data lines and malformed JSON; `[DONE]` marks the stream complete.
fn extract_sse(line: &str) -> Option<SseData> {
    let data = line.trim().strip_prefix("data:")?.trim();
    if data == "[DONE]" {
        return Some(SseData {
            done: true,
            ..Default::default()
        });
    }
    let response: StreamResponse = serde_json::from_str(data).ok()?;
    let usage = response.usage;
    let chunk = response.choices.into_iter().next().and_then(|s| {
        let text = s.delta.content.unwrap_or_default();
        let thought = [s.delta.reasoning, s.delta.reasoning_content]
            .into_iter()
            .flatten()
            .find(|s| !s.is_empty())
            .unwrap_or_default();
        (!text.is_empty() || !thought.is_empty()).then_some(Chunk { text, thought })
    });
    (chunk.is_some() || usage.is_some()).then_some(SseData {
        chunk,
        usage,
        done: false,
    })
}

// reader for a streaming request; each call hands back the next chunk, None once the stream ends
/// Pull-based reader over one streaming HTTP response. Holds: `response` (the raw
/// `reqwest::Response` body being read chunk-by-chunk), `buffer` (bytes not yet split into
/// lines), `queue` (parsed `Chunk`s waiting to be handed out), `usage` (running total, see
/// `apply_data`), `category` (for `add_metric`), `done` (saw `[DONE]`), `finished` (the HTTP
/// body itself is exhausted). Built by `Bot::ask_raw_stream` (`provider_ask.rs`); driven by
/// `Bot::send_stream` (`provider_send_stream.rs`) via repeated calls to `next`.
struct StreamReader {
    response: reqwest::Response,
    buffer: Vec<u8>,        // bytes not yet split into lines
    queue: Vec<Chunk>,      // parsed chunks waiting to be handed out
    usage: Usage,           // token counters accumulated from chunks seen so far
    category: &'static str, // for the token metrics breakdown (!durum)
    done: bool,             // whether [DONE] was seen (a clean end-of-stream marker)
    finished: bool,
}

impl StreamReader {
    /// Fetches the next parsed chunk, reading and buffering more of the HTTP body as needed.
    /// Returns `None` when fully drained, or an error on a body-read failure.
    async fn next(&mut self) -> Result<Option<Chunk>, BotError> {
        loop {
            if let Some(p) = self.queue.pop() {
                return Ok(Some(p));
            }
            if self.finished {
                if self.buffer.is_empty() {
                    return Ok(None);
                }
                // a trailing chunk with no newline can be left over
                let line = String::from_utf8_lossy(&self.buffer).into_owned();
                self.buffer.clear();
                if let Some(v) = extract_sse(&line) {
                    self.apply_data(&v);
                    if let Some(p) = v.chunk {
                        return Ok(Some(p));
                    }
                }
                continue;
            }
            match self.response.chunk().await? {
                Some(p) => {
                    self.buffer.extend_from_slice(&p);
                    self.process_lines();
                }
                None => self.finished = true,
            }
        }
    }

    // applies the done/usage side effects; the chunk itself is returned, not queued
    fn apply_data(&mut self, v: &SseData) {
        if v.done {
            self.done = true;
        }
        if let Some(k) = v.usage {
            self.usage.add(k);
        }
    }

    // only complete lines are processed; trailing incomplete bytes wait in the buffer
    // (even if a utf-8 character is split across chunks, it resolves once the line completes)
    /// Splits `self.buffer` on `\n`, parses each complete line, and queues the resulting
    /// chunks in arrival order, retaining any incomplete trailing line.
    fn process_lines(&mut self) {
        let mut start = 0;
        let mut items = Vec::new();
        for (i, b) in self.buffer.iter().enumerate() {
            if *b == b'\n' {
                let line = String::from_utf8_lossy(&self.buffer[start..i]);
                if let Some(v) = extract_sse(&line) {
                    items.push(v);
                }
                start = i + 1;
            }
        }
        self.buffer.drain(..start);
        let before = self.queue.len();
        for v in items {
            self.apply_data(&v);
            if let Some(p) = v.chunk {
                self.queue.push(p);
            }
        }
        // newly added items are in arrival order; reversed so pop() hands out the earliest first
        self.queue[before..].reverse();
    }
}

// outcome of sending a stream
/// What `Bot::send_stream` (`provider_send_stream.rs`) ended up doing. Holds no data beyond
/// the variant: `Sent(String)` — the final protocol text that was sent (see
/// `Reply::protocol_text`); `Empty` — nothing usable came out; `Silent` — the model asked to
/// stay silent, nothing was sent or recorded to history.
enum StreamResult {
    Sent(String), // the final text was sent
    Empty,        // nothing usable came out of the stream
    Silent,       // the model chose to stay silent ("-"): nothing is sent, and it's not added to history either
}

// reply context for send_stream; a single struct instead of a pile of arguments
/// Bundles `Bot::send_stream`'s (`provider_send_stream.rs`) arguments. Holds: `bot_name`
/// (for stripping the name prefix from streamed text); `reply_to` (normalized reply target,
/// if any); `reaction_target` (message an emoji reaction lands on — always the chat's last
/// message, independent of `reply_to`, see the field comment); `history`/`instruction`/
/// `budget` (passed straight through to a fallback `generate` call on repeat/empty).
struct StreamContext<'a> {
    bot_name: &'a str,
    reply_to: Option<&'a MessageKey>,
    // message the emoji reaction lands on; separate field since reply_to is conditional
    reaction_target: Option<&'a MessageKey>,
    history: &'a [ChatMessage],
    instruction: &'a str,
    budget: Option<u32>,
}
