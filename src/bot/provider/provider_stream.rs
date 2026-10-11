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

/// The most recent provider-reported cumulative usage, with presence preserved across
/// partial snapshots. Repeated final payloads cannot multiply token totals.
#[derive(Default)]
struct StreamUsage {
    reported: Option<Usage>,
}

impl StreamUsage {
    fn observe(&mut self, usage: Option<Usage>) {
        if let Some(usage) = usage {
            if let Some(current) = self.reported.as_mut() {
                current.update_snapshot(usage);
            } else {
                self.reported = Some(usage);
            }
        }
    }
}

// reader for a streaming request; each call hands back the next chunk, None once the stream ends
/// Pull-based reader over one streaming HTTP response. Holds: `response` (the raw
/// `reqwest::Response` body being read chunk-by-chunk), `buffer` (bytes not yet split into
/// lines), `queue` (parsed `Chunk`s waiting to be handed out), `usage` (provider usage
/// snapshots, see `apply_data`), `category` (for `add_metric`), `done` (saw `[DONE]`),
/// `finished` (the HTTP body itself is exhausted). Built by `Bot::ask_raw_stream`
/// (`provider_ask.rs`); driven by `Bot::send_stream` via repeated calls to `next`.
struct StreamReader {
    response: reqwest::Response,
    buffer: Vec<u8>,        // bytes not yet split into lines
    queue: Vec<Chunk>,      // parsed chunks waiting to be handed out
    usage: StreamUsage,     // absent until a provider actually sends a usage object
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
        self.usage.observe(v.usage);
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

#[cfg(test)]
mod stream_usage_reporting_tests {
    use super::*;

    fn observe_lines(lines: &[&str]) -> StreamUsage {
        let mut usage = StreamUsage::default();
        for line in lines {
            if let Some(data) = extract_sse(line) {
                usage.observe(data.usage);
            }
        }
        usage
    }

    #[test]
    fn stream_without_usage_keeps_unknown_fields() {
        let usage = observe_lines(&[
            r#"data: {"choices":[{"delta":{"content":"answer"}}],"usage":null}"#,
            "data: [DONE]",
        ]);
        assert!(usage.reported.is_none());
        let mut metrics = Metrics::default();
        metrics.observe("chat", usage.reported, MetricObservation::Stream { completed: true }, 1);
        assert_eq!(metrics.calls, 1);
        let totals = &metrics.reporting.totals;
        assert_eq!(totals.counted_successful_calls, 1);
        assert_eq!(totals.usage_reported_calls, 0);
        for field in [&totals.input, &totals.output, &totals.cache] {
            assert_eq!(field.known_total, None);
            assert_eq!(field.unknown_calls, 1);
        }
    }

    #[test]
    fn partial_and_repeated_final_stream_usage_is_one_snapshot() {
        let usage = observe_lines(&[
            r#"data: {"choices":[],"usage":{"prompt_tokens":3}}"#,
            r#"data: {"choices":[{"delta":{"content":"answer"}}],"usage":{"completion_tokens":1}}"#,
            r#"data: {"choices":[],"usage":{"prompt_tokens":7,"completion_tokens":13,"prompt_tokens_details":{"cached_tokens":5}}}"#,
            r#"data: {"choices":[],"usage":{"prompt_tokens":7,"completion_tokens":13,"prompt_tokens_details":{"cached_tokens":5}}}"#,
            "data: [DONE]",
        ]);
        let mut metrics = Metrics::default();
        metrics.observe("chat", usage.reported, MetricObservation::Stream { completed: true }, 1);
        assert_eq!(metrics.calls, 1);
        assert_eq!(metrics.input_tokens, 7);
        assert_eq!(metrics.output_tokens, 13);
        assert_eq!(metrics.cache_tokens, 5);
        assert_eq!(metrics.input_tokens + metrics.output_tokens, 20);
        let totals = &metrics.reporting.totals;
        assert_eq!(totals.counted_successful_calls, 1);
        assert_eq!(totals.usage_reported_calls, 1);
        for field in [&totals.input, &totals.output, &totals.cache] {
            assert_eq!(field.reported_calls, 1);
            assert_eq!(field.unknown_calls, 0);
        }
    }

