// ---------- AI ----------

/// Deserialize target for a non-streaming `/chat/completions` response. Holds `choices`
/// (one per requested completion; the crate always requests one) and the optional `usage`
/// counters. Produced by `serde_json::from_str` in `Bot::ask_raw` (`provider_ask_raw.rs`).
#[derive(Deserialize)]
struct Response {
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<Usage>,
}
/// One entry of `Response.choices`: wraps the actual `message`. See `Content` below.
#[derive(Deserialize)]
struct Choice {
    message: Content,
}
/// The `message` object inside a `Choice`: holds the reply `content` and, on
/// reasoning-capable models, the `reasoning`/`reasoning_content` thought fields.
#[derive(Deserialize)]
struct Content {
    content: Option<String>,
    // reasoning-mandatory models (like glm-5.3-flash) sometimes bury the reply inside the
    // thought field instead of content
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
}

// call categories that expect JSON back: when content is empty, the JSON block found
// inside the thought field is treated as the content instead. Values are Turkish because
// they're the same tags shown in the !durum usage breakdown embed (Discord-facing).
const JSON_CATEGORIES: [&str; 5] = ["gunlukcu", "isteklilik", "hedef_sec", "ruh_hali", "uyanis"];

// pulls the reply content out of a model response: content if it's non-empty. If it's
// empty and the caller expects JSON, a `{ … }` block is searched for inside the reasoning
// field (reasoning-mandatory models can bury the reply in the thought); on a plain-prose
// call, reasoning is NEVER treated as content (the coach agent shouldn't mistake a chain
// of thought for the temperament summary it asked for).
/// Uses non-empty reply content first. For JSON categories only, falls back to a
/// JSON-validated object extracted from the thought.
fn response_content(content: &Content, category: &str) -> Option<String> {
    if let Some(c) = content
        .content
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(c.to_string());
    }
    if !JSON_CATEGORIES.contains(&category) {
        return None;
    }
    let thought = content
        .reasoning_content
        .as_deref()
        .or(content.reasoning.as_deref())?;
    let candidate = extract_json(thought);
    (candidate.starts_with('{') && serde_json::from_str::<serde_json::Value>(candidate).is_ok())
        .then(|| candidate.to_string())
}

// length of the thought in a response (for the error message: did the thought eat the whole budget)
/// Counts Unicode characters in the thought, or zero when absent.
fn thought_length(content: &Content) -> usize {
    content
        .reasoning_content
        .as_deref()
        .or(content.reasoning.as_deref())
        .map_or(0, |r| r.chars().count())
}

// Numeric fields remain compatible with the existing process totals and status views.
// Presence is retained separately: a default value is not a provider-reported zero.
#[derive(Deserialize, Default, Clone, Copy, Debug)]
#[serde(from = "ReportedUsage")]
struct Usage {
    prompt_tokens: u64,
    completion_tokens: u64,
    prompt_tokens_details: CacheDetail,
    prompt_tokens_reported: bool,
    completion_tokens_reported: bool,
    cache_tokens_reported: bool,
}

#[derive(Deserialize, Default, Clone, Copy, Debug)]
struct CacheDetail {
    #[serde(default)]
    cached_tokens: u64,
}

#[derive(Deserialize)]
struct ReportedUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    prompt_tokens_details: Option<ReportedCacheDetail>,
}

#[derive(Deserialize)]
struct ReportedCacheDetail {
    cached_tokens: Option<u64>,
}

impl From<ReportedUsage> for Usage {
    fn from(value: ReportedUsage) -> Self {
        let cached = value.prompt_tokens_details.and_then(|detail| detail.cached_tokens);
        Self {
            prompt_tokens: value.prompt_tokens.unwrap_or_default(),
            completion_tokens: value.completion_tokens.unwrap_or_default(),
            prompt_tokens_details: CacheDetail {
                cached_tokens: cached.unwrap_or_default(),
            },
            prompt_tokens_reported: value.prompt_tokens.is_some(),
            completion_tokens_reported: value.completion_tokens.is_some(),
            cache_tokens_reported: cached.is_some(),
        }
    }
}

