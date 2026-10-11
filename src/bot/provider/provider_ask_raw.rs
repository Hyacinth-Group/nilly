impl Bot {
    /// The single non-streaming HTTP call site — every non-streaming request (chat fallback,
    /// all agent calls) goes through here.
    /// `category` tags metrics and selects the JSON-in-thought fallback.
    /// Retries up to `AI_RETRIES` times on network errors, retryable statuses,
    /// reasoning-mandatory 400s, or empty replies after reasoning consumed the budget.
    /// Otherwise returns the last error.
    async fn ask_raw(
        &self,
        mut body: serde_json::Value,
        category: &'static str,
    ) -> Result<String, BotError> {
        let model = body
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or_default()
            .to_string();
        // don't even attempt it on a model already known to refuse turning reasoning off —
        // open with low effort right away instead of eating the same 400 every call
        let mut disabled = if self.reasoning_mandatory_known(&model) {
            self.reasoning_low_effort(&mut body);
            Self::grow_budget(&mut body, REASONING_BUDGET_BASE);
            false
        } else {
            self.disable_reasoning(&mut body, true)
        };
        let mut last_error: BotError = "request never went out".into();
        for attempt in 0..=AI_RETRIES {
            if attempt > 0 {
                sleep(Duration::from_secs(u64::from(attempt) * 2)).await;
                log::warn!("ai [ask_raw]: {last_error} — attempt {}", attempt + 1);
            }
            let resp = match self
                .http
                .post(&self.api_url)
                .bearer_auth(&self.key)
                .json(&body)
                .send()
                .await
            {
                Ok(c) => c,
                Err(e) if e.is_connect() || e.is_timeout() || e.is_request() => {
                    last_error = e.into();
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                // a 404 usually means "no model by this name"; show the body message and the model
                let model = body.get("model").and_then(|m| m.as_str()).unwrap_or("?");
                let error: BotError =
                    format!("{status} (model: {model}): {}", trim_error(&body_text)).into();
                if attempt < AI_RETRIES && disabled && reasoning_mandatory_error(&body_text) {
                    log::warn!(
                        "ai [ask_raw] [{category}]: model won't allow reasoning to be disabled, retrying with it on at low effort"
                    );
                    self.mark_reasoning_mandatory(model);
                    Self::remove_reasoning_fields(&mut body);
                    self.reasoning_low_effort(&mut body);
                    disabled = false;
                    if let Some(new_value) = Self::grow_budget(&mut body, REASONING_BUDGET_BASE) {
                        log::warn!(
                            "ai [ask_raw] [{category}]: the thought could eat the whole budget, raised max_tokens to {new_value}"
                        );
                    }
                    last_error = error;
                    continue;
                }
                if attempt < AI_RETRIES && status_retryable(status) {
                    last_error = error;
                    continue;
                }
                return Err(error);
            }
            let response: Response = serde_json::from_str(&body_text)?;
            self.add_metric(category, response.usage, MetricObservation::Response);
            let first = response.choices.into_iter().next();
            let thought_chars = first.as_ref().map_or(0, |s| thought_length(&s.message));
            let content_empty = first.as_ref().is_none_or(|s| {
                s.message
                    .content
                    .as_deref()
                    .is_none_or(|c| c.trim().is_empty())
            });
            let text = first
                .as_ref()
                .and_then(|s| response_content(&s.message, category));
            match text {
                Some(text) => {
                    if content_empty {
                        log::warn!(
                            "ai [ask_raw] [{category}]: content was empty, took it from the JSON in the thought field ({thought_chars} chars of thought)"
                        );
                    }
                    return Ok(text);
                }
                // reasoning that couldn't be disabled may have eaten the whole budget and
                // left content: null; grow the budget (if possible), retry once more at
                // low effort, and give up if that's still not enough
                None if attempt < AI_RETRIES => {
                    self.reasoning_low_effort(&mut body);
                    let grown = Self::grow_budget(&mut body, REASONING_BUDGET_BASE);
                    log::warn!(
                        "ai [ask_raw] [{category}]: empty response from the model ({thought_chars} chars of thought){}",
                        match grown {
                            Some(y) => format!(", raised max_tokens to {y} and retrying"),
                            None => String::new(),
                        }
                    );
                    last_error = "empty response from the model".into();
                }
                None => {
                    let model = body.get("model").and_then(|m| m.as_str()).unwrap_or("?");
                    let budget = body
                        .get("max_tokens")
                        .and_then(serde_json::Value::as_u64)
                        .map_or("unbudgeted".to_string(), |b| b.to_string());
                    return Err(format!(
                        "empty response from the model [{category}] (model: {model}, max_tokens: {budget}, thought: {thought_chars} chars)"
                    )
                    .into());
                }
            }
        }
        Err(last_error)
    }

}
