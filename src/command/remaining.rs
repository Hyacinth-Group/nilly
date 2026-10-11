impl Bot {
    // !uyan and the settings panel: doesn't delete the plan (deleting it would just let
    // it get rebuilt a minute later and put the bot back to sleep) — instead stays
    // "forced awake" until the planned sleep window would have ended
    /// Without an active sleep window, the override lasts six hours.
    pub fn wake(&self) {
        let mut state = self.state();
        let now = now_unix();
        let end = state
            .plans
            .iter()
            .filter(|plan| plan.start <= now && now < plan.end)
            .map(|plan| plan.end)
            .max()
            .unwrap_or(now + 6 * 3600);
        state.forced_awake_until = end;
    }

    // !uyu [saat] and the settings panel: a temporary sleep plan for testing.
    // `hours` is how long from now the plan should last.
    /// Clears any forced-awake override before adding the new plan.
    pub fn put_to_sleep(&self, hours: i64) {
        let mut state = self.state();
        let now = now_unix();
        state.forced_awake_until = 0;
        state.plans.push(sleep::Plan {
            day: -1,
            insomnia_start: None,
            start: now,
            end: now + hours * 3600,
        });
    }


    fn model_catalogue_url(&self) -> Option<&'static str> {
        match self.api_url.trim_end_matches('/') {
            crate::OPENROUTER_URL => Some("https://openrouter.ai/api/v1/models"),
            crate::MISTRAL_URL => Some("https://api.mistral.ai/v1/models"),
            _ => None,
        }
    }

    /// Checks only the official provider catalogue; failures never validate a model.
    pub(crate) async fn model_exists(&self, id: &str) -> Result<bool, ActionError> {
        #[derive(Deserialize)]
        struct ModelList {
            data: Vec<ModelEntry>,
        }
        #[derive(Deserialize)]
        struct ModelEntry {
            id: String,
        }
        let endpoint = self.model_catalogue_url()
            .ok_or(ActionError::ModelValidationUnsupported)?;
        let mut request = self.http.get(endpoint);
        if endpoint.starts_with("https://api.mistral.ai/") {
            request = request.bearer_auth(&self.key);
        }
        let response = request.send().await.map_err(|e| ActionError::Operation(e.into()))?
            .error_for_status().map_err(|e| ActionError::Operation(e.into()))?;
        let list = response.json::<ModelList>().await.map_err(|e| ActionError::Operation(e.into()))?;
        Ok(list.data.iter().any(|model| model.id == id))
    }
}
