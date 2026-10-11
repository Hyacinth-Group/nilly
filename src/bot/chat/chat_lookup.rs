impl Bot {
    // is this the same as one of the bot's last 5 messages?
    /// Input: `&self`; `channel: &RoomKey`; `reply: &str` — a candidate line. Output:
    /// `bool` — case-insensitive match against the channel's last 5 bot lines. Uses:
    /// `self.state().channel_history`. Used by: `Bot::send_stream` (`provider_send_stream.rs`,
    /// per line), `Bot::reply`'s fallback (`chat_reply.rs`).
    fn is_repeat(&self, channel: &RoomKey, reply: &str) -> bool {
        let state = self.state();
        let prefix = format!("{}: ", state.bot_name);
        let target = reply.trim().to_lowercase();
        state
            .channel_history
            .get(channel)
            .map(|hist| {
                hist.iter()
                    .rev()
                    .filter_map(|l| l.strip_prefix(&prefix))
                    .take(5)
                    .any(|l| l.trim().to_lowercase() == target)
            })
            .unwrap_or(false)
    }

    // looks something up online if the message calls for it: a link -> the page; "araştır/bak"
    // ("look it up") -> a firecrawl search (if a key is set); "haber/gündem/ne oldu" ("news/what's up") -> rss headlines
    /// Input: `&self`; `text: &str` — the triggering message. Output: `Option<String>` — a
    /// Turkish-language findings blurb to fold into the reply instruction, or `None` if no
    /// trigger matched or the lookup failed. Uses: `self.read_page` (`agenda.rs`),
    /// `self.firecrawl_search` (`agenda.rs`), `agenda::rss`, `memory::trim`. Used by:
    /// `Bot::reply` (`chat_reply.rs`), the only caller.
    async fn research(&self, text: &str) -> Option<String> {
        let lower = text.to_lowercase();
        if let Some(url) = text
            .split_whitespace()
            .find(|w| w.starts_with("http://") || w.starts_with("https://"))
        {
            let url = url.trim_end_matches(['>', ')', ',', '.']);
            return match self.read_page(url).await {
                Ok(s) if !s.trim().is_empty() => {
                    Some(format!("Atılan link ({url}):\n{}", memory::trim(&s, 1500)))
                }
                _ => Some(format!("Link açılamadı: {url}")),
            };
        }
        let contains_any = |list: &[&str]| list.iter().any(|k| lower.contains(k));
        let news = contains_any(&[
            "haber",
            "gündem",
            "ne oldu",
            "son dakika",
            "neler oluyor",
            "güncel",
        ]);
        let triggers = [
            "araştır",
            "bak bakalım",
            "baksana",
            "bi bak",
            "googlela",
            "ara bakalım",
            "arasana",
            "internete bak",
            "internetten bak",
        ];
        let search = contains_any(&triggers);
        if search && self.firecrawl.is_some() {
            let mut query = lower.clone();
            for k in triggers
                .iter()
                .chain(["bakar mısın", " lan", " la ", " aq"].iter())
            {
                query = query.replace(k, " ");
            }
            let query: String = query
                .split_whitespace()
                .filter(|w| !w.starts_with('@'))
                .collect::<Vec<_>>()
                .join(" ");
            let query = if query.trim().is_empty() {
                text.to_string()
            } else {
                query
            };
            if let Ok(result) = self.firecrawl_search(&query).await {
                return Some(format!("\"{query}\" araması:\n{result}"));
            }
        }
        if news || search {
            if let Ok(rss) = agenda::rss(&self.http).await {
                let list = rss
                    .iter()
                    .take(12)
                    .map(|h| format!("- {} — {}", h.title, memory::trim(&h.summary, 100)))
                    .collect::<Vec<_>>()
                    .join("\n");
                return Some(format!("Sözcü'den şu anki başlıklar:\n{list}"));
            }
        }
        None
    }
}
