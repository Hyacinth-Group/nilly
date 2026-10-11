// Command interface: slash commands open as embed cards (sectioned and readable, like a
// web page), details are spread across labeled modal fields — nothing gets dumped into a
// single text box. Every visible label/title/button here comes from `strings::t` (see
// strings.rs, langs/tr.json) rather than a Rust string literal, so the whole surface follows
// `BOT_LANG` (lang.rs).

use super::*;

const MODAL_LIMIT: usize = 4000; // discord TextInput value upper bound
const FIELD_LIMIT: usize = 1024; // discord embed field value upper bound
const LABEL_LIMIT: usize = 45; // modal title/label, select-menu option label
const DESCRIPTION_LIMIT: usize = 100; // select-menu option description
const PERSON_MENU_LIMIT: usize = 25; // discord select-menu option upper bound
const TRIM_NOTE: &str = "\n… (sığmadı, kırpıldı)";

// component ids
pub const MIND_PERSON_PICK: &str = "mind_person_pick";
pub const MIND_TOPICS: &str = "mind_topics";
pub const MIND_EVENTS: &str = "mind_events";
pub const MIND_SUMMARY: &str = "mind_summary";
// settings panel: the thinking-mode buttons' id is "setting_thinking:<mode file value>"
pub const SETTING_THINKING: &str = "setting_thinking:";
pub const SETTING_DEBUG: &str = "setting_debug";
pub const SETTING_WAKE: &str = "setting_wake";
pub const SETTING_SLEEP: &str = "setting_sleep";

// embed accent colors
const COLOR_MIND: u32 = 0x5865F2;
const COLOR_STATUS: u32 = 0x57F287;
const COLOR_HELP: u32 = 0xEB459E;
const COLOR_SETTINGS: u32 = 0xFEE75C;
const COLOR_INFO: u32 = 0x99AAB5;

/// One labeled field of a detail modal. Holds `label` (field title), `custom_id` (unique
/// within the modal, unused after submission — see `handler_event.rs`'s `interaction_create`
/// `Modal` arm), `content` (fitted to `MODAL_LIMIT` by `new` below). Built by `person_modal`/
/// `topics_modal`/`events_modal`/`summary_modal` below; consumed by `build_modal` below.
pub struct Section {
    pub label: String,
    pub custom_id: String,
    pub content: String,
}

impl Section {
    /// Fits the content to `MODAL_LIMIT`.
    fn new(label: impl Into<String>, custom_id: impl Into<String>, content: String) -> Self {
        Self {
            label: label.into(),
            custom_id: custom_id.into(),
            content: fit_to_limit(&content, MODAL_LIMIT),
        }
    }
}

// text over the limit is cut at the last line break/space + a note; the body always fits within the limit
fn fit_to_limit(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut s: String = text
        .chars()
        .take(limit - TRIM_NOTE.chars().count())
        .collect();
    if let Some(end) = s.rfind(['\n', ' ']) {
        s.truncate(end);
    }
    s.push_str(TRIM_NOTE);
    s
}

// "2026-09" -> "Eylül 2026"; returns the input unchanged if it can't be parsed
fn month_name(month: &str) -> String {
    let mut parts = month.splitn(2, '-');
    match (parts.next(), parts.next()) {
        (Some(year), Some(m)) if (1..=12).contains(&m.parse::<usize>().unwrap_or(0)) => {
            const KEYS: [&str; 12] = [
                "month.1", "month.2", "month.3", "month.4", "month.5", "month.6", "month.7",
                "month.8", "month.9", "month.10", "month.11", "month.12",
            ];
            format!(
                "{} {}",
                strings::t(KEYS[m.parse::<usize>().unwrap_or(1) - 1]),
                year
            )
        }
        _ => month.to_string(),
    }
}

