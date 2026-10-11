impl Bot {
    /// Single-block-system convenience wrapper around `ask_split`.
    async fn ask(
        &self,
        system: &str,
        history: &[ChatMessage],
        max_tokens: u32,
        category: &'static str,
    ) -> Result<String, BotError> {
        self.ask_split(system, "", history, Some(max_tokens), category)
            .await
    }

    // the system message is two blocks: the fixed block is marked with cache_control on
    // providers that support it (anthropic/gemini); the variable block is re-read every time.
    // budget None means max_tokens isn't sent at all, and the model talks unbudgeted.
    async fn ask_split(
        &self,
        fixed: &str,
        variable: &str,
        history: &[ChatMessage],
        budget: Option<u32>,
        category: &'static str,
    ) -> Result<String, BotError> {
        let model = self.state().model.clone();
        let mut messages = vec![system_json(fixed, variable, &self.api_url)];
        messages.extend(history.iter().map(message_json));
        let mut body = serde_json::json!({
            "model": model,
            "messages": messages,
            "temperature": 0.7,
        });
        if let Some(t) = budget {
            body["max_tokens"] = serde_json::json!(t);
        }
        self.ask_raw(body, category).await
    }

    // streaming request: error handling matches ask_raw, "stream" is added to the body.
    // budget None means max_tokens isn't sent, and the model talks unbudgeted (the release chat path).
    /// The single streaming HTTP call site — opens the request and, on success, hands back a
    /// `StreamReader` positioned at the start of the stream (no chunks read yet).
    /// Retries only before streaming begins; once chunks arrive, `send_stream` owns
    /// failure handling. Retry conditions match `ask_raw`.
    async fn ask_raw_stream(
        &self,
        fixed: &str,
        variable: &str,
        history: &[ChatMessage],
        budget: Option<u32>,
        category: &'static str,
    ) -> Result<StreamReader, BotError> {
        let model = self.state().model.clone();
        let mut messages = vec![system_json(fixed, variable, &self.api_url)];
        messages.extend(history.iter().map(message_json));
        let mut body = serde_json::json!({
            "model": model,
            "messages": messages,
            "temperature": 0.7,
            "stream": true,
            // include usage on the final chunk (the token counter)
            "stream_options": { "include_usage": true },
        });
        if let Some(t) = budget {
            body["max_tokens"] = serde_json::json!(t);
        }
        let mut disabled = if self.reasoning_mandatory_known(&model) {
            Self::apply_budget_floor(&mut body, REASONING_MANDATORY_BASE);
            false
        } else {
            self.disable_reasoning(&mut body, false)
        };
        // the retry only happens before the stream opens: once a chunk starts arriving,
        // the reader has already been returned, and from there send_stream owns the
        // mid-stream-failure path
        let mut last_error: BotError = "request never went out".into();
        for attempt in 0..=AI_RETRIES {
            if attempt > 0 {
                sleep(Duration::from_secs(u64::from(attempt) * 2)).await;
                log::warn!("ai [ask_raw_stream]: {last_error} — attempt {}", attempt + 1);
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
            if !status.is_success() {
                let body_text = resp.text().await.unwrap_or_default();
                let model = body.get("model").and_then(|m| m.as_str()).unwrap_or("?");
                let error: BotError =
                    format!("{status} (model: {model}): {}", trim_error(&body_text)).into();
                if attempt < AI_RETRIES && disabled && reasoning_mandatory_error(&body_text) {
                    log::warn!(
                        "ai [ask_raw_stream]: model won't allow reasoning to be disabled, retrying with it on"
                    );
                    self.mark_reasoning_mandatory(model);
                    Self::remove_reasoning_fields(&mut body);
                    disabled = false;
                    if Self::apply_budget_floor(&mut body, REASONING_MANDATORY_BASE) {
                        log::warn!(
                            "ai [ask_raw_stream]: the small budget may not be enough for reasoning, raised to {REASONING_MANDATORY_BASE}"
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
            return Ok(StreamReader {
                response: resp,
                buffer: Vec::new(),
                queue: Vec::new(),
                usage: StreamUsage::default(),
                category,
                done: false,
                finished: false,
            });
        }
        Err(last_error)
    }

    // system message for a chat reply: looks at who's talking and about what, and pulls
    // only the relevant pieces from memory; shared by generate and generate_stream
}
