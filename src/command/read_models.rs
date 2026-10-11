use super::{ActionError, ThinkingSetting};
use crate::platform::types::UserKey;
use crate::{growth, memory, sleep, travel, Bot, Metrics, State};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct UsageView {
    pub tag: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}
#[derive(Debug, Clone, Serialize)]
pub struct MetricsView {
    pub calls: u32,
    pub input_tokens: u64,
    pub cache_tokens: u64,
    pub output_tokens: u64,
    pub categories: Vec<UsageView>,
}
#[derive(Debug, Clone, Serialize)]
pub struct StatusView {
    pub version: String,
    pub stage: String,
    pub day: i64,
    pub chats: u32,
    pub messages: u32,
    pub model: String,
    pub awake: bool,
    pub forced_awake: bool,
    pub thinking: ThinkingSetting,
    pub thinking_label: String,
    pub debug: bool,
    pub travel: String,
    pub metrics: MetricsView,
    pub myself: String,
    pub agenda: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct SettingsView {
    pub status: StatusView,
}
#[derive(Debug, Clone, Serialize)]
pub struct HelpView {}
#[derive(Debug, Clone, Serialize)]
pub struct PersonView {
    pub id: UserKey,
    pub name: String,
    pub username: String,
    pub previous_names: Vec<String>,
    pub score: i32,
    pub tags: Vec<String>,
    pub note: String,
    pub facts: Vec<String>,
    pub events: Vec<String>,
}
impl From<memory::Person> for PersonView {
    fn from(person: memory::Person) -> Self {
        Self {
            id: person.id,
            name: person.name,
            username: person.username,
            previous_names: person.previous_names,
            score: person.score,
            tags: person.tags,
            note: person.note,
            facts: person.facts,
            events: person.events,
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct MindView {
    pub status: StatusView,
    pub bot_name: String,
    pub date: String,
    pub people: Vec<PersonView>,
    pub topics: Vec<(String, String)>,
    pub events: Vec<(String, Vec<String>)>,
}
pub(crate) fn metrics_from_metrics(metrics: &Metrics) -> MetricsView {
    let mut categories: Vec<_> = metrics
        .categories
        .iter()
        .map(|(tag, usage)| UsageView {
            tag: (*tag).to_string(),
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
        })
        .collect();
    categories
        .sort_by_key(|usage| std::cmp::Reverse(usage.prompt_tokens + usage.completion_tokens));
    MetricsView {
        calls: metrics.calls,
        input_tokens: metrics.input_tokens,
        cache_tokens: metrics.cache_tokens,
        output_tokens: metrics.output_tokens,
        categories,
    }
}
pub(crate) fn status_from_state(state: &State) -> StatusView {
    StatusView {
        version: crate::version_text(),
        stage: growth::stage(&state.growth).name.to_string(),
        day: growth::days(&state.growth) + 1,
        chats: state.growth.chats,
        messages: state.growth.messages,
        model: state.model.clone(),
        awake: sleep::is_awake(state),
        forced_awake: state.forced_awake_until > crate::now_unix(),
        thinking: state.thinking_mode.into(),
        thinking_label: state.thinking_mode.label().to_string(),
        debug: state.debug,
        travel: travel::now()
            .map(|trip| trip.place.to_string())
            .unwrap_or_else(|| crate::strings::t("common.no_travel").to_string()),
        metrics: metrics_from_metrics(&state.metrics),
        myself: state.myself.clone(),
        agenda: state.agenda.clone(),
    }
}
pub(crate) fn settings_from_state(state: &State) -> SettingsView {
    SettingsView {
        status: status_from_state(state),
    }
}
fn mind_with_snapshot(status: StatusView, bot_name: String) -> Result<MindView, ActionError> {
    Ok(MindView {
        status,
        bot_name,
        date: memory::date(),
        people: memory::person_summaries_result()
            .map_err(ActionError::Memory)?
            .into_iter()
            .map(PersonView::from)
            .collect(),
        topics: memory::topic_summaries_result().map_err(ActionError::Memory)?,
        events: memory::event_months_result(3).map_err(ActionError::Memory)?,
    })
}
#[cfg(test)]
pub(crate) fn mind_from_state(state: &State) -> Result<MindView, ActionError> {
    mind_with_snapshot(status_from_state(state), state.bot_name.clone())
}
pub(crate) fn status(bot: &Bot) -> StatusView {
    status_from_state(&bot.state())
}
pub(crate) fn settings(bot: &Bot) -> SettingsView {
    settings_from_state(&bot.state())
}
pub fn help() -> HelpView {
    HelpView {}
}
pub(crate) fn mind(bot: &Bot) -> Result<MindView, ActionError> {
    let (status, name) = {
        let state = bot.state();
        (status_from_state(&state), state.bot_name.clone())
    };
    mind_with_snapshot(status, name)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshots_are_owned_and_preserve_counters_and_modes() {
        let mut state = State {
            model: "fixture".into(),
            thinking_mode: crate::ThinkingMode::Off,
            myself: "private".into(),
            ..Default::default()
        };
        state.growth.chats = 4;
        state.growth.messages = 9;
        state.metrics.calls = 3;
        state.metrics.input_tokens = 10;
        state.metrics.cache_tokens = 2;
        let snapshot = status_from_state(&state);
        state.model.clear();
        state.myself.clear();
        assert_eq!(snapshot.model, "fixture");
        assert_eq!(snapshot.myself, "private");
        assert_eq!(snapshot.chats, 4);
        assert_eq!(snapshot.messages, 9);
        assert_eq!(snapshot.thinking, ThinkingSetting::Off);
        assert_eq!(snapshot.metrics.calls, 3);
        assert_eq!(snapshot.metrics.cache_tokens, 2);
    }
    #[test]
    fn person_views_retain_qualified_identity_and_private_details() {
        let key = UserKey::discord(42);
        let person =
            memory::Person::parse(&key, "# Someone\npuan: -2\netiket: one, two\nnot: note\n");
        let view = PersonView::from(person);
        assert_eq!(view.id, key);
        assert_eq!(view.score, -2);
        assert_eq!(view.tags, ["one", "two"]);
        assert_eq!(view.note, "note");
    }
}
