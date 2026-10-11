# Architecture

## One sentence
Discord events → chat engine (short chats of 12 messages) → each reply's "system message" =
core personality + what the agents have taught it + what's retrieved from memory for that
chat + the task; a finished chat → agents update memory and personality →
`durum/hafiza.redb` (+ `durum/arsiv/`).

## Layers

```
┌──────────────────────── discord (serenity) ────────────────────────┐
│ ready · guild_create · guild_member_addition · message              │
└───────────────┬────────────────────────────────────────────────────┘
                ▼
┌──────────────────────── chat engine (main.rs) ──────────────────────┐
│ State (single Mutex) · Chat{history,counter,hacked} · reply() · generate()│
│ cycles: news · poke · prank · wanderer · sleep                      │
└───────┬───────────────────────┬────────────────────────────────────┘
        ▼                       ▼
┌── openrouter (ask_raw) ───┐  ┌── memory (memory.rs) ──────────────────┐
│ ask → generate (with personality)│  │ INDEX.md · kisiler/ · konular/ ·  │
│ ask → analyze (no personality)│     │ olaylar/ · arsiv/ · retrieve() · limits │
└──────────────────────────┘  └───────────────────────────────────────┘
        ▲                       ▲
┌── agents (agents.rs, agenda.rs) ──────────────────────────────────┐
│ profiler · diarist · coach · critic · summarizer · news_agent ·     │
│ image_commenter · wanderer                                          │
└─────────────────────────────────────────────────────────────────────┘
        ▲
┌── calendar (sleep.rs, travel.rs) ── holds no state, computes from the clock ──┐
└─────────────────────────────────────────────────────────────────────┘
```

## File map
| File | Lines | Role |
|---|---|---|
| `src/main.rs` | ~90 | `mod`/`use` header, `include!("bot/<group>/<group>.rs")`, `main` |
| `src/bot/<group>/*.rs` | 7 subfolders, ~38 files, most <200 | main.rs's former content split by topic (below) |
| `src/command.rs` + `src/command/*.rs` | ~10 files, all <200 | slash command handler (below) |
| `src/modal.rs` | ~710 | embed/component/modal builders (`/zihin` card, detail modals, settings panel, `info_embed`), slash registration `register_commands` |
| `src/chat_cli.rs` | ~170 | `cargo run -- chat`: discord-less terminal chat rig (`impl Bot`), for trying out the output protocol |
| `src/agents.rs` | ~420 | background agents (`impl Bot` block), `News`, `random_image` |
| `src/memory.rs` | ~1025 | redb memory (`durum/hafiza.redb`): read/write/archive, `Person` format, index, retrieval, limits, dates |
| `src/migrate.rs` | ~210 | one-time migration from the old `durum/` markdown tree to `hafiza.redb` (`cargo run -- migrate-durum`) |
| `src/agenda.rs` | ~265 | Sözcü RSS, html cleanup, firecrawl, `wander` agent, `gundem.md` |
| `src/sleep.rs` | ~130 | sleep schedule, local time, sleepless night, "ŞU AN" line |
| `src/travel.rs` | ~240 | yearly event calendar, travel window, "ŞU AN" addendum |
| `src/prompts.rs` | 33 | `include_str!` constant for each prompt |
| `prompts/tr/*.md` | 31 files | prompt texts (file name+content stay Turkish, see AGENTS.md item 8) |
| `langs/tr.json` | 1 file | Discord-facing text (command name/description, embed, button — see `docs/prompts.md`) |
| `Cargo.toml` | | serenity(cache), tokio, reqwest(rustls), serde, serde_json, dotenvy, rand, base64 |

Modules are children of `main.rs`; they reach the root's private items (constants, `State`,
`Bot`, helpers) via `use super::*`. Within the same crate, `impl Bot` blocks are spread across
files.