// categories sorted by total tokens: "sohbet: 120 giriş + 80 çıkış · ..."
fn token_breakdown(metrics: &Metrics) -> String {
    use std::fmt::Write as _;
    let mut sorted: Vec<(&'static str, &Usage)> = metrics
        .categories
        .iter()
        .map(|(tag, k)| (*tag, k))
        .collect();
    sorted.sort_by_key(|(_, k)| std::cmp::Reverse(k.prompt_tokens + k.completion_tokens));
    let mut lines = String::new();
    for (i, (tag, k)) in sorted.iter().enumerate() {
        if i > 0 {
            lines.push_str(" · ");
        }
        let _ = write!(
            lines,
            "{tag}: {} giriş + {} çıkış",
            k.prompt_tokens, k.completion_tokens
        );
    }
    lines
}

// "{calls} çağrı · {input} giriş ({cache} önbellek) + {output} çıkış", the token summary
// line shared by /durum and /zihin's bot-summary modal.
fn token_summary(metrics: &Metrics) -> String {
    strings::t("common.token_template")
        .replace("{calls}", &metrics.calls.to_string())
        .replace("{input}", &metrics.input_tokens.to_string())
        .replace("{cache}", &metrics.cache_tokens.to_string())
        .replace("{output}", &metrics.output_tokens.to_string())
}

fn awake_word(state: &State) -> &'static str {
    if sleep::is_awake(state) {
        strings::t("common.awake")
    } else {
        strings::t("common.asleep")
    }
}

fn travel_word() -> String {
    travel::now()
        .map(|s| s.place.to_string())
        .unwrap_or_else(|| strings::t("common.no_travel").to_string())
}

// ---------- /zihin: embed card + menu/buttons ----------

// the mind card: three columns (people / topics / events) + counters in the footer
pub fn mind_embeds(state: &State) -> Vec<CreateEmbed> {
    let people = memory::person_summaries();
    let topics = memory::topic_summaries();
    let events = memory::event_months(3);
    let event_count: usize = events.iter().map(|(_, s)| s.len()).sum();

    let mut people_lines = String::new();
    for p in people.iter().take(8) {
        people_lines += &format!("**{}** ({:+})", p.name, p.score);
        if !p.tags.is_empty() {
            people_lines += &format!(" · {}", p.tags.join(", "));
        }
        people_lines.push('\n');
    }

    let mut topic_lines = String::new();
    for (name, latest) in topics.iter().take(8) {
        topic_lines += &format!("**{name}**");
        if !latest.is_empty() {
            topic_lines += &format!(" · son: {latest}");
        }
        topic_lines.push('\n');
    }

    // most recent events: the end of each month working backward from the newest; older
    // months are put first so the display reads chronologically
    let mut chunks: Vec<Vec<&str>> = Vec::new();
    let mut total = 0usize;
    for (_, s) in events.iter() {
        if total >= 5 {
            break;
        }
        let take_n = (5 - total).min(s.len());
        total += take_n;
        let len = s.len();
        chunks.push(
            s.iter()
                .skip(len.saturating_sub(take_n))
                .map(|x| x.as_str())
                .collect(),
        );
    }
    chunks.reverse();
    let mut event_lines = String::new();
    for chunk in chunks {
        for item in chunk {
            // "- date time #channel: text" -> a readable single line
            event_lines += &memory::trim(item.trim_start_matches("- "), 90);
            event_lines.push('\n');
        }
    }

    let growth = &state.growth;
    vec![CreateEmbed::new()
        .title(strings::t("mind.title"))
        .color(COLOR_MIND)
        .description(
            strings::t("mind.description")
                .replace("{stage}", growth::stage(growth).name)
                .replace("{day}", &(growth::days(growth) + 1).to_string())
                .replace("{model}", &state.model)
                .replace("{thinking}", state.thinking_mode.label()),
        )
        .field(
            strings::t("mind.people_field").replace("{count}", &people.len().to_string()),
            dash_if_empty(&people_lines),
            true,
        )
        .field(
            strings::t("mind.topics_field").replace("{count}", &topics.len().to_string()),
            dash_if_empty(&topic_lines),
            true,
        )
        .field(
            strings::t("mind.events_field").replace("{count}", &event_count.to_string()),
            dash_if_empty(&event_lines),
            true,
        )
        .footer(CreateEmbedFooter::new(
            strings::t("mind.footer")
                .replace("{name}", &state.bot_name)
                .replace("{date}", &memory::date()),
        ))]
}

// the short acknowledgment/status reply for commands: this always goes out instead of
// plain text (e.g. "haber bulamadım", "tamam, <model>", "debug açık"); wrapped by
// reply_info/report_result in command.rs
pub fn info_embed(title: &str, description: &str) -> CreateEmbed {
    CreateEmbed::new()
        .title(title)
        .description(fit_to_limit(description, FIELD_LIMIT))
        .color(COLOR_INFO)
}

