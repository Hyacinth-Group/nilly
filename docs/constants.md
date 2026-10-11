# Constants

## src/bot/types/types_settings.rs (in the module main.rs includes via include!)
| Constant | Value | Meaning |
|---|---|---|
| VERSION / VERSION_COMMIT / VERSION_DATE | Cargo.toml version / the commit build.rs got from git (+ = there were uncommitted changes at build time) / build date | `version_text()`: /durum and the startup announcement |
| OPENROUTER_URL / OPENROUTER_MODEL | …/api/v1/chat/completions / openai/gpt-4o-mini | default provider |
| MISTRAL_URL / MISTRAL_MODEL | api.mistral.ai/v1/chat/completions / mistral-medium-latest | if MISTRAL_KEY is set, or PROVIDER=mistral |
| CHANCE | 0.35 | fallback dice roll: probability of jumping in if the willingness call fails |
| WILLINGNESS_THRESHOLD / EVALUATION_INTERVAL | 6 / 2 min | willingness score threshold / most frequent evaluation per channel |
| CHAT_TIMEOUT | 30 min | a chat that's been silent this long closes without a goodbye |
| COMMENT_WINDOW | 2 hours | waiting window for comments after posting news |
| NEWS_INTERVAL | 6 hours | news round and the 6-hourly agents |
| POKE_INTERVAL / POKE_CHANCE | 1 hour / 0.3 | spontaneous chiming in |
| PRANK_INTERVAL / PRANK_CHANCE / HACK_SHARE / HACK_MESSAGES | 3 hours / 0.1 / 0.3 / 3 | image and hack prank |
| PROBLEM_SHARE | 0.25 | share of chime-in rounds that are about a coding problem |
| CHANNEL_HISTORY / CHAT_SEED | 60 / 10 | lines kept per channel / seed for a new chat |
| HISTORY_DAYS | 14 | depth of the startup scan |
| MEMORY_SIZE | 2000 | raw memory lines |
| CHAT_SIZE | 20 | chat history sent to the model |
| MESSAGE_LIMIT | 1900 | margin under Discord's 2000 limit |
| STREAM_EDIT_INTERVAL | 1200 ms | minimum time between two edits while streaming (Discord edit rate limit) |
| BURST_LIMIT | 4 | max lines (= separate messages) sent per turn; the rest are dropped. Based on a measurement: in real IM, a person's consecutive message run averages 1.7 messages, 42% of runs are multi-message (Baron 2010) — "split every reply into three" would be wrong, 4 is a ceiling, not a target |
| HALF_LINE_THRESHOLD | 12 | while streaming, the last line (not yet followed by `\n`) is not shown unless it exceeds this many characters; so a "tep" doesn't become a message in its half-typed state and then get deleted on the next edit |
| LINE_DELAY_BASE / _PER_CHAR / _CAP | 300 ms / 15 ms per character / 1500 ms | `send_lines` (NON-stream paths) delay between lines + typing indicator. There is NO delay on the stream path (the stream's own pace is enough). **These three values were never measured**, roughly chosen from human typing speed; may need tuning in production |
| CONNECT_TIMEOUT / READ_TIMEOUT | 15 s / 120 s | http: handshake / between two data chunks (covers the first token). No total time limit, a long thinking stream isn't cut off |
| AI_RETRIES | 2 | extra retry count on network error / 429 / 5xx (total is this + 1) |
| `reply_budget!()` (macro) / REPLY_CAP | debug `Some(2000)` / release `Some(4096)` | chat reply token budget; both have an upper bound, in release it only cuts off runaway cases like repetition/loops |
| REASONING_BUDGET_BASE | 1500 | in a non-stream agent call, when reasoning can't be turned off, the retry budget: max(2×current, this) |
| `owner_id()` | `OWNER_ID`, unset means none | user always favored, and the only user allowed to change `/model` |
| WANDERER_INTERVAL | 4 hours | agenda wandering |
| IMAGE_DIR / STATE_DIR | photos / durum | folders (relative to the working directory; real disk paths) |

## src/memory.rs
PERSON_LIMIT 1800 · PERSON_TARGET 1000 · TOPIC_LIMIT 1500 · TOPIC_TARGET 800 · EVENT_LIMIT 6000 ·
CONTEXT_BUDGET 6000 · INDEX_PEOPLE 40 · FAVORITE_NOTE · STOPWORDS (filtered-out words). Storage
is `durum/hafiza.redb` (fixed file name, not configurable via `.env`); `durum/arsiv/` is the one
exception, still real files (see docs/state-files.md).

## src/agenda.rs
`NEWS_RSS` overrides the feed; default BBC World for English, Sözcü for Turkish · AGENDA_ENTRIES 12 · PAGE_LIMIT 3500

## src/sleep.rs
`TZ_OFFSET` defaults to 0 hours from UTC (fractional offsets supported) · INSOMNIA_CHANCE 0.07 · INSOMNIA_TENSE 0.20 ·
normal sleep 01:00→09:00 ±45 min · sleepless night: up at 01:00, 06:00→13:00 ±45 min

## src/travel.rs
Turkish-only EVENTS table (disabled for `BOT_LANG=en`): new year's 30 Dec 4d, semester break 24 Jan 7d,
ramadan feast 2026: 19 Mar 4d / 2027: 8 Mar, 23 Apr 3d, 19 May 3d, sacrifice feast
2026: 26 May 5d / 2027: 15 May, summer 14 Jul 6d, zeytinli rock 21 Aug 4d, 30 Aug 3d, 29 Oct 3d.

## Numbers embedded in code (not constants)
`send` own message buffer 50 · `pending_mentions` 20 · `retrieve` person ≤4/1200, topic ≤2/800,
event 8, raw lines 12/200, keyword ≤40, ≥2 matches · `read_history` page 100 · news_agent HN 12 +
RSS 12 · wander rss 20, page ≤3 · on-the-road message once a day, 25% · coach last 200 lines ·
profiler 600 · observation 300 · hack entry max_tokens 150

## Scanned guilds
`taranan.md` is a **redb key**, not a physical file. The first-join history scan stores scanned
guild IDs there; `State::load` restores them and `guild_create` updates the record.

## Environment variables (.env)
See [configuration.md](configuration.md) for the complete environment reference.
`BOT_LANG` defaults to `en` and is fixed for the life of the process, independently of the OS `LANG`.
`GUILD_ID` and `CHANNELS` blank means every accessible server/channel; restrict them before first launch.

## src/growth.rs
NAME_STAGE 2 (yerlesik) · STAGES: yeni (0 days, 0 chats, confidence×0.7, poke×0.4) · isinma (3d, 8c, ×0.8, ×0.7) ·
yerlesik (10d, 25c, ×1, ×1) · eski-toprak (30d, 80c, ×1, ×1.2)