    #[test]
    fn partial_stream_usage_retains_only_reported_fields() {
        let usage = observe_lines(&[
            r#"data: {"choices":[],"usage":{"prompt_tokens":0}}"#,
            r#"data: {"choices":[],"usage":{"completion_tokens":4}}"#,
            "data: [DONE]",
        ]);
        let mut metrics = Metrics::default();
        metrics.observe("chat", usage.reported, MetricObservation::Stream { completed: true }, 1);
        let totals = &metrics.reporting.totals;
        assert_eq!(totals.input.known_total, Some(0));
        assert_eq!(totals.output.known_total, Some(4));
        assert_eq!(totals.cache.known_total, None);
        assert_eq!(totals.cache.unknown_calls, 1);
    }

    #[test]
    fn empty_usage_object_differs_from_no_stream_usage() {
        let usage = observe_lines(&[r#"data: {"choices":[],"usage":{}}"#, "data: [DONE]"]);
        assert!(usage.reported.is_some());
        let mut metrics = Metrics::default();
        metrics.observe("chat", usage.reported, MetricObservation::Stream { completed: true }, 1);
        assert_eq!(metrics.reporting.totals.usage_reported_calls, 1);
        assert_eq!(metrics.reporting.totals.input.known_total, None);
        assert_eq!(metrics.reporting.totals.input.unknown_calls, 1);
    }

    #[test]
    fn failed_stream_is_not_claimed_as_successful_reporting_coverage() {
        let usage = observe_lines(&[
            r#"data: {"choices":[],"usage":{"prompt_tokens":7,"prompt_tokens_details":{"cached_tokens":5}}}"#,
            r#"data: {"choices":[],"usage":{"prompt_tokens":7,"prompt_tokens_details":{"cached_tokens":5}}}"#,
        ]);
        let mut metrics = Metrics::default();
        metrics.observe("chat", usage.reported, MetricObservation::Stream { completed: false }, 1);
        // Existing counters counted even a read-failed stream, so retain that behavior.
        assert_eq!(metrics.calls, 1);
        assert_eq!(metrics.input_tokens, 7);
        assert_eq!(metrics.reporting.totals.counted_successful_calls, 0);
        assert_eq!(metrics.reporting.totals.usage_reported_calls, 1);
        assert_eq!(metrics.reporting.totals.input.known_total, Some(7));
        assert_eq!(metrics.reporting.totals.input.reported_calls, 1);
        assert_eq!(metrics.reporting.totals.input.unknown_calls, 0);
        assert_eq!(metrics.reporting.totals.cache.known_total, Some(5));
        assert_eq!(metrics.reporting.totals.cache.reported_calls, 1);
        assert_eq!(metrics.reporting.totals.cache.unknown_calls, 0);
        assert_eq!(metrics.reporting.totals.output.known_total, None);
        assert_eq!(metrics.reporting.totals.output.unknown_calls, 1);
        let category = &metrics.reporting.categories["chat"];
        assert_eq!(category.counted_successful_calls, 0);
        assert_eq!(category.usage_reported_calls, 1);
        assert_eq!(category.input.known_total, Some(7));
    }

    #[test]
    fn failed_stream_without_usage_counts_unknown_fields_once() {
        let mut metrics = Metrics::default();
        metrics.observe("chat", None, MetricObservation::Stream { completed: false }, 1);
        assert_eq!(metrics.calls, 1);
        let totals = &metrics.reporting.totals;
        assert_eq!(totals.counted_successful_calls, 0);
        assert_eq!(totals.usage_reported_calls, 0);
        for field in [&totals.input, &totals.output, &totals.cache] {
            assert_eq!(field.known_total, None);
            assert_eq!(field.reported_calls, 0);
            assert_eq!(field.unknown_calls, 1);
        }
    }
}
