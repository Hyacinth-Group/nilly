impl Bot {
    // when thinking mode is off, turns off reasoning generation in the request (so it
    // doesn't spend tokens on it). Each provider has its own vocabulary: openrouter
    // understands "reasoning", qwen-style routers understand "enable_thinking", mistral
    // has no such switch at all (sending both fields at once broke some providers, so the
    // choice is made based on the URL). Returns true if a field was actually added (some
    // models still refuse to let reasoning be turned off — the caller detects that and
    // retries once more).
    // force=true: turns it off regardless of the user's mode — ask_raw (non-streaming)
    // never reads/shows the reasoning_content field at all, so background agents (profiler,
    // coach, diarist, wanderer, willingness, mood) shouldn't waste their small max_tokens
    // budget thinking even while the mode is "hide", and shouldn't end up with content: null
    // and an "empty response from the model" error. ask_raw_stream (streaming, chat) still
    // respects the user's mode: "hide" shows a thought counter, "show" shows the full text,
    // so it passes force=false.
    /// Returns whether a disabling field was added.
    fn disable_reasoning(&self, body: &mut serde_json::Value, force: bool) -> bool {
        if !force && self.state().thinking_mode != ThinkingMode::Off {
            return false;
        }
        let Some(o) = body.as_object_mut() else {
            return false;
        };
        let url = self.api_url.to_lowercase();
        if url.contains("openrouter") {
            o.insert("reasoning".into(), serde_json::json!({ "enabled": false }));
        } else if !url.contains("mistral") {
            o.insert("enable_thinking".into(), serde_json::json!(false));
        } else {
            return false; // mistral: no field was added, nothing to roll back
        }
        true
    }

    // rolls back the fields disable_reasoning added (used to retry after a mandatory-reasoning error)
    fn remove_reasoning_fields(body: &mut serde_json::Value) {
        if let Some(o) = body.as_object_mut() {
            o.remove("reasoning");
            o.remove("enable_thinking");
        }
    }

    // raises max_tokens if it's below the floor, so a reasoning-mandatory model isn't left
    // without enough budget. Leaves it alone if max_tokens isn't set at all (an unbudgeted
    // call). Returns true if it changed anything (for logging).
    fn apply_budget_floor(body: &mut serde_json::Value, floor: u32) -> bool {
        match body.get("max_tokens").and_then(serde_json::Value::as_u64) {
            Some(current) if (current as u32) < floor => {
                body["max_tokens"] = serde_json::json!(floor);
                true
            }
            _ => false,
        }
    }

    // grows the budget on a model that won't let reasoning be turned off: the thought can
    // eat most of the budget, and a 500 floor left a 1200-budget diarist call untouched.
    // Leaves it alone if max_tokens isn't set; returns the new value if it grew the budget.
    fn grow_budget(body: &mut serde_json::Value, floor: u32) -> Option<u32> {
        let current = body
            .get("max_tokens")
            .and_then(serde_json::Value::as_u64)? as u32;
        let new_value = current.saturating_mul(2).max(floor);
        if new_value <= current {
            return None;
        }
        body["max_tokens"] = serde_json::json!(new_value);
        Some(new_value)
    }

    // openrouter: if reasoning can't be turned off, at least make it think briefly (a
    // unified parameter that a model without support simply ignores). Skipped for every
    // other URL: an unknown field could break the request there.
    fn reasoning_low_effort(&self, body: &mut serde_json::Value) {
        if !self.api_url.to_lowercase().contains("openrouter") {
            return;
        }
        if let Some(o) = body.as_object_mut() {
            o.insert("reasoning".into(), serde_json::json!({ "effort": "low" }));
        }
    }

    // the raw request with a descriptive error; everything goes through here. Backs off
    // and retries on a network error / 429 / 5xx; some models (e.g. certain GLM reasoning
    // variants) refuse to let reasoning be disabled ("mandatory"/"cannot be disabled" 400)
    // — in that case the fields are removed and it retries with reasoning left on, so the
    // model doesn't fail on every single turn and jam the chat.
    // category only feeds the token metrics breakdown (!durum); it has no effect on the request.
}