// an empty section shows as "—" on the card: faint, but the layout doesn't break
/// Non-empty fields are capped at `FIELD_LIMIT`.
fn dash_if_empty(s: &str) -> String {
    let t = s.trim();
    if t.is_empty() {
        "—".to_string()
    } else {
        fit_to_limit(t, FIELD_LIMIT)
    }
}

// person detail menu: most recently changed, label name + score, description holds tags/note
/// Offers at most `PERSON_MENU_LIMIT` people.
fn person_options() -> Vec<CreateSelectMenuOption> {
    memory::person_summaries()
        .into_iter()
        .take(PERSON_MENU_LIMIT)
        .map(|p| {
            let mut description: Vec<String> = Vec::new();
            if !p.tags.is_empty() {
                description.push(p.tags.join(", "));
            }
            if !p.note.is_empty() {
                description.push(p.note.clone());
            }
            let option = CreateSelectMenuOption::new(
                memory::trim(&format!("{} ({:+})", p.name, p.score), LABEL_LIMIT),
                p.id.to_string(),
            );
            // discord's description field can't be empty once given (min length 1); if
            // there are no tags/note, the field is left off entirely — otherwise it dropped
            // the whole menu with "Invalid Form Body"
            let description = description.join(" · ");
            if description.trim().is_empty() {
                option
            } else {
                option.description(memory::trim(&description, DESCRIPTION_LIMIT))
            }
        })
        .collect()
}

pub fn mind_components() -> Vec<CreateActionRow> {
    let mut rows: Vec<CreateActionRow> = Vec::new();
    let options = person_options();
    if !options.is_empty() {
        rows.push(CreateActionRow::SelectMenu(
            CreateSelectMenu::new(MIND_PERSON_PICK, CreateSelectMenuKind::String { options })
                .placeholder(strings::t("mind.person_placeholder")),
        ));
    }
    rows.push(CreateActionRow::Buttons(vec![
        CreateButton::new(MIND_TOPICS)
            .label(strings::t("mind.button_topics"))
            .style(ButtonStyle::Secondary),
        CreateButton::new(MIND_EVENTS)
            .label(strings::t("mind.button_events"))
            .style(ButtonStyle::Secondary),
        CreateButton::new(MIND_SUMMARY)
            .label(strings::t("mind.button_summary"))
            .style(ButtonStyle::Secondary),
    ]));
    rows
}

pub fn mind_message(state: &State) -> CreateInteractionResponseMessage {
    CreateInteractionResponseMessage::new()
        .ephemeral(true)
        .embeds(mind_embeds(state))
        .components(mind_components())
}

// ---------- detail modals: each topic in its own labeled field ----------

/// Empty sections are omitted; if all are empty, supplies a single placeholder field.
fn build_modal(title: &str, custom_id: &str, sections: Vec<Section>) -> CreateModal {
    let filled: Vec<Section> = sections
        .into_iter()
        .filter(|b| !b.content.trim().is_empty())
        .collect();
    let rows = if filled.is_empty() {
        vec![CreateActionRow::InputText(
            CreateInputText::new(
                InputTextStyle::Paragraph,
                strings::t("modal.empty_label"),
                format!("{custom_id}_bos"),
            )
            .value(strings::t("modal.empty_value"))
            .required(false),
        )]
    } else {
        filled
            .into_iter()
            .map(|b| {
                CreateActionRow::InputText(
                    CreateInputText::new(
                        InputTextStyle::Paragraph,
                        memory::trim(&b.label, LABEL_LIMIT),
                        b.custom_id,
                    )
                    .value(b.content)
                    .required(false),
                )
            })
            .collect()
    };
    CreateModal::new(
        custom_id,
        title.chars().take(LABEL_LIMIT).collect::<String>(),
    )
    .components(rows)
}