### `src/bot/*.rs` and `src/command/*.rs`: `include!`, not a real `mod`
Both split the former single-file content of main.rs (respectively command.rs) into files by
topic, mostly under 200 lines — but wired in with `include!`, not `mod` (see
`docs/decisions.md`): so that visibility, `use super::*`, and the `reply_budget!` macro's scope
never change anywhere, the files compile as if they were written in the same (root) module; the
other six sibling modules (`agents.rs, agenda.rs, command.rs, modal.rs, chat_cli.rs, sleep.rs`)
were never touched. Once `src/bot/` had piled up 38 files in a single folder (2026-09-03) it was
moved into 7 topic-based subfolders — each subfolder has its own aggregator file
(`<group>/<group>.rs`, carrying the same name as the folder), which in turn collects the other
files in that folder via `include!("...")` (relative to the folder, no `bot/` prefix); `main.rs`
only calls `include!("bot/<group>/<group>.rs")`:
`types/` (constants+`State`/`Bot`/`Chat`/`ChatMessage`/`ThinkingMode`), `text/` (pure
text/protocol helpers, `Reply`, `parse_reply`), `provider/` (the AI call layer — split across
six separate `impl Bot` blocks, from `ask_raw` to `send_reply`), `chat/` (the `reply` loop,
`research`), `cycle/` (growth+memory scanning+background cycles+actions), `handler/` (discord
events — `handler_event.rs`+`handler_buttons.rs`; since `impl EventHandler for Handler` has to
be a single trait impl, `handler_event.rs` stayed at 423 lines, a Rust E0119 constraint),
`tests/` (main.rs's former test module, `tests_1..4.rs`). `setup.rs` (`Bot::setup`, startup)
stayed folderless, directly under `src/bot/`, since it's a single file. `src/command/` groups
(not put into folders, 7 files is already few): `registration_*` (registration table+helpers),
`cards.rs`/`actions.rs`/`settings.rs` (command bodies), `remaining.rs`
(`wake`/`put_to_sleep`/`set_debug`/`model_exists`).

### Discord boundary characterization (step 0)
The runtime is still Discord-only, with Serenity types and `include!` shared scope. Step 0
adds characterization fixtures around the existing ingress and UI builders; it does not
introduce domain IDs, a transport trait, a memory-key migration, or another platform.

- **Ingress (`src/bot/tests/tests_4.rs`):** bot/webhook/DM exclusions and configured
  guild/channel scope, Serenity's safe-content decoding, option-based global-name fallback,
  custom-reaction labels, and first-image selection. Image eligibility is the case-sensitive
  `image/` MIME prefix, not filename extension or dimensions; analysis-off leaves the
  original safe text without image markers. Image-only messages retain the existing Turkish
  marker even when the UI language is English.
- **Slash registration (`src/command.rs`):** the ordered 16-command table and serialized
  `CreateCommand`/option shape. All six arguments are optional; sleep has minimum 1 and no
  maximum. Thinking choices keep `goster/gizle/sessiz/kapat` and debug keeps `ac/kapat`
  on the wire while their labels and argument names follow `BOT_LANG`. The registration
  itself supplies no owner permission restriction; `/model` ownership is enforced by its
  existing executor.
- **Private UI (`src/modal.rs`):** status, help, mind, and the initial settings card serialize
  with ephemeral flag 64, one rich embed, empty attachments, and no plain-text content or
  explicit allowed-mentions field. The settings refresh uses `UpdateMessage` (type 7) with
  explicit flags 0, otherwise the same card/components. Its two button rows retain the
  Turkish thinking-mode suffixes, active-mode/debug styles, and awake/asleep disabled states.
  The mind detail buttons retain their existing IDs; the person-menu row is optional.
  Status adds the token breakdown only when categories exist. Info-embed descriptions fit
  1024 Unicode scalar values, including the existing Turkish truncation marker.

These fixtures are offline builder/decoder characterizations, not live Discord API probes.
The existing 93-test baseline is retained as an acceptance requirement; integration checks
are recorded separately in the progress log.


## Shared state
A single `Mutex<State>` (`Bot::state()` is poison-resilient). Inside it: bot name, favorite's
name, agent outputs (profile, temperament, corrections, myself, index, agenda), raw memory
(`VecDeque<String>`, 2000 lines), the bot's own messages (50), open chats, busy channels,
banned channels, channels awaiting news, posted news ids, scanned guilds, sleep schedules,
mentions received while asleep, travel-announcement flags. The lock is never held across a
single `.await`.

## Every reply's system message (system_text)
In order, empty ones are skipped:
1. `kisilik.md` (`{ad}`, `{favori_satiri}` filled in)
2. HUYUN — `huy.md` (coach)
3. BU GRUP HAKKINDA BİLDİKLERİN — `profil.md` (profiler)
4. HAFIZA DİZİNİ — `INDEX.md` (index: person+score+tag, topics, event count)
5. BU SOHBET İÇİN HAFIZADAN GETİRİLENLER — `memory::retrieve` (budget 6000 chars)
6. GÜNDEM — `gundem.md` last 3 entries (wanderer)
7. SENİN SON HALİN — `kendim.md` (diarist)
8. ŞU AN — date/time/day + travel status (sleep is only a reply gate in code)
9. KENDİNE NOTLAR — `duzeltmeler.md` (critic)
10. ŞU ANKİ GÖREVİN — that call's instruction (farewell, hack, welcome, message from the
    road...)

## Agent schedule
| When | Agents |
|---|---|
| on connecting to a guild (once per guild) | read_history → profiler → coach (if temperament is empty) |
| 10 min after startup, then every 4 hours (while awake) | wander |
| every 6 hours (while awake) | profiler → diarist(observation) → coach → news_agent+share; if traveling, only profiler+coach |
| every finished chat | diarist(chat) → summarizer → critic |
| once a minute | sleep-schedule check, reply to pending mentions on waking |
