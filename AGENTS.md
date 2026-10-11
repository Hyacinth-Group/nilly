# discord-bot — entry point for agents

This is the file AI agents developing this project should read FIRST. Kept short; detail
lives under `docs/`. Rule: only "always-true" information goes here.

## What this is
A bot that acts like a member who's been hanging around a Discord server for years. Rust
(serenity 0.12 + tokio + reqwest), replies through OpenRouter (default `openai/gpt-4o-mini`)
or Mistral (`mistral-medium-latest`); both are OpenAI-compatible chat/completions, picked via
`.env`. `MODEL` lets you pick any model through OpenRouter (GLM, Grok, Gemini, Claude, ...);
the only provider-specific difference is `cache_control` (prompt cache), added conditionally
based on the target address (`supports_cache`, src/main.rs — added on every request going to
openrouter.ai, the decision is left to openrouter; not added on Mistral's native API or on a
custom `API_URL` router). Personality isn't code — it comes from background agents and
file-based memory (`durum/`). Prompts live in `prompts/<lang>/*.md`, embedded at compile time
via `include_str!` (this directory and its filenames are deliberately kept Turkish — part of
the bot's Turkish way of operating; the code side is English, see item 8). The bot runs in a
single language, chosen via `.env`'s `BOT_LANG` (default `en`, item 12).

## Quick commands
```
cargo build            # build
cargo test              # 86 unit tests (memory, agenda, travel, stream, willingness, target, cache, output protocol, chat_cli, command table, reply parsing, reaction label)
cargo clippy             # 0 warnings expected
cargo fmt                # before every commit
cargo run --release      # .env: DISCORD_TOKEN + (OPENROUTER_KEY or MISTRAL_KEY); MODEL, PROVIDER, API_URL, FIRECRAWL_KEY, NEWS_CHANNEL, GUILD_ID, CHANNELS, DEBUG_CHANNEL, IMAGE_ANALYSIS, STEALTH optional
cargo run -- chat        # terminal chat bench without Discord (no token needed, just a model key); for trying out the output protocol
```

## Signpost
| Need | Where to look |
|---|---|
| Session state: what's done, open plan (read this FIRST after a compact) | docs/progress.md, docs/roadmap.md |
| Big picture, layers, data flow | docs/architecture.md |
| What a function does, who calls it, locking rules | docs/modules.md |
| Step-by-step what happens on an event (message, chat, sleep, travel, prank, news) | docs/flows.md |
| Output protocol (line = message, `-` silence, `tepki:` emoji, image, CLI bench) | docs/flows.md ("Output protocol", "CLI chat") |
| `durum/` file formats, limits, summarization | docs/state-files.md |
| Which prompt is used where, placeholders, max_tokens | docs/prompts.md |
| All constants and what they mean | docs/constants.md |
| Why things were built this way (decisions + rationale) | docs/decisions.md |
| Adding a new agent/prompt/cycle/state file, pitfalls, checklist | docs/development.md |
| Runtime vocabulary that stays Turkish (prompts, `durum/` fields, agent names) | docs/glossary.md |
| Growth stages and name-picking | docs/modules.md (growth), docs/flows.md |
| Code/doc knowledge graph: what connects to what, god nodes, cross-module bridges | graphify-out/graph.json (`/graphify query "..."`), graphify-out/GRAPH_REPORT.md |

## Invariant rules (also true in the code)
1. **A lock is never held across an await.** `Bot::state()` returns a `std::sync::MutexGuard`;
   it's always taken inside a `{ let state = bot.state(); ... }` block and dropped before any
   `.await`.
2. **Model output is bounded, the code doesn't trust it.** Scores are `clamp`ed, file sizes are
   fixed, 1900 characters per message (an over-limit reply isn't truncated, it's split into a
   new message). A reply is a line-based protocol (`parse_reply`): every line is its own
   message, **at most 4 lines per turn** (`BURST_LIMIT`) — normally 4 messages; a line over
   1900 characters is further split, thought messages are also added in thinking "show" mode.
   A lone `-` means silence, `tepki: 💀` is an emoji reaction instead of text (only known emoji
   blocks are accepted). The chat-reply budget lives in the `reply_budget!()` macro: debug
   `Some(2000)`, release `Some(REPLY_CAP=4096)` — both have an upper bound, an ordinary reply
   stays well under it, it only cuts off runaway cases like repetition/loops; other calls use a
   fixed max_tokens. Whatever the model says, the favorite gets +10.
3. **Mentions go out disabled** (`CreateAllowedMentions::new()`), only the welcome ping pings.
4. **No replies to bots, webhooks, or DMs.** It doesn't write while asleep but still listens:
   messages are processed into memory, and on waking it evaluates what was written overnight
   (a definite reply if it was tagged).
5. **Nothing in memory is ever deleted**: a file over its limit gets summarized, the raw chunk
   goes to `durum/arsiv/`.
6. **The only path that talks with personality is `Bot::generate` / `Bot::generate_stream`**,
   the only function that does analysis is `Bot::analyze`. Agents are personality-free. Any new
   "conversation" must go through `generate` (or `generate_stream` for a streamed chat reply),
   any new "evaluation" must go through `analyze`.
7. **Prompt text is never written into Rust** — it goes in `prompts/<lang>/*.md` and is wired
   up in `src/prompts.rs` via `include_str!`; text that reaches Discord (command name/
   description, embed, button) likewise isn't written into Rust — it goes in `langs/<lang>.json`
   and is read via `src/strings.rs`'s `t(key)` (item 12). Placeholders are curly-braced like
   `{name}`, filled in with `replace`.
8. **Identifiers are English and ASCII, comments are English** — but the bot's Turkish way of
   operating doesn't touch the code: `prompts/<lang>/*.md` and `langs/<lang>.json` (directory +
   filename + content), `durum/` file formats (field names, filenames), and everything that
   reaches Discord (slash command names/descriptions, embed text, button/menu labels, model
   output) stays Turkish. Code shouldn't read like "an AI wrote this": short, plain, comments
   state the reason. For the runtime vocabulary that stays Turkish, see docs/glossary.md.