impl Usage {
    /// Adds another `Usage`'s counters into `self` (field-by-field sum).
    fn add(&mut self, other: Usage) {
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
        self.prompt_tokens_details.cached_tokens += other.prompt_tokens_details.cached_tokens;
    }

    /// Stream usage payloads are cumulative snapshots, not additive deltas.
    /// Retain earlier reported fields when a later partial snapshot omits them.
    fn update_snapshot(&mut self, other: Usage) {
        if other.prompt_tokens_reported {
            self.prompt_tokens = other.prompt_tokens;
            self.prompt_tokens_reported = true;
        }
        if other.completion_tokens_reported {
            self.completion_tokens = other.completion_tokens;
            self.completion_tokens_reported = true;
        }
        if other.cache_tokens_reported {
            self.prompt_tokens_details.cached_tokens = other.prompt_tokens_details.cached_tokens;
            self.cache_tokens_reported = true;
        }
    }
}

/// Legacy process totals, plus presence-aware accounting for successfully observed
/// provider responses. Neither accounting represents all billed attempts.
#[derive(Default, Clone, Debug)]
struct Metrics {
    calls: u32,
    input_tokens: u64,
    cache_tokens: u64, // the portion of input_tokens served from cache (when the provider reports it)
    output_tokens: u64,
    last_call_secs: i64,
    categories: HashMap<&'static str, Usage>,
    reporting: UsageReporting,
}

/// A known subtotal and its coverage. Cache tokens are a subset of input, not extra input.
#[derive(Default, Clone, Debug, Serialize)]
struct ReportedTokens {
    known_total: Option<u64>,
    reported_calls: u32,
    unknown_calls: u32,
}

impl ReportedTokens {
    fn observe(&mut self, value: Option<u64>) {
        if let Some(value) = value {
            *self.known_total.get_or_insert(0) += value;
            self.reported_calls += 1;
        } else {
            self.unknown_calls += 1;
        }
    }
}

#[derive(Default, Clone, Debug, Serialize)]
struct CategoryUsageReporting {
    counted_successful_calls: u32,
    usage_reported_calls: u32,
    input: ReportedTokens,
    output: ReportedTokens,
    cache: ReportedTokens,
}

impl CategoryUsageReporting {
    fn observe(&mut self, usage: Option<Usage>, successful: bool) {
        self.counted_successful_calls += u32::from(successful);
        self.usage_reported_calls += u32::from(usage.is_some());
        self.input.observe(usage.filter(|u| u.prompt_tokens_reported).map(|u| u.prompt_tokens));
        self.output.observe(
            usage.filter(|u| u.completion_tokens_reported).map(|u| u.completion_tokens),
        );
        self.cache.observe(
            usage.filter(|u| u.cache_tokens_reported)
                .map(|u| u.prompt_tokens_details.cached_tokens),
        );
    }
}

/// Coverage includes decoded successful HTTP responses and every drained/read-failed
/// stream observation. Reported fields survive stream failure; only missing fields are
/// unknown. `counted_successful_calls` excludes read-failed streams independently.
/// Empty replies retried after a decoded response are still observations.
/// Costs/currencies are intentionally absent: the existing parser has no normalized
/// monetary reporting contract, and token totals cannot establish one.
#[derive(Default, Clone, Debug, Serialize)]
struct UsageReporting {
    #[serde(flatten)]
    totals: CategoryUsageReporting,
    categories: HashMap<&'static str, CategoryUsageReporting>,
}

enum MetricObservation {
    Response,
    Stream { completed: bool },
}