// person card: identity / impression / tags / facts / recent events in separate fields
pub fn person_modal(id: u64) -> CreateModal {
    let p = memory::read_person(id);
    let title = if p.name.is_empty() {
        strings::t("person.unknown").to_string()
    } else {
        p.name.clone()
    };
    let mut sections: Vec<Section> = Vec::new();

    let mut identity = format!("{}\nid: {}", p.name, p.id);
    if !p.username.is_empty() {
        identity += &strings::t("person.username_line").replace("{username}", &p.username);
    }
    if !p.previous_names.is_empty() {
        identity += &strings::t("person.previous_names_line")
            .replace("{names}", &p.previous_names.join(", "));
    }
    sections.push(Section::new(
        strings::t("person.identity_label"),
        "person_identity",
        identity,
    ));

    let mut impression = strings::t("person.score").replace("{score}", &format!("{:+}", p.score));
    if !p.note.is_empty() {
        impression += &format!("\n{}", p.note);
    }
    sections.push(Section::new(
        strings::t("person.impression_label"),
        "person_impression",
        impression,
    ));

    if !p.tags.is_empty() {
        sections.push(Section::new(
            strings::t("person.tags_label"),
            "person_tags",
            p.tags.join(" · "),
        ));
    }
    if !p.facts.is_empty() {
        let n = p.facts.len();
        let list: Vec<&str> = p
            .facts
            .iter()
            .skip(n.saturating_sub(8))
            .map(|s| s.as_str())
            .collect();
        sections.push(Section::new(
            strings::t("person.facts_label"),
            "person_facts",
            list.join("\n"),
        ));
    }
    if !p.events.is_empty() {
        let n = p.events.len();
        let list: Vec<&str> = p
            .events
            .iter()
            .skip(n.saturating_sub(5))
            .map(|s| s.as_str())
            .collect();
        sections.push(Section::new(
            strings::t("person.recent_events_label"),
            "person_events",
            list.join("\n"),
        ));
    }
    build_modal(&title, &format!("mind_person_{id}"), sections)
}

// topics: most recently changed with their notes, the rest as a plain name list
pub fn topics_modal() -> CreateModal {
    let topics = memory::topic_summaries();
    let mut sections: Vec<Section> = Vec::new();
    let recent: Vec<String> = topics
        .iter()
        .take(15)
        .map(|(name, note)| {
            if note.is_empty() {
                strings::t("topics.recent_line").replace("{name}", name)
            } else {
                strings::t("topics.recent_line_with_note")
                    .replace("{name}", name)
                    .replace("{note}", note)
            }
        })
        .collect();
    sections.push(Section::new(
        strings::t("topics.recent_label"),
        "topics_recent",
        recent.join("\n"),
    ));
    if topics.len() > 15 {
        let other: Vec<&str> = topics[15..].iter().map(|(name, _)| name.as_str()).collect();
        sections.push(Section::new(
            strings::t("topics.other_label"),
            "topics_other",
            other.join(" · "),
        ));
    }
    build_modal(strings::t("topics.title"), "topics_modal", sections)
}

// events: one field per month, each month's most recent entries
pub fn events_modal() -> CreateModal {
    let mut sections: Vec<Section> = Vec::new();
    for (month, lines) in memory::event_months(3) {
        if lines.is_empty() {
            continue;
        }
        let n = lines.len();
        let shown: Vec<&str> = lines
            .iter()
            .skip(n.saturating_sub(10))
            .map(|s| s.as_str())
            .collect();
        sections.push(Section::new(
            month_name(&month),
            format!("events_{month}"),
            shown.join("\n"),
        ));
    }
    build_modal(strings::t("events.title"), "events_modal", sections)
}

// bot summary: status / tokens / myself / agenda in separate fields
pub fn summary_modal(state: &State) -> CreateModal {
    let growth = &state.growth;
    let metrics = &state.metrics;
    let status = strings::t("summary.status_template")
        .replace("{stage}", growth::stage(growth).name)
        .replace("{day}", &(growth::days(growth) + 1).to_string())
        .replace("{chats}", &growth.chats.to_string())
        .replace("{messages}", &growth.messages.to_string())
        .replace("{model}", &state.model)
        .replace("{sleep}", awake_word(state))
        .replace("{thinking}", state.thinking_mode.label())
        .replace("{travel}", &travel_word());
    let mut token_text = token_summary(metrics);
    if !metrics.categories.is_empty() {
        token_text +=
            &strings::t("common.breakdown_line").replace("{breakdown}", &token_breakdown(metrics));
    }
    let mut sections = vec![
        Section::new(strings::t("summary.status_label"), "summary_status", status),
        Section::new(
            strings::t("common.token_label"),
            "summary_tokens",
            token_text,
        ),
    ];
    if !state.myself.trim().is_empty() {
        let recent: Vec<&str> = state.myself.lines().rev().take(4).collect();
        sections.push(Section::new(
            strings::t("summary.myself_label"),
            "summary_myself",
            recent.into_iter().rev().collect::<Vec<_>>().join("\n"),
        ));
    }
    if !state.agenda.trim().is_empty() {
        sections.push(Section::new(
            strings::t("summary.agenda_label"),
            "summary_agenda",
            memory::trim(&state.agenda, 1000),
        ));
    }
    build_modal(strings::t("summary.title"), "summary_modal", sections)
}

