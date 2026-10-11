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

// token counters as reported by the provider; accumulated for cost visibility
/// One request's (or, after `add`, several requests' summed) token counts. Holds
/// `prompt_tokens`, `completion_tokens`, `prompt_tokens_details` (see `CacheDetail`).
/// Field names match the OpenAI-compatible `usage` JSON object exactly (serde). Appears as
/// `Response.usage`/`StreamResponse.usage`, and accumulated into `Metrics.categories`.
#[derive(Deserialize, Default, Clone, Copy, Debug)]
struct Usage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    #[serde(default)]
    prompt_tokens_details: CacheDetail,
}

// the field OpenAI-compatible providers use to report a prompt-cache hit; 0 if absent
/// Holds `cached_tokens` — the portion of `prompt_tokens` served from the provider's prompt
/// cache. Nested inside `Usage.prompt_tokens_details`.
#[derive(Deserialize, Default, Clone, Copy, Debug)]
struct CacheDetail {
    #[serde(default)]
    cached_tokens: u64,
}

impl Usage {
    /// Adds another `Usage`'s counters into `self` (field-by-field sum).
    fn add(&mut self, other: Usage) {
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
        self.prompt_tokens_details.cached_tokens += other.prompt_tokens_details.cached_tokens;
    }
}

// model usage accumulated for the whole session; shown by !durum.
// categories give a breakdown by call type (sohbet/isteklilik/profilci/...).
/// Session-wide token accounting. Holds `calls` (count), `input_tokens`/`output_tokens`,
/// `cache_tokens` (the cached share of `input_tokens`), `last_call_secs` (unix time of the
/// most recent call), and `categories` (per-call-site `Usage` breakdown, keyed by the same
/// tags as `JSON_CATEGORIES` and more — e.g. `"sohbet"`, `"profilci"`). Lives at
/// `State.metrics`; read by `modal::status_message`/`summary_modal`/`token_breakdown`.
#[derive(Default, Clone, Debug)]
struct Metrics {
    calls: u32,
    input_tokens: u64,
    cache_tokens: u64, // the portion of input_tokens served from cache (when the provider reports it)
    output_tokens: u64,
    last_call_secs: i64,
    categories: HashMap<&'static str, Usage>,
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

// parses a single "data: ..." SSE line; None for keepalives/malformed lines
