// The bot's background agents. None of them speak with personality; they all do plain
// analysis and write the result to durum/. The talking side (main.rs) only reads these results.
//
//   profiler    gets to know the group                -> profil.md
//   diarist     memory entry from a finished chat      -> kisiler/, konular/, olaylar/, kendim.md
//   coach       what kind of personality to have       -> huy.md
//   critic      what went wrong in the last chat        -> duzeltmeler.md
//   summarizer  shrinks a file that's over its limit    -> file shrinks, the overflow goes to arsiv/
//   news_agent  what to post from hacker news           -> the picked news item
//   image_commenter  what to say about the attached image -> a one-line comment
//   wanderer    (agenda.rs) browses the news, writes its take -> gundem.md

use super::*;
use crate::memory::{self, Person, RecordSnapshot};
use base64::Engine;
use serde::Deserialize;
use std::path::PathBuf;

// what the diarist wrote in one pass; for !zihin test and logging
/// Counts of what one `diarist` call produced. Holds `people`/`topics`/`events` (how many
/// of each were written) and `output_chars` (size of the model's raw JSON reply). Returned
/// by `Bot::diarist` below; read by `Bot::cmd_mind`'s `test:true` path (`command/cards.rs`).
#[derive(Default, Debug, Clone, Copy)]
pub struct DiaristSummary {
    pub people: usize,
    pub topics: usize,
    pub events: usize,
    pub output_chars: usize, // character count of the model's output
}

/// Deserialize target for the diarist's JSON reply (see `prompts/gunlukcu.md`). Field
/// names are Turkish on purpose — they must match the JSON keys the model is instructed to
/// produce (see AGENTS.md rule 8 / `docs/glossary.md`). Holds `olay` (one-line event summary),
/// `kisiler`/`konular` (per-person/per-topic records, see `PersonRecord`/`TopicRecord`
/// below), `kendim` (the bot's own updated state, if it changed). Parsed by `Bot::diarist`
/// below, the only consumer.
#[derive(Deserialize, Default)]
struct Record {
    #[serde(default)]
    olay: String,
    #[serde(default)]
    kisiler: Vec<PersonRecord>,
    #[serde(default)]
    konular: Vec<TopicRecord>,
    #[serde(default)]
    kendim: String,
}
/// One `Record.kisiler` entry: `isim` (speaker handle resolved from captured observations),
/// `puan_degisimi` (score delta, clamped to ±3 by `Bot::diarist`), `not` (updated note),
/// `bilgiler`/`etiketler` (new facts/tags to merge in). Field names Turkish for the same
/// reason as `Record` above.
#[derive(Deserialize, Default)]
struct PersonRecord {
    #[serde(default)]
    isim: String,
    #[serde(default)]
    puan_degisimi: i32,
    #[serde(default)]
    not: String,
    #[serde(default)]
    bilgiler: Vec<String>,
    #[serde(default)]
    etiketler: Vec<String>,
}
/// One `Record.konular` entry: `ad` (topic name), `not` (dated note to append). Field
/// names Turkish for the same reason as `Record` above.
#[derive(Deserialize, Default)]
struct TopicRecord {
    #[serde(default)]
    ad: String,
    #[serde(default)]
    not: String,
}

fn agent_snapshot(key: &str) -> Result<RecordSnapshot, BotError> {
    Ok(
        memory::read_versioned(key)?.unwrap_or_else(|| RecordSnapshot {
            key: key.to_owned(),
            content: String::new(),
            revision: 0,
            modified: 0,
        }),
    )
}

impl Bot {
    /// Updates the persisted group profile from recent messages.
    pub async fn profiler(&self) -> Result<bool, BotError> {
        let snapshot = agent_snapshot("profil.md")?;
        let sample = recent_messages(&self.state(), 600);
        if sample.is_empty() {
            return Ok(false);
        }
        let result = self
            .analyze(
                &sample,
                &self
                    .default_prompt_context()
                    .render(prompts::current().profile_extract),
                1200,
                "profilci",
            )
            .await?;
        memory::replace_generated(self, "profil.md", snapshot.revision, &result)?;
        log::debug!("profiler: profile updated");
        Ok(true)
    }

