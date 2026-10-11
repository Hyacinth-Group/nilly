// News agenda: BBC World in English, Sozcu in Turkish, or a NEWS_RSS override.
// Fetches pages via firecrawl (or a plain download if there's no key), and the bot writes
// its own take into its journal (durum/gundem.md). The coach and every reply read this; it feeds the
// personality too. The news agent uses the same RSS feed as its news source.

use super::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub fn rss_url() -> String {
    if let Ok(raw) = crate::config::runtime_value("NEWS_RSS") {
        let raw = raw.trim();
        if !raw.is_empty() {
            return raw.to_string();
        }
    }
    match lang::Lang::current() {
        lang::Lang::Tr => "https://www.sozcu.com.tr/rss/news.xml",
        lang::Lang::En => "https://feeds.bbci.co.uk/news/world/rss.xml",
    }
    .to_string()
}

pub const AGENDA_ENTRIES: usize = 12; // how many entries stay in gundem.md, older ones go to the archive
pub const PAGE_LIMIT: usize = 3500; // characters of a page sent to the model

/// One RSS item: `title`, `link`, `summary` (the feed's own description, cleaned of HTML).
/// Produced by `rss` below; consumed by `Bot::wander` here and `Bot::news_agent`
/// (`agents.rs`), `Bot::research` (`chat_lookup.rs`).
pub struct RssNews {
    pub title: String,
    pub link: String,
    pub summary: String,
}