9. `cargo fmt` reflows lines; if you're going to do a text-matching patch, read the file's
   current state first (docs/development.md "pitfalls").
10. **Session memory lives in the `docs/` folder.** If context gets compacted, or in a new
    session, read `docs/progress.md` and `docs/roadmap.md` first; drop a chronological note in
    `docs/progress.md` at every meaningful step (commit-sized), update `docs/roadmap.md` if the
    plan changes.
11. **The bot is managed only through slash (`/`) commands**, no `!`/text commands. Commands
    live in a single registration table (`command::definitions()`, src/command.rs): name,
    description, Discord options, and executor together; `modal::register_commands` generates
    the Discord registration from this table, `interaction_create` (main.rs) looks up
    `Interaction::Command` by name in the table and runs it. Every command returns an embed (no
    plain text); commands that might take over 3s (ones making a network/model call) `defer`
    first and edit the result in with `report_result`.
12. **The bot runs in a single language, fixed for the life of the process.** Chosen via
    `.env`'s `BOT_LANG` (`src/lang.rs`'s `Lang::current()`, read once on first call and
    cached; not `LANG` — most shells already set `LANG` for the OS locale). Personality/agent
    prompts come from `prompts/<lang>/` (`prompts::current()`), everything reaching Discord
    comes from `langs/<lang>.json` (`strings::t(key)`). `tr` and `en` are filled in; adding a
    new language means adding one file to each of these two directories plus one `match` arm
    each in `prompts.rs`/`strings.rs`.

## Knowledge graph (graphify)
`graphify-out/graph.json` is a persistent graph of the whole repo (Rust AST + docs + prompts),
built with [graphify](https://github.com/Graphify-Labs/graphify). Before reading many files to
answer a "what calls / what connects" question, query it: `/graphify query "<question>"`,
`/graphify path "A" "B"`, `/graphify explain "State"`. After changing code or docs, refresh it
with `/graphify . --update` (only changed files are re-read) and commit `graph.json` +
`GRAPH_REPORT.md` together with the change. `cache/`, `graph.html`, `cost.json` are per-machine
and ignored; rebuild the HTML with `graphify export html`.

## State folder (runtime, not tracked by git)
`durum/hafiza.redb` (redb, a single file): `INDEX.md` pointer · `kisiler/` `konular/`
`olaylar/` content · `huy.md profil.md duzeltmeler.md kendim.md gundem.md` agent outputs — all
stored under the same string key as their old file path, formats unchanged (see
docs/state-files.md). `durum/arsiv/` overflow: the one exception, still real `.md` files (for
human eyes only). Migrating from an older `durum/` tree: `cargo run -- migrate-durum`
(`src/migrate.rs`).

## Known gaps / unverified
- **2026-09-04: the bot was run live on its real Discord server, two rounds** (see
  docs/progress.md). Round 1: basic messaging (line-per-message bursting, `-` silence, `tepki:`
  emoji reactions), the slash command table (`command::definitions()`: registration, options,
  the defer+edit flow, embed output), and streaming + thinking (live edit cadence). Round 2:
  gpt-4o-mini's and Mistral's image commentary (`image_commenter`); the reasoning-mandatory
  model (glm-5.3-flash) resilience path (budget×2, effort=low) plus debug mode and the
  settings-panel buttons; willingness/target/waking mini-calls and their thresholds
  (WILLINGNESS_THRESHOLD=6, interest≥5); GUILD_ID/CHANNELS filters and conditional reply-to
  (`last_was_tagged`); emoji-reaction rate-limit behavior; `send_lines`'s inter-line delay
  constants; CLI chat mode (`cargo run -- chat`) against a real model key; `Handler::reaction_add`
  + the `GUILD_MESSAGE_REACTIONS` intent; `BOT_LANG=en` end-to-end (an actual English reply from
  a real model, and `langs/en.json`'s ~160 keys rendering within Discord's embed/button size
  limits); the `supports_cache` openrouter assumption; and the `durum/hafiza.redb` migration run
  against a real production `durum/` tree. All confirmed working as designed. Option
  names/appearance under every choice/min/max combination weren't individually exercised.
- Thinking only shows up if the model produces it (`reasoning` / `reasoning_content`);
  gpt-4o-mini doesn't produce it, so today's behavior stands unchanged on that model.
- Person files are id-based (`kisiler/<id>.md`); a record whose name can't be resolved to an id
  is skipped for that round (`State.name_to_id`). Old slug files aren't read.
- Keyword matching is a plain substring; no stemming.
- Holiday dates are hand-written for 2026-2027 (`src/travel.rs`), later years need adding.
- Nickname changing (name picking) depends on the bot having CHANGE_NICKNAME permission on the
  server; if not, it's logged and the name is used anyway — this specific fallback path (the
  bot lacking the permission) hasn't been observed live.