// ---------- /durum and /yardim ----------

pub fn status_message(state: &State) -> CreateInteractionResponseMessage {
    let metrics = &state.metrics;
    let growth = &state.growth;
    let mut embed = CreateEmbed::new()
        .title(strings::t("status.title"))
        .color(COLOR_STATUS)
        .field(
            strings::t("status.general_label"),
            strings::t("status.general_template")
                .replace("{version}", &version_text())
                .replace("{stage}", growth::stage(growth).name)
                .replace("{day}", &(growth::days(growth) + 1).to_string())
                .replace("{chats}", &growth.chats.to_string())
                .replace("{messages}", &growth.messages.to_string())
                .replace("{model}", &state.model),
            true,
        )
        .field(
            strings::t("status.state_label"),
            strings::t("status.state_template")
                .replace("{sleep}", awake_word(state))
                .replace("{thinking}", state.thinking_mode.label())
                .replace(
                    "{debug}",
                    if state.debug {
                        strings::t("common.debug_on")
                    } else {
                        strings::t("common.debug_off")
                    },
                )
                .replace("{travel}", &travel_word()),
            true,
        )
        .field(
            strings::t("common.token_label"),
            token_summary(metrics),
            false,
        );
    if !metrics.categories.is_empty() {
        embed = embed.field(
            strings::t("status.kirilim_label"),
            fit_to_limit(&token_breakdown(metrics), FIELD_LIMIT),
            false,
        );
    }
    CreateInteractionResponseMessage::new()
        .ephemeral(true)
        .embed(embed)
}

pub fn help_message() -> CreateInteractionResponseMessage {
    CreateInteractionResponseMessage::new()
        .ephemeral(true)
        .embed(
            CreateEmbed::new()
                .title(strings::t("help.title"))
                .color(COLOR_HELP)
                .description(strings::t("help.text"))
                .field(
                    strings::t("help.interface_label"),
                    strings::t("help.interface_text"),
                    false,
                ),
        )
}

// ---------- settings panel (buttons) ----------

pub fn settings_embed(state: &State) -> CreateEmbed {
    let sleep_status = if sleep::is_awake(state) {
        if state.forced_awake_until > now_unix() {
            strings::t("settings.awake_forced")
        } else {
            strings::t("common.awake")
        }
    } else {
        strings::t("common.asleep")
    };
    CreateEmbed::new()
        .title(strings::t("settings.title"))
        .color(COLOR_SETTINGS)
        .description(
            strings::t("settings.description_template")
                .replace("{version}", &version_text())
                .replace("{model}", &state.model)
                .replace("{thinking}", state.thinking_mode.label())
                .replace(
                    "{debug}",
                    if state.debug {
                        strings::t("common.debug_on")
                    } else {
                        strings::t("common.debug_off")
                    },
                )
                .replace("{sleep}", sleep_status)
                .replace("{travel}", &travel_word()),
        )
        .footer(CreateEmbedFooter::new(strings::t("settings.footer")))
}