// html to plain text: script/style dropped, tags stripped, whitespace collapsed
/// Decodes entities as well as stripping script/style blocks and tags.
pub fn clean_html(raw: &str) -> String {
    let mut s = raw.replace("<![CDATA[", "").replace("]]>", "");
    for tag in ["script", "style"] {
        while let Some(start) = s.find(&format!("<{tag}")) {
            match s[start..].find(&format!("</{tag}>")) {
                Some(end) => s.replace_range(start..start + end + tag.len() + 3, " "),
                None => break,
            }
        }
    }
    let mut out = String::new();
    let mut inside = false;
    for c in s.chars() {
        match c {
            '<' => inside = true,
            '>' => inside = false,
            c if !inside => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Extracts cleaned text from an XML element, allowing attributes in its opening tag.
fn tag_content(chunk: &str, tag: &str) -> Option<String> {
    let start = chunk
        .find(&format!("<{tag}>"))
        .or_else(|| chunk.find(&format!("<{tag} ")))?;
    let start = start + chunk[start..].find('>')? + 1;
    let end = start + chunk[start..].find(&format!("</{tag}>"))?;
    Some(clean_html(&chunk[start..end]))
}

/// Fetches and parses the configured RSS feed.
/// Keeps items with a non-empty title and an http(s) link; fails if none qualify.
pub async fn rss(http: &reqwest::Client) -> Result<Vec<RssNews>, BotError> {
    let xml = http
        .get(rss_url())
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let mut list = Vec::new();
    for chunk in xml.split("<item").skip(1) {
        let chunk = chunk.split("</item>").next().unwrap_or("");
        let (Some(title), Some(link)) = (tag_content(chunk, "title"), tag_content(chunk, "link"))
        else {
            continue;
        };
        if title.is_empty() || !link.starts_with("http") {
            continue;
        }
        list.push(RssNews {
            title,
            link,
            summary: tag_content(chunk, "description").unwrap_or_default(),
        });
    }
    if list.is_empty() {
        return Err("rss came back empty".into());
    }
    Ok(list)
}

// a number derived from the link, to remember which news items were already posted
/// RSS items lack numeric IDs, so the link hash acts as a deduplication key.
pub fn link_id(link: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    link.hash(&mut hasher);
    hasher.finish()
}

// gundem.md entries: each entry starts with a "## date time" line
/// Trims entries and drops empty blocks.
pub fn entries(text: &str) -> Vec<String> {
    let mut list: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.starts_with("## ") {
            list.push(line.to_string());
        } else if let Some(last) = list.last_mut() {
            last.push('\n');
            last.push_str(line);
        }
    }
    list.into_iter()
        .map(|g| g.trim().to_string())
        .filter(|g| !g.is_empty())
        .collect()
}

/// Returns the last three agenda entries, separated by blank lines.
pub fn latest_agenda(text: &str) -> String {
    let list = entries(text);
    let skip = list.len().saturating_sub(3);
    list[skip..].join("\n\n")
}

impl Bot {
    // page text: via firecrawl if a key is set, otherwise a plain download with tags stripped
    /// Caps the page text at `PAGE_LIMIT` characters.
    pub async fn read_page(&self, url: &str) -> Result<String, BotError> {
        let text = match &self.firecrawl {
            Some(key) => {
                #[derive(Deserialize)]
                struct Response {
                    data: Option<Data>,
                }
                #[derive(Deserialize)]
                struct Data {
                    markdown: Option<String>,
                }
                let response: Response = self
                    .http
                    .post("https://api.firecrawl.dev/v1/scrape")
                    .bearer_auth(key)
                    .json(&serde_json::json!({ "url": url, "formats": ["markdown"], "onlyMainContent": true }))
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                response
                    .data
                    .and_then(|d| d.markdown)
                    .ok_or("firecrawl returned empty")?
            }
            None => clean_html(
                &self
                    .http
                    .get(url)
                    .send()
                    .await?
                    .error_for_status()?
                    .text()
                    .await?,
            ),
        };
        Ok(text.chars().take(PAGE_LIMIT).collect())
    }

    // a firecrawl web search: title, description, url (an error if there's no key)
    /// Returns up to five title/description/URL lines; fails without a key or results.
    pub async fn firecrawl_search(&self, query: &str) -> Result<String, BotError> {
        let key = self.firecrawl.as_ref().ok_or("no firecrawl key set")?;
        #[derive(Deserialize)]
        struct Response {
            data: Option<Vec<SearchResult>>,
        }
        #[derive(Deserialize, Default)]
        struct SearchResult {
            #[serde(default)]
            title: String,
            #[serde(default)]
            description: String,
            #[serde(default)]
            url: String,
        }
        let response: Response = self
            .http
            .post("https://api.firecrawl.dev/v1/search")
            .bearer_auth(key)
            .json(&serde_json::json!({ "query": query, "limit": 5 }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let list = response
            .data
            .unwrap_or_default()
            .iter()
            .filter(|r| !r.title.is_empty())
            .map(|r| {
                format!(
                    "- {} — {} ({})",
                    r.title,
                    memory::trim(&r.description, 160),
                    r.url
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        if list.is_empty() {
            return Err("search came back empty".into());
        }
        Ok(list)
    }

    // browses the news every so often: picks whatever catches its interest from the rss
    // feed, reads it, writes its own take into its journal
    /// Returns false for an empty feed; propagates model/storage conflicts to administration.
    pub async fn wander(&self) -> Result<bool, BotError> {
        let snapshot = memory::read_versioned("gundem.md")?;
        let expected = snapshot.as_ref().map(|record| record.revision).unwrap_or(0);
        let news_items = rss(&self.http).await?;
        if news_items.is_empty() {
            return Ok(false);
        }
        let list = news_items
            .iter()
            .take(20)
            .enumerate()
            .map(|(i, item)| format!("{i}. {} — {}", item.title, memory::trim(&item.summary, 120)))
            .collect::<Vec<_>>()
            .join("\n");
        let context = self.default_prompt_context();
        let instruction = {
            let state = self.state();
            context
                .render_with(
                    prompts::current().wanderer_pick,
                    &[
                        ("{ad}", &state.bot_name),
                        ("{huy}", &state.temperament),
                        ("{profil}", &state.profile),
                    ],
                )
                .into_owned()
        };
        let pick = self.analyze(&list, &instruction, 20, "gezgin_sec").await?;
        let selected: Vec<usize> = pick
            .split(|c: char| !c.is_ascii_digit())
            .filter_map(|s| s.parse().ok())
            .filter(|n| *n < news_items.len().min(20))
            .take(3)
            .collect();
        if selected.is_empty() {
            return Err("wander: couldn't parse selected news".into());
        }

        let mut read_text = String::new();
        for i in selected {
            let item = &news_items[i];
            let content = match self.read_page(&item.link).await {
                Ok(m) if !m.trim().is_empty() => m,
                _ => item.summary.clone(),
            };
            read_text.push_str(&format!(
                "## {}\n{}\n{}\n\n",
                item.title, item.link, content
            ));
        }

        let note_instruction = context.render(prompts::current().wanderer_note);
        let note = self
            .generate(
                &[user(read_text)],
                &note_instruction,
                Some(350),
                "gezgin_not",
            )
            .await?;

        let mut list = entries(
            snapshot
                .as_ref()
                .map(|record| record.content.as_str())
                .unwrap_or(""),
        );
        list.push(format!("## {}\n{}", memory::date_time(), note.trim()));
        let mut archived = Vec::new();
        while list.len() > AGENDA_ENTRIES {
            archived.push(list.remove(0));
        }
        let text = list.join("\n\n");
        memory::replace_generated(self, "gundem.md", expected, &text)?;
        // Transaction history retains displaced prose even if these filesystem copies fail.
        for entry in archived {
            memory::archive("gundem.md", &entry);
        }
        log::debug!("wander: agenda note written");
        Ok(true)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// Verifies `tag_content` extracts title/link/description from a raw RSS `<item>` chunk, CDATA included.
    #[test]
    fn rss_splits_into_items() {
        let xml = r#"<rss><channel><atom:link href="x"/><item><title><![CDATA[Başlık &amp; devam]]></title>
        <link>https://ornek.com/a</link><description><![CDATA[<p>özet <b>kalın</b></p>]]></description></item>
        <item><title>ikinci</title><link>https://ornek.com/b</link></item></channel></rss>"#;
        let mut list = Vec::new();
        for chunk in xml.split("<item").skip(1) {
            let chunk = chunk.split("</item>").next().unwrap();
            list.push((
                tag_content(chunk, "title").unwrap(),
                tag_content(chunk, "link").unwrap(),
                tag_content(chunk, "description").unwrap_or_default(),
            ));
        }
        assert_eq!(list[0].0, "Başlık & devam");
        assert_eq!(list[0].1, "https://ornek.com/a");
        assert_eq!(list[0].2, "özet kalın");
        assert_eq!(list[1].0, "ikinci");
    }

    /// Verifies `clean_html` strips scripts/tags and unescapes entities.
    #[test]
    fn html_gets_cleaned() {
        let html = "<html><script>var a=1;</script><body><h1>Selam</h1>  <p>dünya &amp; ötesi</p></body></html>";
        assert_eq!(clean_html(html), "Selam dünya & ötesi");
    }

    /// Verifies `entries` splits an agenda file into its `##`-headed blocks, and `latest_agenda` returns the whole text unchanged when nothing needs trimming.
    #[test]
    fn agenda_entries_parsed() {
        let text = "## 2026-09-01 10:00\nbir\niki\n\n## 2026-09-01 14:00\nüç";
        let result = entries(text);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "## 2026-09-01 10:00\nbir\niki");
        assert_eq!(latest_agenda(text), text);
    }
}