- **2026-09-03: the codebase (src/**/*.rs, README.md) was translated from Turkish to English**
  (identifiers, comments, file/directory names, .env variable names). The bot's way of
  operating (prompts/, `durum/` file formats, everything reaching Discord) was deliberately
  left Turkish — see item 8. This translation wasn't verified on live Discord; only checked via
  the compiler + 76 unit tests + clippy. AGENTS.md/docs/dev/ prose stayed Turkish, only code
  references (function/file/env variable names) were updated.
- **2026-09-03: multilingual infrastructure added (item 12), only `tr` filled in.** The
  `BOT_LANG`/`Lang` selection and prompt+string dispatch, and whether `langs/tr.json`'s ~160
  keys fit Discord's actual embed/button size limits (`modal.rs`'s `LABEL_LIMIT`/`FIELD_LIMIT`
  etc.), were confirmed live 2026-09-04 (see the note above; `en` confirmed the same day too).
- **2026-09-03: `en` (English) was filled in — `prompts/en/*.md` (31 files) and `langs/en.json`
  (158 keys), translated by hand.** JSON field names (the `Record`/`PersonRecord`/`TopicRecord`
  keys `olay`/`kisiler`/`isim`/`puan_degisimi`/`not`/`bilgiler`/`etiketler`/`konular`/`ad`/
  `kendim`) were left Turkish in the English prompts too — the Rust structs still expect these
  names, translating them would break JSON parsing. Slash command names were translated too
  (`durum→status`, `dusunme→thinking` etc.); command option values (things like
  `goster`/`gizle`/`ac`/`kapat`) weren't translated at all, only the visible labels. Generating
  an actual English reply with a real model, and the English text fitting Discord's
  embed/button size limits, were both confirmed live 2026-09-04 (see the note above).
- **2026-09-03: moved from `durum/` markdown files to `durum/hafiza.redb`.** This environment
  had no real production `durum/` data at the time (empty, never run live); the migration was
  first verified only by 85 unit tests + running `migrate-durum` against a hand-built made-up
  file tree and reading it back with `cargo run -- chat` (model.md came through correctly). The
  migration was later run against a real production `durum/` tree and confirmed live 2026-09-04
  (see the note above).
- **2026-09-03: the `resimler/` folder was moved to `photos/`** (code, `.gitignore`, docs
  updated); since it's an empty folder holding only `.gitkeep`, there's nothing extra to verify
  live.
- **2026-09-03: the `dev/` folder was merged into `docs/`, and all doc content (architecture,
  modules, flows, state files, prompts, constants, decisions, development, glossary, progress,
  roadmap) was translated from Turkish to English.** Filenames went English too (see the table
  in `docs/README.md`). This reverses item 8's earlier "AGENTS.md/docs/dev/ prose stayed
  Turkish" decision for these two folders — AGENTS.md/CLAUDE.md itself is still Turkish at that
  point, only the content under `docs/` is English. The translation was checked only by the
  compiler + clippy + tests (doc content doesn't affect live behavior); cross-references between
  files (`see docs/...`) were updated by hand, some may have been missed.
- **2026-09-04: AGENTS.md itself translated from Turkish to English**, at the user's explicit
  request (this reverses the "AGENTS.md ... stays Turkish" part of the note above; `docs/` was
  already English as of the previous day). CLAUDE.md's pointer text was updated to match.
  Content/meaning unchanged, translation only; checked by re-reading against the previous
  Turkish version, not by any live/build verification (this is prose, doesn't affect runtime
  behavior).