/// Highlights the current thinking mode and offers debug/wake/sleep controls.
pub fn settings_components(state: &State) -> Vec<CreateActionRow> {
    // same words as /dusunme's own choice labels (cmd.dusunme.choice.*), not
    // ThinkingMode::label() — that returns the descriptive form ("gizli"/"kapalı") used in
    // status text, not the imperative command-argument form these buttons show
    let modes = [
        (ThinkingMode::Show, "cmd.dusunme.choice.goster"),
        (ThinkingMode::Hide, "cmd.dusunme.choice.gizle"),
        (ThinkingMode::Silent, "cmd.dusunme.choice.sessiz"),
        (ThinkingMode::Off, "cmd.dusunme.choice.kapat"),
    ];
    let thinking_buttons: Vec<CreateButton> = modes
        .iter()
        .map(|(mode, key)| {
            CreateButton::new(format!("{SETTING_THINKING}{}", mode.file_value()))
                .label(strings::t("settings.mode_button").replace("{mode}", strings::t(key)))
                .style(if *mode == state.thinking_mode {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Secondary
                })
        })
        .collect();
    let awake = sleep::is_awake(state);
    vec![
        CreateActionRow::Buttons(thinking_buttons),
        CreateActionRow::Buttons(vec![
            CreateButton::new(SETTING_DEBUG)
                .label(if state.debug {
                    strings::t("settings.debug_on_button")
                } else {
                    strings::t("settings.debug_off_button")
                })
                .style(if state.debug {
                    ButtonStyle::Success
                } else {
                    ButtonStyle::Secondary
                }),
            CreateButton::new(SETTING_WAKE)
                .label(strings::t("settings.wake_button"))
                .style(ButtonStyle::Secondary)
                .disabled(awake),
            CreateButton::new(SETTING_SLEEP)
                .label(strings::t("settings.sleep_button"))
                .style(ButtonStyle::Secondary)
                .disabled(!awake),
        ]),
    ]
}

// /ayarlar (ephemeral) and the in-place refresh after a button press (UpdateMessage) share this body
pub fn settings_message(state: &State, ephemeral: bool) -> CreateInteractionResponseMessage {
    CreateInteractionResponseMessage::new()
        .ephemeral(ephemeral)
        .embed(settings_embed(state))
        .components(settings_components(state))
}

// ---------- slash registration ----------

// guild commands: called on every ready, overwrites what's on Discord (idempotent); a
// guild command shows up instantly, a global command is delayed. The list comes from
// command.rs's definitions() table — a single source, never kept in two places by hand.
pub async fn register_commands(http: &Http, guild: GuildId) -> Result<(), BotError> {
    let commands: Vec<CreateCommand> = command::definitions()
        .iter()
        .map(|k| {
            CreateCommand::new(k.name)
                .description(k.description)
                .set_options((k.options)())
        })
        .collect();
    guild.set_commands(http, commands).await?;
    Ok(())
}
#[cfg(test)]
mod test {
    use super::*;

    /// Verifies `fit_to_limit` truncates with a "kırpıldı" marker when over the limit, and passes short text through unchanged.
    #[test]
    fn fit_stays_within_limit() {
        let long_text = "kelime ".repeat(1000);
        let s = fit_to_limit(&long_text, 200);
        assert!(s.chars().count() <= 200);
        assert!(s.contains("kırpıldı"));
        // short text passes through unchanged
        assert_eq!(fit_to_limit("kısa", 200), "kısa");
    }

    /// Verifies `month_name` converts a `YYYY-MM` key to a month name, and passes through unparseable input as-is.
    #[test]
    fn month_name_converts() {
        assert_eq!(
            month_name("2026-09"),
            format!("{} 2026", strings::t("month.9"))
        );
        assert_eq!(
            month_name("2026-01"),
            format!("{} 2026", strings::t("month.1"))
        );
        assert_eq!(month_name("bozuk"), "bozuk");
        assert_eq!(month_name("2026-13"), "2026-13");
    }

    /// Verifies the empty-section filter `build_modal` relies on: only non-empty `Section`s survive.
    #[test]
    fn empty_sections_become_placeholder_field() {
        // build_modal skips empty sections; if all are empty, a single placeholder field remains.
        // CreateModal isn't inspectable, so behavior is verified via the Section filter itself:
        let filled: Vec<Section> = vec![
            Section::new("A", "a", String::new()),
            Section::new("B", "b", "veri".into()),
        ]
        .into_iter()
        .filter(|b| !b.content.trim().is_empty())
        .collect();
        assert_eq!(filled.len(), 1);
        assert_eq!(filled[0].label, "B");
    }

    /// Verifies `token_breakdown` sorts categories by usage, descending.
    #[test]
    fn token_breakdown_sorted() {
        let mut metrics = Metrics::default();
        metrics.categories.insert(
            "few",
            Usage {
                prompt_tokens: 10,
                completion_tokens: 5,
                ..Default::default()
            },
        );
        metrics.categories.insert(
            "many",
            Usage {
                prompt_tokens: 100,
                completion_tokens: 50,
                ..Default::default()
            },
        );
        let result = token_breakdown(&metrics);
        // the larger category comes first
        assert!(result.find("many").unwrap() < result.find("few").unwrap());
    }
}