    // pulls what should be written to memory out of a finished chat (or a 6-hour
    // observation) and files it away
    /// `source` labels the observation in the prompt; `channel` labels the event entry.
    pub async fn diarist(
        &self,
        transcript: String,
        source: &str,
        channel: &str,
        observed: &HashMap<String, UserRef>,
    ) -> Result<DiaristSummary, BotError> {
        if transcript.trim().is_empty() {
            return Err("transcript is empty".into());
        }
        let mut platforms = observed.values().map(|person| person.id.platform());
        let first = platforms.next();
        let kind = first
            .filter(|kind| platforms.all(|other| other == *kind))
            .unwrap_or(platform::types::PlatformKind::Terminal);
        let context = platform::prompt_context::PromptContext::for_platform(kind);
        let self_snapshot = agent_snapshot("kendim.md")?;
        let (instruction, bot_name) = {
            let state = self.state();
            let text = context
                .render_with(
                    prompts::current().diarist,
                    &[
                        ("{ad}", &state.bot_name),
                        ("{kaynak}", source),
                        (
                            "{favori}",
                            state.favorite_name.as_deref().unwrap_or("kimse"),
                        ),
                    ],
                )
                .into_owned();
            (text, state.bot_name.clone())
        };
        let reply = match self
            .analyze_with(&transcript, &instruction, 1200, "gunlukcu", &context)
            .await
        {
            Ok(c) => c,
            Err(e) => {
                log::warn!("diarist: {e}");
                return Err(e);
            }
        };
        let mut summary = DiaristSummary {
            output_chars: reply.chars().count(),
            ..DiaristSummary::default()
        };
        let record: Record = match serde_json::from_str(extract_json(&reply)) {
            Ok(r) => r,
            Err(e) => {
                // don't waste the model's effort: an unparseable output is archived raw
                memory::archive(&format!("gunlukcu-{}.md", memory::slug(source)), &reply);
                log::warn!("diarist: couldn't parse json, raw transcript archived: {e}");
                return Err(format!(
                    "couldn't parse json ({e}); output starts with: {}",
                    memory::trim(&reply, 120)
                )
                .into());
            }
        };

        if !record.olay.is_empty() {
            memory::add_event(Some(self), channel, &record.olay)?;
            summary.events = 1;
        }
        for pr in record.kisiler {
            if pr.isim.is_empty() || pr.isim.eq_ignore_ascii_case(&bot_name) {
                continue;
            }
            // the mind is id-based: if the name can't be resolved, this record is skipped this round
            let Some(observation) = observed.get(&pr.isim.to_lowercase()) else {
                log::warn!("diarist: couldn't resolve '{}' to an id, skipped", pr.isim);
                continue;
            };
            let id = &observation.id;
            memory::update_person(Some(self), id, |person| {
                if person.name.is_empty() {
                    person.name = observation.display_name.clone();
                } else if !person.name.eq_ignore_ascii_case(&observation.display_name) {
                    // display name changed: don't lose the old one
                    if !person
                        .previous_names
                        .iter()
                        .any(|e| e.eq_ignore_ascii_case(&person.name))
                    {
                        person.previous_names.push(person.name.clone());
                    }
                    person.name = observation.display_name.clone();
                }
                if person.username.is_empty() {
                    if let Some(username) = &observation.username {
                        person.username = username.clone();
                    }
                }
                // whatever the model says, the limits are ours
                person.score = person.score.clamp(-10, 10) + pr.puan_degisimi.clamp(-3, 3);
                if !pr.not.trim().is_empty() {
                    person.note = pr.not.trim().to_string();
                }
                for fact in pr.bilgiler {
                    let fact = fact.trim().to_string();
                    if !fact.is_empty() && !person.facts.contains(&fact) {
                        person.facts.push(fact);
                    }
                }
                for tag in pr.etiketler {
                    let tag = tag.trim().to_lowercase();
                    if !tag.is_empty() && !person.tags.contains(&tag) {
                        person.tags.push(tag);
                    }
                }
                if !record.olay.is_empty() {
                    person
                        .events
                        .push(format!("{}: {}", memory::date_time(), record.olay));
                }
            })?;
            summary.people += 1;
        }
        for tr in record.konular {
            if !tr.ad.trim().is_empty() && !tr.not.trim().is_empty() {
                memory::add_topic(Some(self), tr.ad.trim(), &tr.not)?;
                summary.topics += 1;
            }
        }
        if !record.kendim.trim().is_empty() {
            memory::replace_generated(
                self,
                "kendim.md",
                self_snapshot.revision,
                record.kendim.trim(),
            )?;
        }
        log::info!(
            "mind: diarist [{source}]: {} person(s), {} topic(s), {} event(s) written",
            summary.people,
            summary.topics,
            summary.events
        );

        self.summarizer().await?;
        Ok(summary)
    }