impl Metrics {
    fn observe(
        &mut self,
        category: &'static str,
        usage: Option<Usage>,
        observation: MetricObservation,
        at: i64,
    ) {
        // Keep existing counters: non-stream responses used to count only with usage;
        // streams were counted once even when usage was missing or reading failed.
        if usage.is_some() || matches!(observation, MetricObservation::Stream { .. }) {
            let legacy = usage.unwrap_or_default();
            self.calls += 1;
            self.input_tokens += legacy.prompt_tokens;
            self.cache_tokens += legacy.prompt_tokens_details.cached_tokens;
            self.output_tokens += legacy.completion_tokens;
            self.last_call_secs = at;
            self.categories.entry(category).or_default().add(legacy);
        }
        let successful = !matches!(observation, MetricObservation::Stream { completed: false });
        self.reporting.totals.observe(usage, successful);
        self.reporting.categories.entry(category).or_default().observe(usage, successful);
    }
}

// one streamed chunk: reasoning models also send a thought, plain models send content only
/// One piece of a streamed reply: `text` (a content delta) and/or `thought` (a reasoning
/// delta). Produced by `extract_sse`, consumed by `StreamReader::next`.
#[derive(Default, Clone, PartialEq)]
struct Chunk {
    text: String,
    thought: String,
}

/// Deserialize target for one streamed SSE `data:` payload. Holds `choices` (usually one
/// `StreamChoice`, empty on the final usage-only chunk) and the optional `usage`, which
/// arrives once `stream_options.include_usage` is set (see `Bot::ask_raw_stream`).
#[derive(Deserialize)]
struct StreamResponse {
    #[serde(default)]
    choices: Vec<StreamChoice>,
    // arrives on the final chunk when include_usage is set; choices can be empty then
    #[serde(default)]
    usage: Option<Usage>,
}
/// One entry of `StreamResponse.choices`: wraps the incremental `delta`. See `StreamDelta`.
#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}
/// The incremental `delta` object inside a `StreamChoice`: `content` and/or a
/// `reasoning`/`reasoning_content` field, depending on the provider (see field comment).
#[derive(Deserialize, Default)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
    // openrouter calls it "reasoning", OpenAI-compatible routers call it "reasoning_content"
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
}

// what comes out of one SSE line: a content chunk and/or a usage counter
/// Result of parsing one SSE line via `extract_sse`. Holds `chunk` (content/thought, if the
/// line carried any), `usage` (if this was the final usage-only chunk), and `done` (true for
/// the literal `data: [DONE]` line). Consumed by `StreamReader::apply_data`/`process_lines`.
#[derive(Default)]
struct SseData {
    chunk: Option<Chunk>,
    usage: Option<Usage>,
    done: bool,
}

#[cfg(test)]
mod usage_reporting_tests {
    use super::*;

    fn usage(value: serde_json::Value) -> Usage {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn missing_usage_and_reported_zero_are_distinct() {
        let mut metrics = Metrics::default();
        metrics.observe("test", None, MetricObservation::Response, 1);
        metrics.observe(
            "test",
            Some(usage(serde_json::json!({
                "prompt_tokens": 0,
                "completion_tokens": 0,
                "prompt_tokens_details": {"cached_tokens": 0}
            }))),
            MetricObservation::Response,
            2,
        );
        let totals = &metrics.reporting.totals;
        assert_eq!(totals.counted_successful_calls, 2);
        assert_eq!(totals.usage_reported_calls, 1);
        for field in [&totals.input, &totals.output, &totals.cache] {
            assert_eq!(field.known_total, Some(0));
            assert_eq!(field.reported_calls, 1);
            assert_eq!(field.unknown_calls, 1);
        }
        // Preserve the legacy rule: absent non-stream usage never incremented calls.
        assert_eq!(metrics.calls, 1);
        assert_eq!(metrics.input_tokens + metrics.output_tokens, 0);
    }

    #[test]
    fn partial_usage_and_categories_keep_independent_coverage() {
        let mut metrics = Metrics::default();
        metrics.observe(
            "chat",
            Some(usage(serde_json::json!({"prompt_tokens": 10}))),
            MetricObservation::Response,
            1,
        );
        metrics.observe(
            "agent",
            Some(usage(serde_json::json!({"completion_tokens": 4}))),
            MetricObservation::Response,
            2,
        );
        metrics.observe("chat", None, MetricObservation::Response, 3);
        let totals = &metrics.reporting.totals;
        assert_eq!(totals.counted_successful_calls, 3);
        assert_eq!(totals.usage_reported_calls, 2);
        assert_eq!(totals.input.known_total, Some(10));
        assert_eq!(totals.output.known_total, Some(4));
        assert_eq!(totals.input.unknown_calls, 2);
        assert_eq!(totals.output.unknown_calls, 2);
        assert_eq!(totals.cache.known_total, None);
        assert_eq!(totals.cache.unknown_calls, 3);
        let chat = &metrics.reporting.categories["chat"];
        assert_eq!(chat.counted_successful_calls, 2);
        assert_eq!(chat.usage_reported_calls, 1);
        assert_eq!(chat.input.known_total, Some(10));
        assert_eq!(chat.input.unknown_calls, 1);
        assert_eq!(chat.output.known_total, None);
        let agent = &metrics.reporting.categories["agent"];
        assert_eq!(agent.counted_successful_calls, 1);
        assert_eq!(agent.input.known_total, None);
        assert_eq!(agent.output.known_total, Some(4));
    }

    #[test]
    fn valid_provider_response_retains_totals_with_cached_input_subset() {
        let response: Response = serde_json::from_value(serde_json::json!({
            "choices": [{"message": {"content": "answer"}}],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 25,
                "total_tokens": 125,
                "prompt_tokens_details": {"cached_tokens": 80}
            }
        })).unwrap();
        assert_eq!(response_content(&response.choices[0].message, "sohbet").as_deref(), Some("answer"));
        let mut metrics = Metrics::default();
        metrics.observe("chat", response.usage, MetricObservation::Response, 10);
        metrics.observe("chat", response.usage, MetricObservation::Response, 11);
        assert_eq!(metrics.calls, 2);
        assert_eq!(metrics.input_tokens, 200);
        assert_eq!(metrics.output_tokens, 50);
        assert_eq!(metrics.cache_tokens, 160);
        assert_eq!(metrics.input_tokens + metrics.output_tokens, 250);
        assert_eq!(metrics.categories["chat"].prompt_tokens, 200);
        assert_eq!(metrics.categories["chat"].completion_tokens, 50);
        assert_eq!(metrics.categories["chat"].prompt_tokens_details.cached_tokens, 160);
        let reporting = serde_json::to_value(&metrics.reporting).unwrap();
        assert_eq!(reporting["input"]["known_total"], 200);
        assert_eq!(reporting["cache"]["known_total"], 160);
        assert!(reporting.get("cost").is_none());
        assert!(reporting.get("currency").is_none());
    }

    #[test]
    fn empty_and_null_usage_fields_are_not_reported_zero() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({
                "prompt_tokens": null,
                "completion_tokens": null,
                "prompt_tokens_details": null
            }),
            serde_json::json!({"prompt_tokens_details": {"cached_tokens": null}}),
        ] {
            let mut metrics = Metrics::default();
            metrics.observe("test", Some(usage(value)), MetricObservation::Response, 1);
            let totals = &metrics.reporting.totals;
            assert_eq!(totals.usage_reported_calls, 1);
            for field in [&totals.input, &totals.output, &totals.cache] {
                assert_eq!(field.known_total, None);
                assert_eq!(field.reported_calls, 0);
                assert_eq!(field.unknown_calls, 1);
            }
        }
    }

    #[test]
    fn reporting_does_not_backfill_legacy_counters() {
        let mut metrics = Metrics {
            calls: 8,
            input_tokens: 400,
            ..Default::default()
        };
        metrics.observe("test", None, MetricObservation::Response, 1);
        assert_eq!(metrics.calls, 8);
        assert_eq!(metrics.input_tokens, 400);
        assert_eq!(metrics.reporting.totals.counted_successful_calls, 1);
        assert_eq!(metrics.reporting.totals.input.known_total, None);
        assert_eq!(metrics.reporting.totals.input.unknown_calls, 1);
    }
}