    // shrinks files that are over their limit; the raw chunk that comes out goes to the archive
    pub async fn summarizer(&self) -> Result<(), BotError> {
        let context = self.default_prompt_context();
        for (kind, rel) in memory::over_limit()? {
            let Some(snapshot) = memory::read_versioned(&rel)? else {
                continue;
            };
            let old = &snapshot.content;

            let result = match kind {
                "kisi" => {
                    self.analyze(
                        old,
                        &context.render_with(
                            prompts::current().summarizer_person,
                            &[("{sinir}", &memory::PERSON_TARGET.to_string())],
                        ),
                        700,
                        "ozetleyici_kisi",
                    )
                    .await
                }
                "konu" => {
                    self.analyze(
                        old,
                        &context.render_with(
                            prompts::current().summarizer_topic,
                            &[("{sinir}", &memory::TOPIC_TARGET.to_string())],
                        ),
                        600,
                        "ozetleyici_konu",
                    )
                    .await
                }
                _ => {
                    // event file: the older half gets summarized, the newer half stays as-is
                    let other_lines: Vec<&str> =
                        old.lines().filter(|l| !l.starts_with("- ")).collect();
                    let lines: Vec<&str> = old.lines().filter(|l| l.starts_with("- ")).collect();
                    let cut = lines.len() * 6 / 10;
                    let (older, newer) = lines.split_at(cut);
                    match self
                        .analyze(
                            &older.join("\n"),
                            &context.render(prompts::current().summarizer_events),
                            400,
                            "ozetleyici_olaylar",
                        )
                        .await
                    {
                        Ok(summary_text) => Ok(format!(
                            "{}\n{}\n\n{}\n",
                            other_lines.join("\n").trim(),
                            summary_text.trim(),
                            newer.join("\n")
                        )),
                        Err(e) => Err(e),
                    }
                }
            };

            match result {
                Ok(new_content)
                    if !new_content.trim().is_empty() && new_content.len() < old.len() =>
                {
                    let content = if let Some(id) = platform::keys::parse_person_record(&rel) {
                        let mut person = Person::parse(&id, &new_content);
                        if person.name.is_empty()
                            || !new_content.contains("## Bildik")
                            || !new_content.contains("## Son")
                        {
                            return Err(
                                format!("summarizer: invalid person record for {rel}").into()
                            );
                        }
                        let original = Person::parse(&id, old);
                        person.name = original.name;
                        person.username = original.username;
                        person.previous_names = original.previous_names;
                        person.score = original.score.clamp(-10, 10);
                        person.tags = original.tags;
                        person.note = original.note;
                        if is_owner(&id) {
                            person.score = 10;
                            person.note = memory::FAVORITE_NOTE.to_owned();
                        }
                        person.text_preserving(old)
                    } else {
                        new_content.trim_end().to_owned()
                    };
                    if content.len() >= old.len() {
                        return Err(format!(
                            "summarizer: {rel} didn't shrink after preserving fields"
                        )
                        .into());
                    }
                    memory::replace_generated(self, &rel, snapshot.revision, &content)?;
                    // Durable history already retains the entire displaced body.
                    memory::archive(&rel, old);
                    log::debug!(
                        "summarizer: {rel} {} -> {} characters",
                        old.len(),
                        content.len()
                    );
                }
                Ok(_) => return Err(format!("summarizer: {rel} didn't shrink").into()),
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// Updates temperament notes; does nothing until message history exists.
    pub async fn coach(&self) -> Result<bool, BotError> {
        let context = self.default_prompt_context();
        let snapshot = agent_snapshot("huy.md")?;
        let (text, instruction) = {
            let state = self.state();
            if state.recent_messages.is_empty() {
                return Ok(false);
            }
            let own = state
                .own_messages
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            let text = format!(
                "GRUP PROFİLİ\n{}\n\nKİŞİ DİZİNİ\n{}\n\nGÜNDEM NOTLARI (internette okuyup düşündükleri)\n{}\n\nBOTUN SON HALİ\n{}\n\nŞU ANKİ HUYUN\n{}\n\nSON KONUŞMALAR\n{}\n\nBOTUN KENDİ SON MESAJLARI\n{}",
                state.profile,
                state.index,
                if state.agenda.is_empty() { "(henüz gezmedi)" } else { &state.agenda },
                if state.myself.is_empty() { "(bir şey yok)" } else { &state.myself },
                if snapshot.content.is_empty() { "(henüz yok, ilk kez yazıyorsun)" } else { &snapshot.content },
                recent_messages(&state, 200),
                if own.is_empty() { "(henüz konuşmadı)" } else { &own },
            );
            (
                text,
                context
                    .render_with(prompts::current().coach, &[("{ad}", &state.bot_name)])
                    .into_owned(),
            )
        };
        let result = self.analyze(&text, &instruction, 800, "hoca").await?;
        memory::replace_generated(self, "huy.md", snapshot.revision, &result)?;
        log::debug!("coach: temperament updated");
        Ok(true)
    }

    /// Saves corrections from a finished chat; ignores an empty transcript.
    pub async fn critic(&self, transcript: String) {
        if transcript.trim().is_empty() {
            return;
        }
        let context = self.default_prompt_context();
        let snapshot = match agent_snapshot("duzeltmeler.md") {
            Ok(snapshot) => snapshot,
            Err(error) => return log::warn!("critic: {error}"),
        };
        let instruction = {
            let state = self.state();
            context
                .render_with(
                    prompts::current().critic,
                    &[("{ad}", &state.bot_name), ("{mevcut}", &snapshot.content)],
                )
                .into_owned()
        };
        match self
            .analyze(&transcript, &instruction, 400, "elestirmen")
            .await
        {
            Ok(notes) => {
                match memory::replace_generated(self, "duzeltmeler.md", snapshot.revision, &notes) {
                    Ok(_) => log::debug!("critic: notes updated"),
                    Err(error) => log::warn!("critic: discarded result: {error}"),
                }
            }
            Err(e) => log::warn!("critic: {e}"),
        }
    }

    // gathers news from two sources (hacker news + Turkey's agenda), picks one based on the profile
    /// None means successfully fetched sources contain no unposted news.
    pub async fn news_agent(&self) -> Result<Option<News>, BotError> {
        let posted = self.state().posted_news.clone();
        let mut news_items: Vec<News> = Vec::new();

        let ids: Vec<u64> = self
            .http
            .get("https://hacker-news.firebaseio.com/v0/topstories.json")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        for id in ids.into_iter().filter(|id| !posted.contains(id)).take(12) {
            let url = format!("https://hacker-news.firebaseio.com/v0/item/{id}.json");
            let mut item: News = self
                .http
                .get(&url)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            if item.title.is_empty() {
                continue;
            }
            item.source = "hn";
            news_items.push(item);
        }

        for article in agenda::rss(&self.http).await?.into_iter().take(12) {
            let id = agenda::link_id(&article.link);
            if !posted.contains(&id) {
                news_items.push(News {
                    id,
                    title: article.title,
                    url: article.link,
                    score: 0,
                    source: "gündem",
                });
            }
        }
        if news_items.is_empty() {
            return Ok(None);
        }

        let list = news_items
            .iter()
            .enumerate()
            .map(|(i, item)| format!("{i}. [{}] {}", item.source, item.title))
            .collect::<Vec<_>>()
            .join("\n");
        let profile = self.state().profile.clone();
        let pick = self
            .analyze(
                &list,
                &self
                    .default_prompt_context()
                    .render_with(prompts::current().news_pick, &[("{profil}", &profile)]),
                10,
                "haber_sec",
            )
            .await?;
        let n: usize = pick
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()?;
        if n >= news_items.len() {
            return Err("news selection is outside the fetched candidates".into());
        }
        Ok(Some(news_items.swap_remove(n)))
    }

    // shows the model the image and gets a one-line personality comment; if the model
    // doesn't support vision, it writes blind
    pub async fn image_commenter(
        &self,
        channel: &ConversationKey,
        path: &PathBuf,
    ) -> Result<String, BotError> {
        let context = self.prompt_context(channel);
        let (system, bot_name) = {
            let state = self.state();
            let (fixed, variable) = system_text_with(
                &state,
                &context.render(prompts::current().image_post),
                "",
                &context,
            );
            (format!("{fixed}\n\n{variable}"), state.bot_name.clone())
        };
        let data = tokio::fs::read(path).await?;
        let mime = match path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase()
            .as_str()
        {
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            _ => "image/png",
        };
        let b64 = base64::engine::general_purpose::STANDARD.encode(data);
        let body = serde_json::json!({
            "model": self.state().model.clone(),
            "max_tokens": 120,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": [
                    {"type": "text", "text": "görsel ekte"},
                    {"type": "image_url", "image_url": {"url": format!("data:{mime};base64,{b64}")}}
                ]}
            ]
        });
        let reply = match self.ask_raw(body, "resimci").await {
            Ok(c) => c,
            Err(_) => {
                self.generate_for(
                    channel,
                    &[user("bir görsel atıyorsun ama ne olduğunu hatırlamıyorsun")],
                    &context.render(prompts::current().image_post),
                    Some(120),
                    "resimci",
                )
                .await?
            }
        };
        Ok(clean(reply, &bot_name))
    }
}

/// One candidate/picked news item. Holds `id` (Hacker News numeric id, or `agenda::link_id`
/// for RSS items), `title`, `url`, `score` (Hacker News only, kept for potential future
/// ranking, currently unused — `#[allow(dead_code)]`), `source` (`"hn"`/`"gündem"`, not
/// serialized — set after deserializing). Produced by `Bot::news_agent` above; consumed by
/// `Bot::send_news`/`post_news` (`cycle_news.rs`).
#[derive(Deserialize, Default, Clone)]
pub struct News {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub score: i64,
    #[serde(skip)]
    pub source: &'static str,
}

// a random image from the photos/ folder
/// Accepts png/jpg/jpeg/gif/webp; returns `None` if no image is available.
pub fn random_image() -> Option<PathBuf> {
    let files: Vec<PathBuf> = std::fs::read_dir(IMAGE_DIR)
        .ok()?
        .flatten()
        .map(|d| d.path())
        .filter(|p| {
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp")
        })
        .collect();
    if files.is_empty() {
        return None;
    }
    let i = rand::random::<usize>() % files.len();
    Some(files[i].clone())
}
