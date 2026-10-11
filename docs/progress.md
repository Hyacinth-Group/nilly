# Progress log

Chronological. Newest at the top. Each line: date · commit (if any) · what+why · verification.

---

## 2026-10-11 · Step 0 Discord boundary characterization
Added offline fixtures around the current Discord boundary without introducing another
platform or changing the existing command/UI contract:
- Ingress fixtures in `src/bot/tests/tests_4.rs` capture bot/webhook/DM and guild/channel
  filters, exact Serenity safe content, display-name fallback, case-sensitive first-image
  MIME selection and analysis-off behavior, image markers, and custom-reaction labels.
- `src/command.rs` now captures all 16 registrations in order, their serialized option
  defaults/bounds, language-dependent names/labels, and unchanged Turkish choice values.
- `src/modal.rs` now captures the private status/help/mind/settings builders, the explicit
  flags-0 in-place settings refresh, button IDs/styles/disabled states, optional status
  token breakdown, and the 1024-character info-embed fitting/Turkish trim marker. The mind
  fixture reuses the existing shared one-shot test database initializer rather than creating
  a second database convention.
- `docs/architecture.md` records these current boundaries and quirks. Existing tests and
  assertions are left intact; the requested 93-test baseline remains the acceptance floor.

Verification: no cargo checks, tests, formatting, or live Discord calls were run by the
fixture workers. Final integration checks and commit are left to the owner; no new-platform,
memory migration, or transport-extraction completion is claimed.


## 2026-10-06 · serenity vendored
serenity 0.12.5's crates.io source copied to `vendor/serenity/` and wired in via
`[patch.crates-io]` in Cargo.toml, so it can be modified in-tree (user plans performance work).
Unmodified so far. Since it's now a path dependency, cargo shows serenity's own 3 deprecation
warnings on build (they were hidden as a registry dep); `cargo clippy` for this crate still 0.
Verified: build + 86 tests.

## 2026-10-06 · Stealth mode (`STEALTH` in .env)
New startup-only flag `Bot.stealth` (`STEALTH=on/1/evet/açık`, default off) so the bot passes as
a regular member. When on: `ready` makes no command API call at all (no registration, no
clearing — commands left on Discord from an earlier run stay until removed by hand); `interaction_create` returns immediately; no embeds go out (version
announcement in `guild_create`, `debug_note` — log only, `run_prank`'s "no images" notice);
`send_stream` forces Show/Hide thinking to Silent (no spoiler/counter/button) and skips live
edits, posting the final layout once the stream ends. Plain text, emoji reactions, images and
the welcome ping are unchanged. Verified: build + 86 tests + clippy; not yet run live.

## 2026-09-04 · First live test on the bot's real Discord server
User ran the bot against its actual Discord server (not a paste of logs — direct, hands-on
testing) and confirmed three of the long-standing "unverified" items from AGENTS.md's "Known
gaps" list are working as designed:
- **Basic messaging**: line-per-message bursting (each line of a reply arrives as its own
  Discord message), `-` silence, and `tepki:` emoji reactions (`create_reaction`) all behave as
  expected.
- **Slash command table** (`command::definitions()`): command registration, options, the
  defer+edit flow (`defer`/`report_result`), and embed-only output were all seen working in a
  real Discord client.
- **Stream + thinking**: the live message-edit cadence while a reply streams in, and the
  thinking display (show/hide/silent modes), were observed directly on Discord, not just via
  the unit-test fake SSE server.
- Updated accordingly: AGENTS.md's "Known gaps / unverified" section (these three items
  removed/trimmed), `docs/roadmap.md`'s "Remaining risk"/"Known risks" notes that referenced
  them, and the "Project status" paragraph in `README.md`/`README.tr.md`. Still open at this
  point: reaction rate-limit behavior under load, `send_lines`'s exact delay constants,
  willingness/target threshold tuning, `reaction_add`, CLI chat mode, English-language live
  generation, and every other item still listed in AGENTS.md's "Known gaps" section.

## 2026-09-04 · Second round of live testing — most remaining gaps closed
Same day, a further, broader round of hands-on testing against the bot's real Discord server.
Confirmed working as designed:
- gpt-4o-mini's and Mistral's image commentary (`image_commenter`).
- The reasoning-mandatory model (glm-5.3-flash) resilience path (budget×2, effort=low, JSON
  parsed out of the thinking), plus debug mode and the settings-panel buttons.
- Willingness/target/waking mini-calls, with the current thresholds (WILLINGNESS_THRESHOLD=6,
  interest≥5) holding up in practice.
- GUILD_ID/CHANNELS scope filters and conditional reply-to (`last_was_tagged`).
- Emoji-reaction rate-limit behavior under real use, and `send_lines`'s inter-line delay
  constants (300ms + 15ms/character, capped at 1500ms).
- CLI chat mode (`cargo run -- chat`) against a real model key.
- `Handler::reaction_add` and the `GUILD_MESSAGE_REACTIONS` intent (reactions left on the bot's
  own messages are picked up and evaluated).
- `BOT_LANG=en` end-to-end: an actual English reply generated by a real model, and
  `langs/en.json`'s ~160 keys rendering within Discord's embed/button size limits.
- The `supports_cache` openrouter assumption (cache_control silently ignored by models that
  don't support it).
- The `durum/` → `durum/hafiza.redb` migration (`migrate-durum`), run against a real production
  `durum/` tree rather than the hand-built test tree used earlier.
- Updated accordingly: AGENTS.md's "Known gaps / unverified" section (consolidated into the
  2026-09-04 note, individual now-resolved bullets removed) and the three 2026-09-03 dated notes
  that had referenced these as unverified (multilingual infra, English fill-in, redb migration).
  Remaining known-untested item: nickname changing when the bot lacks the CHANGE_NICKNAME
  permission on the server. Also untouched (not live-testable / not attempted): keyword
  stemming, holiday-date coverage past 2027, and person-file name-to-id resolution edge cases.

## 2026-09-04 · dev/+docs/ merged and translated, AGENTS.md/README.md rewritten, v1.0.0
User requests across the session: rename `resimler/` → `photos/`; merge `dev/` into `docs/`
and translate all doc content to English; translate `AGENTS.md`/`CLAUDE.md` to English; rewrite
`README.md` as a full technical document, plus a Turkish translation (`README.tr.md`); bump the
crate version to 1.0.0 for the 2026 release.
- `resimler/` renamed to `photos/` (code, `.gitignore`, docs); `.env.example` comments
  translated to English (commit `a64525b`).
- `dev/` merged into `docs/` as a single documentation home; all Turkish filenames renamed to
  English (`mimari.md`→`architecture.md`, `kararlar.md`→`decisions.md`, `ilerleme.md`→
  `progress.md`, etc.) and their full content translated Turkish→English, done via four
  parallel background agents (one of the four hit a session rate limit partway through
  `progress.md` and had to be relaunched). Runtime artifacts (durum/ field names, agent names,
  literal prompt/example text) deliberately left untranslated per AGENTS.md item 8. Stale
  content caught along the way and fixed in `docs/development.md`'s "not done" list: reaction-
  handling, id-based person keys, and call/token metrics were listed as missing but are already
  implemented (commit `de13a3f`).
- `AGENTS.md`/`CLAUDE.md` translated Turkish→English by hand at the user's request; also fixed
  a stale unit-test count (77→86) noticed while rewriting the quick-commands section (commit
  `3b117a4`).
- `README.md` rewritten from a dense bullet list into a structured technical document: badges,
  table of contents, architecture diagram, agent/memory-model tables, a full configuration
  table kept in sync with `.env.example` (added `LOG_LEVEL`/`LOG_COLOR`, previously undocumented
  there), a commands table, and a documentation map (commit `c744183`). A full Turkish
  translation added as `README.tr.md`, with a language-switcher line at the top of each file.
- `Cargo.toml` version bumped `0.2.0` → `1.0.0`. `Cargo.lock` was briefly untracked at the
  user's request, then kept committed once flagged that a binary crate (unlike a library)
  should ship a locked dependency tree for reproducible builds — user agreed, reverted.
- Verification: `cargo build`/`test`/`clippy --all-targets`/`fmt --check` clean after every
  step (86 tests). Doc/README content changes don't affect runtime behavior; none of this was
  exercised against a live Discord connection, same caveat as everything else in this
  environment.

## 2026-09-03 · English filled in: prompts/en/*.md (31) + langs/en.json (158 keys)
User: "add the English translations for the langs and markdown files too, get that going in the
project" — the multilingual infrastructure already existed (see the two entries below), only
`tr` was filled in; this round `en` was translated by hand.
- 31 prompt files (including `kisilik.md`, 182 lines) were translated by hand; examples specific
  to Turkish (slang escape patterns, the abbreviated-swearword example "aq/amk/mk") were not
  translated literally, they were replaced with English equivalents serving the same function.
  Identity (ITU physics) and political opinion (Erdoğan/AKP/TRT) content stayed the same, only
  the language changed.
  **Unchanged**: the JSON schemas of `gunlukcu.md`/`hedef-sec.md`/`isteklilik.md`/`ruh-hali.md`/
  `uyanis.md` — the field names (`olay`, `kisiler`, `isim`, `puan_degisimi`, `not`, `bilgiler`,
  `etiketler`, `konular`, `ad`, `kendim`, `hedef`, `puan`, `sebep`, `durum`, `yogunluk`,
  `ilgi`, `konu`, `kim`) were left in Turkish in the English prompt too — the `Record`/
  `PersonRecord`/`TopicRecord`/etc. structs in Rust still expect these names via `#[serde]`;
  if translated, the model could produce correct JSON and it would still fail to deserialize.
- `langs/en.json`: all 158 keys were translated (the key set and every value's `{placeholder}`
  set match tr.json one-to-one, verified with a script). Slash command names were also
  translated (`durum→status`, `yardim→help`, `zihin→mind`, `sifirla→reset`, `haber→news`,
  `sorun→problem`, `gez→wander`, `saka→prank`, `ajanlar→agents`, `uyan→wake`, `uyu→sleep`,
  `dusunme→thinking`; `hack`/`model`/`debug` stayed the same) — the VALUES of command OPTIONS
  (`goster`/`gizle`/`sessiz`/`kapat`, `ac`/`kapat`) were not translated at all, these are a fixed
  protocol matched by `ThinkingMode::from_arg`; only the visible labels are English.
- `src/lang.rs`: `Lang::En` added, `parse("en")` recognizes it. `src/prompts.rs`/`src/strings.rs`:
  an `en` submodule + `match` arm (same pattern as `Lang::Tr`).
- Test: because of the global `Lang::current()` shared by both languages in the same process
  during `cargo test`, I couldn't test `en` directly through `t()`/`prompts::current()` (whichever
  test runs first locks in that language) — instead, tests calling `prompts::get(Lang::En)` and
  `strings::table(Lang::En)` directly were added (bypassing the global). Real process-level
  verification: `BOT_LANG=en cargo run -- chat` was run, "language: En" landed in the log, no
  panic (all 31 files + the JSON compiled/parsed).
- Not verified: producing an actual English reply with a real model was never tried; whether the
  English text fits Discord embed/button size limits was not checked (the same gap already
  existed for `tr`). See AGENTS.md "Known risks".
- Verification: 86 tests (85→86), clippy 0 warnings, fmt clean.

## 2026-09-03 · durum/ markdown → redb (`durum/hafiza.redb`) + migrator
## 2026-09-03 · durum/ markdown → redb (`durum/hafiza.redb`) + migrator
User: "the bot should use redb instead of storing all its memory as markdown under durum" →
"also this redb migration needs a migrator" → "plan it out, it doesn't have to be redb, we'll
decide." First a full inventory of `durum/`+`memory.rs` was pulled (with an Explore agent), then
three options (redb/rusqlite/plain JSON) were presented to the user, redb was picked (rationale:
a new entry in docs/decisions.md — the project avoids C dependencies, rusqlite would have broken
that). The design approved in plan mode: don't change the data model, only change the container —
every redb value is byte-for-byte the same text the file had before the migration, and the key is
that file's old relative path.
- `src/memory.rs`: two redb tables `CONTENT`/`MODIFIED` (`static DB: OnceLock<Database>`, same
  pattern as `Lang`/`prompts`/`strings`); `path`/`files` removed, `read`/`write`/
  `append`/`add_topic`/`over_limit`/`person_summaries`/`topic_summaries`/`event_months`/
  `load_channel_history`/`refresh_index` moved to redb — signatures unchanged, `Person::parse`/
  `text`/`retrieve`/`keywords`/`trim`/`slug` untouched. `WRITE_LOCK` was dropped (redb orders its
  own writers internally). `durum/arsiv/` was deliberately left out, it's still a real file
  (protected by `ARCHIVE_LOCK`, `archive_append`).
- `agents.rs`'s `summarizer` was updated for `memory::over_limit()` now returning a `String` key
  instead of a `PathBuf` (the `STATE_DIR` strip_prefix line was dropped).
- `src/migrate.rs` (new): `cargo run -- migrate-durum [--from] [--to] [--dry-run] [--force]`.
  Scans the old tree excluding `arsiv/`, carries the real OS mtime over via
  `memory::write_with_mtime` (so everyone's "most recently changed" order doesn't reset on
  migration day), refuses if the target is non-empty and `--force` wasn't given, never touches
  the original files.
- A race condition was caught during testing: `memory.rs`'s test helper was opening the DB with
  a `DB.get().is_none()` check, and in cargo test's parallel threads two tests could pass that
  check at the same time and both try to open the same redb file, one of them blowing up with
  "memory::init wasn't called" — fixed with `std::sync::Once`, verified with 5 consecutive
  `cargo test` runs.
- Manual end-to-end verification: with the real binary, a made-up `durum/` tree was built
  (person+topic+monthly events+model.md+profil.md+arsiv), then `migrate-durum --dry-run` → real
  run → re-run without force (rejected) → re-run with `--force` (accepted) → opened with
  `cargo run -- chat`, `model: z-ai/glm-5.3-flash` landed in the log (the fixture's model.md was
  read correctly) — proof the post-migration read path works end to end.
- There's no real production `durum/` data in this environment (empty, outside git); this was
  never tried against a real tree, see AGENTS.md "Known risks".
- Verification: 85 tests (82→85), clippy 0 warnings, fmt clean, the manual end-to-end check above.

## 2026-09-03 · Multilingual infrastructure: BOT_LANG, prompts/<lang>/, langs/<lang>.json
## 2026-09-03 · Multilingual infrastructure: BOT_LANG, prompts/<lang>/, langs/<lang>.json
User: "let's pick a language in .env and choose from under prompts/<lang> based on it, there
should be a language system inside the slash commands too, let's write localization files into a
langs folder" → in a clarification round (infrastructure ready, only tr filled in / command
name+description+embed+button all covered / a single global .env language, not native Discord
locale) → along the way two more corrections: "langs/ should be json not .md" and "call the
prompts folder prompts".
- `src/lang.rs`: `Lang` enum (only `Tr` for now), `Lang::current()` — reads `BOT_LANG` on first
  call, fixes it for the life of the process with `OnceLock` (`BOT_LANG` not `LANG`: shells
  already use `LANG` for the OS locale, to avoid clashing).
- `prompts/<name>.md` (31 files) → `prompts/tr/<name>.md`; `src/prompts.rs` is now
  `mod tr { include_str! }` + a `Prompts` struct + `prompts::current()` — ~40 call sites
  (`agents.rs`, `agenda.rs`, `src/bot/**`) moved from standalone `PROMPT_ADI` constants to
  `prompts::current().field` (thanks to the global `OnceLock`, none of them had to carry
  `self.lang` around).
- `langs/tr.json` (158 keys, flat `{"key":"value"}`) + `src/strings.rs`'s `strings::t(key)`:
  the slash command table (name/description/option/choice — the **value** of command options is
  never translated, only the visible label), the `HELP` text (used to be a Rust constant in
  `command.rs`, now `help.text`), all of `modal.rs`'s embed titles/field labels/button text/month
  names, the response embeds of `command/{actions,settings,cards}.rs`, a handful of event
  announcements (`geldim`, the debug trace title, "photos folder is empty"). What was
  deliberately left untranslated: `ThinkingMode::label()`'s descriptive form of "hidden"/"off",
  the state vocabulary of `growth`/`sleep`/`travel` — these are the bot's own state vocabulary,
  not the Discord shell; it's reasonable for them to stay a separate layer until a second
  language is added.
- `langs/tr.json` is parsed with `serde_json` (already a dependency, no new dependency added).
- Verification: 82 tests (81→82, new `lang`/`prompts`/`strings` tests), clippy 0 warnings, fmt
  clean. Never seen on live Discord — see AGENTS.md "Known risks" (whether the size limits are
  respected in a second language still needs separate checking).

## 2026-09-02 · Live bugs: kirp() off-by-one, geldim/debug embed, reasoning learning
The first live log round. Three separate fixes (each in its own commit, details in the commit
messages):
- `hafiza::kirp` was overshooting the limit by 1 when appending "…" → the `/zihin` person menu was
  getting rejected outright when it exceeded Discord's description limit of 100
  ("Must be 100 or fewer in length"). Fixed + test.
- Emin: "the geldim message should be an embed too, everything except the actual conversation
  should be an embed" — the version announcement, debug traces, and the "photos folder is empty"
  error were turned into embeds.
- Emin, from a live log: the reasoning-mandatory model was repeating the same wasted "turn off"
  attempt every turn → now learned once via `Bot.reasoning_zorunlu_modeller` and not forgotten.
- Verification: 76 tests, clippy 0 warnings, fmt clean.

## 2026-09-02 · main.rs + komut.rs split into ~50 files (the 200-line rule), IMAGE_ANALYSIS added
Continuation of the previous entry, same session. Emin: "you can move the functions in main.rs
into a separate folder and split related ones file by file" → "split the commands into separate
files too" → "it'd be better if no file were longer than 200 lines". Details and technical
constraints (impl-block splitting limits, E0119): `docs/decisions.md`.
- Under `src/bot/` and `src/komut/`, ~50 small files via `include!` (not a real `mod`); 3 of them
  exceed 200 lines out of structural necessity (`handler_event.rs` 423 — a single trait impl
  requirement, `sohbet_cevapla.rs` 261 — a single function, `testler_3.rs` 204 — test grouping).
- **`RESIM_ANALIZI` (.env)**: Emin said "photo scanning should be togglable via .env and no longer
  changeable by command." `Bot.resim_analizi: bool`, read only in `Bot::kur()`, no command writes
  it; while off, the `message` handler never even looks at attached images.
- A general token/performance sweep was done (Emin: "optimize token usage"), no concrete
  bottleneck was found (details in decisions.md) — no change was made.
- In this round the user was bothered by the pace ("you work for 5 min and spend 20 min writing
  notes"); intermediate `cargo build` calls were stopped, verification was done only once at the
  end of the phase (`cargo check`/`test`/`clippy`/`fmt`) — see
  `~/.claude/.../memory/feedback_build_cadence.md`.
- Verification: 75 tests, clippy 0 warnings, fmt clean.

## 2026-09-02 · Panel image abandoned, the bot fully moved to slash commands
Emin, in sequence: "how does this zihin command generate the image, with AI?" (answer: no, hand-
built SVG+resvg) → "instead of that, make it an embed, it looks bad, but make the embed proper,
with a button for parts that don't fit that opens a modal for the user" → "put together a command
manager and move all commands under it, and have every command return embed output instead of
plain text" → "fully disable the bang commands, the bot should run only on slash commands." Plan
approval: delete the old PNG code entirely, move all remaining `!` commands to slash (the
text-only/reaction-only distinction doesn't exist in slash anyway, every interaction is forced to
have a response).
- **`zihin_gorsel.rs` deleted entirely** (SVG drawing, the embedded Inter fonts in `fonts/`, the
  `resvg` dependency from `Cargo.toml`, the `cargo run -- zihin` CLI, 6 tests). `/zihin` already
  had embed+button+select+modal (`modal::zihin_embedleri/zihin_bilesenleri`); that became the only
  path.
- **`Bot::komut` (one big `match`) and the `!`/`/` text-capture block in `Handler::message` were
  removed.** Replaced with the `komut::KomutTanimi` registration table (`src/komut.rs`): name,
  description, Discord options (`CreateCommandOption`), and executor (`fn(&Bot,&Context,
  &CommandInteraction) -> Pin<Box<dyn Future<...>+Send>>` via the `komut_gir!` macro) all in one
  place. `modal::komutlari_kayit` (registration) and `interaction_create` (dispatch) both read
  from the same table.
- 12 old `!` commands (`sifirla, haber, sorun, gez, saka, hack, ajanlar, uyan, uyu, dusunme, model,
  debug`) moved to slash with identical functional counterparts; `zihin test` became `/zihin`'s
  `test:Boolean` option. Discord wants an initial response within 3 s: commands that make
  network/model calls (`haber/sorun/gez/saka/hack/ajanlar/uyan/uyu/zihin test/model id change`)
  immediately acknowledge via `ertele` (`Defer`) and edit in the result with `sonucu_bildir`
  (`edit_response`); the actual content already went to the channel via `Bot::gonder`, unchanged.
- Every command that needs a text reply now returns via `modal::bilgi_embed` as an embed (no plain
  `content` anymore). The now-unused `modal::durum_metni` and `Bot::gonder_ekli` were deleted.
- **The first build hit AGENTS.md rule 1**: expressions like `modal::durum_mesaji(&bot.durum())`
  were carrying a `MutexGuard` across `.await` (the `komut_gir!` future needs to be `Send`); fixed
  by pulling it into its own `let` line, `let yanit = ...; yanit_gonder(..., yanit).await;` (the
  guard drops at the `;`).
- Docs sync: AGENTS.md (quick commands, rule 11 added, known risks), docs/moduller.md,
  docs/akislar.md, docs/durum-dosyalari.md, docs/sabitler.md, docs/mimari.md (command.rs/modal.rs
  added to the file map), two new decisions added to docs/kararlar.md (append-only, the old
  resvg/panel decisions weren't deleted — the rationale at the time still holds, this just adds a
  new one on top).
- Verification: 75 tests (79 − 6 zihin_gorsel + 3 command-table tests − 1
  `durum_metni_sayac_tasir`, since `durum_metni` is gone), clippy 0 warnings, fmt clean. **Not
  verified**: not a single slash command (including the new 12) has ever been seen live on
  Discord.
- Phase 3 (splitting main.rs into topic-based files under `src/bot/`) was in the plan but wasn't
  done in this session yet — logged as an open item in dev/yol-haritasi.md.

## 2026-09-02 · Reasoning resilience, !zihin test, debug mode, settings panel, mind-image fixes
- `sor_ham`: on mandatory-reasoning retry, budget max(2×, 1500) + openrouter `reasoning.effort=low`;
  when content is empty in categories expecting JSON, the JSON content in the thinking field
  (`yanit_icerigi`); error message includes category/model/budget/thinking length. `gunlukcu` →
  `Result<GunlukcuOzet>`, info logs across the mind chain.
- `!zihin test`: feeds the last 30 lines straight to günlükçü and writes the result.
- `!debug [aç|kapat]` + `Durum.debug` (debug.md) + `debug_not`: willingness score/reason
  (`isteklilik_coz`), target, mood, question ceiling, silence/reaction/line traces; DEBUG_KANALI.
- `!ayarlar` / `/ayarlar`: a button-based panel (thinking mode, debug, wake/sleep), `ayar_dugmesi`
  refreshes it in place; shared with the `uyandir/uyut` commands.
- Mind-image review fixes: letter buckets set to the ceiling, PNG attached from memory
  (`gonder_ekli`), mood made deterministic.
- Verification: 79 tests, clippy 0 warnings, release build.

## 2026-09-02 · Version info: !durum + startup announcement
- `build.rs` (new): `SURUM_COMMIT` (git rev-parse --short HEAD, `+` suffix if dirty) and
  `SURUM_TARIH` env vars; `surum_metni()` in main.rs. `!durum`/`/durum` shows
  "sürüm: v0.2.0 (69e2851, 2026-09-02)".
- `guild_create`: once per process, posts "geldim · version · model · thinking" to the default
  channel (`Handler.duyuruldu`); doesn't enter the channel note.
- Verification: 71 tests, clippy 0 warnings.

## 2026-09-02 · Review round: protocol fixes (finishing the line-based assumptions)
All the high/medium findings from the gate + three reviewer reports were applied. They all had
the same root: assumptions that held when a reply was a single message broke under the line-based
protocol.
- **`gonder_cevap` split out** (the body of `gonder_satirlar`, takes a `Cevap`): if there's no
  reaction target, the reaction gets dropped; if there's really nothing to send, it returns
  `None`. Previously, an opening that was just "tepki: 💀" sent nothing to Discord, yet the chat
  would still open and the timeout counter would start.
- **The welcome ping** is no longer glued onto the text upfront, it's added to the first line at
  send time: `<@id> -` was hiding the silence marker, `<@id> tepki: 💀` was hiding the reaction
  line.
- **`gonder_akis`**: the `-` + `tepki: 💀` combo no longer counts as silence (the emoji is
  dropped); the `ilk` flag is only consumed once something is actually written (it used to run out
  while the layout was still empty, delaying the first paint by 1-2 s); on regeneration after a
  repeat, a reaction-only reply isn't counted as "empty"; the transcript is written in a single
  file write (`kanal_not_coklu`, previously there were 4-5 full writes per turn).
- **`sohbet_baslat`'s dedup is now line-based**: since the opening drops into history line by line,
  exact-equality never held, and the opening ended up showing to the model twice (in the news
  path, a link message also gets interleaved).
- **`cevapla`**: on the fallback `uret` branch, repeat filtering is line-based and fed to
  `gonder_cevap`; in the `Sus` branch, the `hackli` counter also decrements (the hack joke used to
  get stuck when it went silent); `ruh_hali_belirle` sends an image-free copy of the history.
- **Command detection on raw text**: in a message with an image the text was `[resim] !durum`, so
  commands like `!durum`, `!saka`, `/haber` were being silently swallowed.
- **`dokum` prefixes every bot line with a name**: in a multi-line reply, the 2nd and later lines
  carried no prefix, and eleştirmen/günlükçü/hoca were counting them as belonging to people in the
  group.
- **`emoji_ayikla` narrowed** (`emoji_basi`/`emoji_devami`): `—`, `…`, `→`, typographic quotes were
  being counted as emoji and sent to Discord, and the request came back 400.
- **`slop_temizle`**: the numeric-prefix strip moved to `cevap_parcala` and only applies to an
  actual list (≥2 numbered lines) — "3. sınıftayım" was turning into "sınıftayım"; `**`/`__`
  stripping no longer reaches inside backticks (`` `__init__` `` was getting mangled). Also in this
  round, a literally-repeated line back to back no longer goes out a second time.
- **Prompts**: `kisilik.md` — "if others cut in" → "if other messages cut in" (the code condition
  is `bekleyenler.len() > 1`), a `:kekw:` warning, MUHABBET got "silence isn't a cop-out",
  KANDIRILMAZSIN's line "you say when you don't know" changed to "you don't make things up when
  you don't know" (two separate sections were both banning "I don't know"). `elestirmen.md` got an
  explanation of multi-line bot replies and the reaction line.
- **Docs**: `moduller.md` (that trim's 1900 cap applies to the WHOLE reply on the non-stream path,
  that `tepki_hedefi` and `yanit` can diverge, new/changed functions), `akislar.md` (the first
  delta, the claim about the CLI writing to disk), `README.md`/`AGENTS.md` ("4 messages" → "4
  lines", CLI folder creation), `promptlar.md` (the claim about the example reply), `mimari.md`
  (main.rs ~4200), `kararlar.md` (why the emoji-whitelist counter-proposal wasn't taken + this
  round's decisions).
- Verification: `cargo fmt --check` clean · `cargo clippy --all-targets` **0 warnings** ·
  `cargo test` **70 passed, 0 failed** (previously 65) · `cargo build --release` succeeded.
- **Not applied:** the YASAK KALIPLAR fix from "repeating back to back in the same message" to
  "in the same turn"; that section is listed in the spec's acceptance bar as "not a single line
  should change." Growing `SOHBET_TOHUM`/`KANAL_GECMIS` in proportion to line inflation also
  wasn't done (a fixed setting, needs live measurement).

## 2026-09-02 · "React like a normal human": output protocol, silence, reaction, image, CLI
Emin's request: "it should be able to react like a normal person while chatting; let's remove the
limits in the personality." Mechanical limits that were lifted: "one thought per message", "no
emoji/bullet points/paragraphs," the "two-three sentences" ceiling in NASIL YAZARSIN, the
single-message requirement, the inability to react, the inability to see images, the requirement
to reply to every message in an open dialogue. (The lines "if he asks something technical you
answer in two sentences" and "if he's venting you listen for two sentences" in NE YAPMAZSIN were
deliberately LEFT ALONE: those sections were treated as untouchable, so the sentence ceiling
hasn't fully disappeared.) What was left untouched: the SINIRLAR (server rules), KANDIRILMAZSIN,
YASAK KALIPLAR, NE YAPMAZSIN, TAKINTILARIN, RUH HALİN, KİMLİĞİN, LAF SOKULUNCA, İSTEK GELİNCE,
İNSANLARA KARŞI TAVRIN sections of `kisilik.md`.
- **Output protocol (`Cevap`, `cevap_parcala`)**: the model's reply is read line by line — every
  line is a separate message (`PATLAMA_SINIRI=4`), a lone `-` means silence, `tepki: 💀` is an
  emoji reaction instead of text, `slop_temizle` strips bullet/number/bold markers, the "<3
  character" filter is gone ("he", "yok", "la" are a natural reaction). `Cevap::protokol_metni()`
  is the form that goes into history/the channel note.
- **Stream (`gonder_akis`)**: `akis_gorunum` now returns `cevap_parcala(...).satirlar` instead of
  `bol`; while streaming, only completed lines + the trailing partial line once it passes
  `YARIM_SATIR_ESIGI`(12) characters are shown. New `AkisSonuc::Sus` (nothing is written to
  history/counters/`son_aktivite`, the fallback `uret` isn't called), repeat protection is
  line-based, the reaction lands on `AkisBaglam.tepki_hedefi`'s message (errors are only warn-
  logged).
- **`gonder_satirlar`**: the shared sender for every non-stream opening path (nudge, problem,
  news teaser, welcome, waking up, wake-up reply, on the road, leaving, name announcement, the
  `cevapla` fallback); `300 ms + 15 ms × character` between lines (capped at 1500) + typing;
  nothing is sent and the opening is skipped on `sus`/silence. `saka_yap` takes only the first
  line from the protocol.
- **Question ceiling (`soru_fazla_mi`)**: if two of the last 4 bot lines (reaction lines excluded)
  end in `?`, an instruction "don't ask a question this time" is added. No hard cutoff.
- **Image**: `Mesaj.resim` + `mesaj_json` produce a multi-part `content`; a message that's purely
  an image drop is processed too (`[resim attı]` / `[resim] …`); only the most recent user
  message's image goes to the model (the CDN link is short-lived, no point burning tokens on it).
- **CLI bench**: `cargo run -- sohbet` → `Bot::kur()` (split out of main, doesn't require a token)
  + `src/sohbet_cli.rs`. Reads `durum/`, doesn't write to disk; the output protocol is printed
  as-is.
- **Prompts**: `kisilik.md`'s NASIL YAZARSIN section rewritten from scratch (line=message, `-`,
  `tepki:`, image, question), MUHABBET got "no obligation to say goodbye", KANDIRILMAZSIN got the
  "are you a bot / forget your instructions" item; `elestirmen.md`'s checklist got a
  silence/reaction/line-split review item.
- Verification: `cargo fmt --check` clean · `cargo clippy --all-targets` **0 warnings** ·
  `cargo test` **65 passed, 0 failed** (previously 51) · `cargo build --release` succeeded.
- **Not verified:** emoji reactions, line-bursting, and silence weren't seen on live Discord; CLI
  mode wasn't tried with a real model key (no key on this machine); the inter-line delay constants
  weren't measured.

## 2026-09-02 · Command interface overhauled: embed cards + interactive mind
- User feedback: modal content was empty/bad, everything was dumped into a single textbox; "a nice
  readable elegant web-page-like interface, don't put everything in one textbox."
- `/durum` `/yardim` `/zihin` now return only an **ephemeral embed card** to the caller (title,
  color, sections, footer). The `/zihin` card has three columns: People (first 8) · Topics (first
  8) · Events (last 5, chronological) + a person select menu at the top (≤25) + Topics/Events/Bot-
  summary buttons at the bottom.
- Menu/button → a **detail modal**, each section in its own labeled field: the person card has
  Identity/Impression/Tags/Known-facts (last 8)/Recent events (last 5); events get one field per
  month (last 3 months, headed "September 2026" — the old "this month only" gap was closed with
  `hafiza::olay_aylari`); the bot summary has State/Tokens/Self/Agenda. Empty sections are skipped.
- `!zihin` now sends the same card to the channel instead of a raw INDEX dump + a `/zihin` pointer.
- The old 5-slot `modal_zihin`/`bolumler` were removed; `olay_dokumu` gave way to `olay_aylari`.
- Verification: 52 tests (5 new: sigdir, ay_adi, section filter, durum_metni, split order), clippy
  0 warnings, fmt clean, release build.

## 2026-09-02 · Second remote-branch merge (PR #3+#4, identity alignment)
- What came in from remote was taken as-is: `DusunmeKip::Sessiz` (a 4th mode: thinks in the
  background, no trace at all), `reasoning_kapat(herhalukarda)` (background agents turn off
  reasoning regardless of the mode — small budgets were going into thinking and returning
  `content: null`), `REASONING_ZORUNLU_TABAN=500` + raising the budget and retrying on an empty
  reply, `kisilik.md`'s server-rules alignment (no encouraging harassment/insults, the SINIRLAR
  section) + the ITU physics identity, haber-sec consistency.
- Conflicts were only in docs (ilerleme.md, kararlar.md) — merged chronologically; main.rs/
  komut.rs/moduller.md merged cleanly on their own. The local hot-path cleanup and the
  `cevap_ver = acik && katil` fix were kept.
- Verification: 51 tests, clippy 0 warnings, fmt clean.

## 2026-09-02 · PR #2 merge + fixes + hot-path allocation cleanup
- A remote PR (token optimization, multi-provider generality, discussion behavior, prod-readiness:
  category metrics, the reasoning-mandatory fallback, cache generality, mood, GUILD_ID/CHANNELS,
  taranan.md) was merged with local. Conflicts resolved by hand: the watchdog is a single function
  (`dongu_bekle` + KAPANIYOR + a 5 s sleep in every branch), `hafiza::yaz` uses the local body
  (lock + fixed `.tmp`; the remote's pid+counter-allocated name wasn't taken), progress
  chronology.
- Fixes: `cevap_ver = acik && katil` (in an open chat, the willingness result was being thrown
  away — the PR's main promise had been left half-finished); `devam_eden_diyalog` compares names
  with `kucult` (Turkish İ); `ruh_hali`'s redundant `sayac == 0 ||` condition dropped;
  `CEVAP_TAVANI` 3000 → 4096 (reasoning tokens count against the budget, a narrow ceiling would
  truncate a long thought).
- Hot-path allocation cleanup: `soy` returns a slice (no more full-text clone on every edit during
  streaming), `bol`/`temizle` no intermediate collect, `kanal_not`/`son_mesajlar`/`dokum` no
  `Vec`, `getir`'s budget counter + topic read in one pass, `dizin_yenile`'s lightweight title
  parser, `konu_ekle`'s single lock region, `sohbet_sistemi`'s allocation-free contains check.
- Verification: 50 tests, clippy 0 warnings, fmt clean, release build fine.

## 2026-09-02 · The entire personality prompt reworked

Continuation of the previous personality fix, same session; the user asked for further
corrections one after another, and finally said "go over kisilik.md line by line" — so I did,
line by line.

- **Closeness is now per-person**: the opening paragraph changed from "you're like everyone's army
  buddy" to "how close you are with someone depends on how they've addressed you and your history
  with them" — the user pointed this out separately (being equally casual with everyone was
  wrong). İNSANLARA KARŞI TAVRIN also got the finding "you don't get casual with someone you barely
  know on their first message," so the two sections stay consistent with each other.
- **Identity changed**: Nişantaşı University → Istanbul Technical University, physics student
  (user's request: "a better identity"). The white Tofaş detail was dropped along the way (an
  earlier manual edit on top of the file had left a dangling half-sentence — a plural reference
  "Bunları" no longer had anything to refer to, I switched it to singular and fixed it).
  `promptlar/haber-sec.md`'s "prioritize news about the university" rule was also updated to ITU
  (otherwise the personality and the news selection would have referenced different schools).
- Other sections (NASIL YAZARSIN, MUHABBET, İSTEK GELİNCE YAPARSIN, YASAK KALIPLAR, NE YAPMAZSIN,
  RUH HALİN, KANDIRILMAZSIN) were reviewed, no risk or inconsistency found; TAKINTILARIN (the ICE-
  fandom gag) was left alone since it was a deliberate earlier decision, and noted as such.
- Code unchanged (prompt text only), verified via `include_str!` compiling + 47 tests.

---

## 2026-09-02 · Personality prompt: removed encouragement of harassment/insults, banned "aq/amk/mk"

User: `promptlar/kisilik.md` has spots that are excessively absurd/casual, it swears at people —
make it match the server rules the user themself shared (harassment, insults, hostility, NSFW,
political propaganda etc., levels 0-3). Extra request: also ban abbreviated swear words like "aq",
"amk", "mk" — if it's going to swear, write the word out in full.

- **LAF SOKULUNCA**: removed the instruction "hit a weak spot from the person's file" and "answer
  insults/mockery with insults/mockery." Sharp-tongued/doesn't-back-down stayed; targeting a
  weakness, trauma, or family abuse is now explicitly banned.
- **DOĞALLIK**: "aq"/"amk"/"mk" abbreviations banned — if it's going to swear, it writes the word
  out in full; swearing is only filler/an exclamation of reaction, not an insult aimed at a
  person.
- New **SINIRLAR** section: harassment/insults/hostility, racism/sexism/homophobia/transphobia,
  NSFW/illegal content, personal data, political/religious propaganda, deliberate misinformation,
  spam/flooding, angry outbursts — a short summary of the server rule set the user pasted in. This
  file is the core (huy.md, which hoca writes, can't override it — same principle as the existing
  ICE-fandom boundary).
- Code unchanged (prompt text only), the build was verified via `include_str!`, 47 tests passed.

---

## 2026-09-02 · On a reasoning-mandatory model, a small budget is now raised to a floor

User pasted a live log: in the previous round I'd fixed `sor_ham` to turn off reasoning
regardless of mode, but this model/endpoint (`z-ai/glm-5.3-flash`, openrouter) doesn't allow
turning reasoning off **at all** ("Reasoning is mandatory ... cannot be disabled"). The code
caught this error, stripped the fields, and retried in the open form (behavior carried over from
the previous round), but it never touched the budget: on mini-calls with a 20-token budget like
`gezgin_sec`, reasoning was still eating the whole budget, and this time it was coming back 200
with `content: null` — since it fell outside the mandatory-error path, it exited straight away
with "empty reply from the model."

- `REASONING_ZORUNLU_TABAN=500` + `Bot::butce_tabanini_uygula(govde, taban)`: raises `max_tokens`
  if it's present and under the floor, doesn't touch a budgetless call.
- `sor_ham`: on the mandatory-reasoning retry, the budget is raised to the floor. Also, getting a
  200 back with empty/null content is no longer an instant error — the budget is raised to the
  floor and it's tried once more (`AI_YENIDEN_DENEME` gives up once exhausted).
- `sor_ham_akis`: the same budget floor applies on the mandatory-reasoning branch (there's no
  empty-content retry on the stream side, `gonder_akis` already handles a short/empty reply
  separately).
- Verification: 47 tests (new `butce_taban_altindaysa_yukselir`), clippy 0 warnings, `cargo fmt`,
  debug build.

---

## 2026-09-02 · A "silent" thinking mode added (4th mode)

User's request: even in "hide" mode it prints "X words thought" while thinking, they want one
that shows nothing at all — not even a button — but genuinely thinks in the background.

- `DusunmeKip` gained a 4th variant, `Sessiz`. Since it's not counted as Kapali in
  `reasoning_kapat`, reasoning is requested normally in the stream request (not turned off, the
  model genuinely thinks).
- `gonder_akis`: thinking is only accumulated in Goster/Gizle modes; in Sessiz it's never
  collected at all.
- `akis_gorunum`: during the thinking phase (reply empty), Sessiz also returns an empty vector
  like Kapali — the message never opens until the reply starts. Once the reply starts, only the
  reply goes out, no button is added (the `kip == Gizle` condition already excludes Sessiz).
- `!düşünme` help text and command output updated: `göster/gizle/sessiz/kapat`.
- Verification: 46 tests (2 new asserts extended for `Sessiz`), clippy 0 warnings, `cargo fmt`,
  debug build.

---

## 2026-09-02 · The silent "empty reply" error in background agents fixed

The possibility flagged in the previous round as "not something fully fixable on the code side"
came true: in live logs, profilci/hoca/günlükçü/gezgin were repeatedly throwing "empty reply from
the model," which is why `kisiler/konular/olaylar` had stayed empty.

- **Root cause**: `reasoning_kapat` only turned reasoning off when the user's global thinking mode
  was `Kapali`. It wasn't turned off in `gizle` mode — but `sor_ham` (the non-stream path used by
  background agents) never reads/shows the `reasoning_content` field at all anyway. The reasoning-
  mandatory model (glm-5.3-flash) was spending these agents' small `max_tokens` budgets (20-1200)
  entirely on thinking and returning `content: null`.
- **Fix**: `reasoning_kapat` now takes a `herhalukarda: bool` parameter. `sor_ham` (background
  agents + non-stream chat) now always passes `true`, turning off reasoning regardless of the
  user's mode. `sor_ham_akis` (stream, chat) still looks at the user's mode (`false`) because
  there `gizle`/`göster` genuinely correspond to something actually shown (the counter/full text).
- Verification: 46 tests, clippy 0 warnings, `cargo fmt`, debug build.

---

## 2026-09-02 · Memory hardening + cycle watchdog + scan order
- `hafiza::yaz` is now atomic (temp file + rename) + serialized through `YAZMA_KILIDI`; `ekle` is a
  real append (the whole file isn't rewritten). Test: disk round-trip + no leftover temp file (43
  tests).
- If günlükçü's JSON can't be parsed, the raw output is salvaged to `arsiv/gunlukcu-<kaynak>.md`.
- `dongu_bekle`: all six cycles now start under a watchdog, on panic it logs + retries after 5 s;
  graceful shutdown via `KAPANIYOR` (AtomicBool) — cycles return at the top of their tick, the
  watchdog doesn't restart them then.
- Expired news chats are cleared on the minute tick (`zaman_asimi_kapat`), `haber_bekleyen` no
  longer bloats.
- The startup scan is prepended ahead of memory: live messages arriving while the scan is running
  aren't overwritten, chronology is preserved; `ad_id` only fills in when empty (live mapping takes
  priority).
- Verification: 43 tests, clippy 0 warnings, fmt clean.

## 2026-09-02 · HTTP timeout architecture + mechanical hardening (d8d7fc8)
- The global 60 s timeout is gone: `connect_timeout` 15 s + `read_timeout` 120 s (reset on every
  read → covers the first token); no cap on total duration, a long thinking stream is no longer
  cut off.
- Retry on transient errors (network, 429, 500/502/503/504): 2 s and 4 s backoff, at most 2 extra
  attempts (`sor_ham` + `sor_ham_akis`; the stream only before it opens).
- `reasoning_kapat` is now provider-specific: openrouter uses `reasoning.enabled`, mistral gets no
  parameter, others get `enable_thinking:false` (sending both at once broke some providers).
- `MesgulGuard` (RAII): the channel's busy flag is released on every exit including a panic; 8
  scattered `remove` calls collapsed into one guard.
- `soy` is now char-safe (no byte slicing) + `kucult` (İ→i̇ composed dot) — 4 new tests.
- Typing pulled out of the edit loop (`yaz_akis`); done once before the model call.
- Verification: 42 tests, clippy 0 warnings.

---

## 2026-09-02 · First live logs: two real production bugs
User pasted a real log from the live bot (`z-ai/glm-5.3-flash`, openrouter). Two separate bugs:

- **400 "Reasoning is mandatory ... cannot be disabled"**: the `reasoning:{enabled:false}` sent
  under "thinking off" mode was being rejected by this GLM variant, and the bot couldn't reply in
  that channel at all. `reasoning_kapat` now returns `true` once it has added the fields;
  `sor_ham`/`sor_ham_akis` now recognize this specific error (`reasoning_zorunlu_hatasi`), strip
  the fields, and retry once more. Note: for the same model's small-`max_tokens` mini-calls
  (haber_sec=10, hedef_sec/ruh_hali=40, isteklilik=80) there's still a risk of hidden reasoning
  eating the budget and producing an empty reply (an "empty reply" was also seen in a profilci
  log) — this isn't something fully fixable on the code side, reasoning-mandatory models are
  fundamentally in tension with this architecture (many small-budget mini-calls).
- **"replies to every message" (the actual discord complaint)**: on inspection, `acik` (is chat
  open in this channel) alone was found to mean "no need to evaluate, just reply directly" — once
  a chat was open, EVERYONE's message in the channel was being replied to directly regardless of
  who it was addressed to (this is a completely separate mechanism from the reply-to/tag issue I
  fixed in the previous round). `devam_eden_diyalog` was added: if the sender of the last user
  message in the chat is the same as this message's sender (genuinely talking to themself), it
  auto-continues; if someone else wrote, it still goes through the willingness evaluation (same 2
  min rate limit, no extra cost).
- Verification: 43 tests, clippy 0 warnings, `cargo fmt`, release build. The new logic in the
  `message` handler (inline, not a pure function) couldn't be verified with a unit test — needs
  watching live.

---

## 2026-09-02 · Mood agent + second resilience round
Continuation of the previous round: "start both" (mood agent + backlog).

- **Mood agent**: `promptlar/ruh-hali.md`, `Bot::ruh_hali_belirle`, `Sohbet.ruh_hali`. A cheap
  mini-call when the chat opens and every 4 turns (not every message); it picks a single
  state+intensity from the taxonomy, intensity <3 counts as neutral. Added to the instructions as
  "YOUR CURRENT MOOD"; the personality prompt has a rule: "don't announce it, let it color your
  tone."
- **`hafiza::yaz` atomic** (temp file + rename) — no half-written file on crash/kill.
- **`dongu_bekci`**: the 6 background cycles now log and restart after 5 s on panic (previously
  they silently stopped running forever).
- **`soy` made char-safe**: `.chars().skip(n)` instead of `onek.len()` (bytes, from lowercase) —
  there was a panic risk on letters like Turkish capital İ (lowercasing changes byte length).
- **`durum/huy.md`'s sleep theme cleaned up + `hoca.md` corrected**: user complaint "I ran !uyan
  but it still says it's tired/sleepy" — lines in huy.md where hoca had mistaken the frequent
  `!uyan` chatter during testing for a permanent TAVIR ("sleepy," "slept my ass off," "sick of
  being woken up") had nothing to do with the bot's real sleep system, the mere word overlap was
  confusing things. Also the DOĞALLIK section was working backwards, forcing a fixed line instead
  of ("a pattern to avoid") — a KALIPLAR invention. The prompt was fixed, the existing file cleaned
  by hand — but the `durum/huy.md` in this repo may not be the file the user's LIVE bot is using;
  if so they need to make the same fix there too (deleting and restarting the file, or manually
  removing the lines).
- The TOON assessment was put to the user again, same result: not worth a general migration, the
  only real candidate is the index (INDEX.md), not implemented yet.
- Verification: a single fmt+clippy+test+release at the end of this round (user's request:
  "leave the compile check for the very end," instead of building on and off).

**Left (deliberately deferred)**: making `reasoning_kapat` conditional on the target address (low
priority, whether Mistral rejects an unknown extra field wasn't verified) · serializing agent
writes to one queue · günlükçü JSON-error raw-dump recovery · archive append · graceful shutdown
(watch) · channel-based wake-up · cleanup of expired news chats · scan order · pulling typing out
of the edit loop · error classification+retry.

---

## 2026-09-02 · Token optimization + prod-readiness sweep
User's request: "the project is spending way too many tokens, we need to provide an optimization
engine" + afterward, prod-readiness, multi-provider generality, discussion/reply-to behavior,
a channel-history scan bug, and scope-narrowing requests all came in the same session. All in one
package:

- **isteklilik/hedef_sec moved to the cache**: previously `analiz()` embedded the profile+index
  into the user message on every mini-call and resent it at full price (the most frequently
  triggered call per channel). Now `sor_bolumlu` is called directly, profile+index/instruction sit
  in a fixed block (carrying cache_control).
- **A token ceiling on the chat reply in release too**: `CEVAP_TAVANI=3000` (previously `None`,
  unbounded).
- **Token metrics broken down by call type**: `Metrik.kategoriler`, `!durum` dumps the biggest-
  burning items; `Kullanim.prompt_tokens_details.cached_tokens` is read (to see the cache hit
  rate).
- **cache_control is now conditional on the target address** (`onbellek_destekler`): the first
  version checked the model name (claude/anthropic/gemini); when the user asked "why don't you
  support GLM," it turned out to be the wrong question — it can safely be added on any request
  going to OpenRouter regardless of model (it's part of OpenRouter's own schema, ignored on a
  model that doesn't support it); the real risk is the Mistral native API or a custom `API_ADRES`
  router. Now it only checks for the `openrouter.ai` address.
- **Reply-to is conditional again**: `Sohbet.son_etiketlendi` was added; the base `yanit` is only
  `son_mesaj` if tagged or `bekleyenler.len() > 1`, otherwise `None` (a plain message). This
  reverses the earlier "every reply is a reply-to" decision (both decisions are in kararlar.md,
  with rationale).
- **`durum/taranan.md` made persistent**: complaint "it re-scans messages from scratch on every
  connect" — `taranan` was in-memory, so every process restart repeated the 14-day scan.
- **GUILD_ID/CHANNELS (.env, optional)**: locks the bot to a single server/channel list; if empty,
  the old behavior (runs wherever it's reached) continues unchanged.
- **HTTP client timeout split out (P0 closed)**: `connect_timeout(10s)` + `timeout(180s)`,
  previously a single `.timeout(60s)` could cut off a long stream mid-way.
- **`mesgul` flag made RAII (`MesgulKilit`)**: 7 manual `remove` calls replaced with `Drop`; a
  panic in between no longer leaves the channel locked forever.
- Verification: 40 tests, clippy 0 warnings, `cargo fmt`, release build fine. None of this was
  seen live on Discord (no token, a constraint that holds project-wide).
- Deferred (asked/suggested to the user, not yet coded): converting `durum/` files to TOON
  (a risky/low-payoff assessment was made, below), adding a human-mood-imitation taxonomy to the
  personality prompt, the rest of the broad prod-readiness backlog (dev/yol-haritasi.md).

**TOON assessment**: `durum/` is mostly free-form text/prose (kisilik, huy, profil notes) — TOON's
real win is uniform table/array data, so there's no big gain here; the person/topic/event files
are read and written directly by prompts fed to the model (15 files), and moving to TOON would
mean rewriting all of them plus carrying the risk of whether small models (gpt-4o-mini, GLM) can
reliably produce a rarely-seen format. The only real candidate is `INDEX.md` (the index): uniform,
sent on every reply. Not implemented yet, put to the user.

---

## 2026-09-02 · Step 8 · Modals + /zihin coded
- New `src/modal.rs`: slash commands (`/durum` `/yardim` `/zihin`) open a modal, `!` commands stay
  in parallel as plain text; the mind modal is public to everyone, 5 slots (bot summary / people
  in two halves / topics / events+agenda). `sigdir` cuts an overflow at the 4000 limit on a line/
  space boundary + adds a note.
- `hafiza.rs` gained three new dump helpers: `kisi_dokumleri` (mtime order), `konu_dokumleri`,
  `olay_dokumu`. `interaction_create` branched: Command→modal, Modal→ephemeral confirmation,
  Component→`dusunce_dugmesi` (a separate impl Handler block). Guild slash registration on ready
  (idempotent).
- `komut.rs`: `!zihin` (an index dump + a `/zihin` pointer), `!durum` now shares
  `modal::durum_metni`; a slash note added to the YARDIM text.
- serenity 0.12.5 confirmations: `CreateModal::new(custom_id, title)` order, `CreateInputText::new(
  style, label, custom_id)`, `GuildId::set_commands`, the interaction variant is
  `Interaction::Modal`.
- Verification: 38 tests (4 new: slot limit/truncation, short text, person splitting,
  durum_metni), clippy 0 warnings, fmt clean. Remaining risk: live modal behavior on Discord
  hasn't been seen yet.

## 2026-09-02 · Step 8 planned: Modals + /zihin (not coded yet)
- Decisions: slash commands open a modal + `!` commands stay in parallel as plain text; the mind
  modal is public to everyone; 5 slots: Bot summary / People I-II / Topics / Events+Agenda.
- The `!zihin` message command will give an INDEX summary + a `/zihin` pointer (no dumping
  5×4000 into the channel).
- Confirmed from the serenity 0.12.5 source: `CreateModal::new(custom_id, title)` (order matters),
  `CreateInputText` in create_components.rs. Detailed to-do list in yol-haritasi.md.
- Text duplication in Steps 3-6 of the roadmap was cleaned up.

## 2026-09-02 · Log noise cut + colored output + message cleanup
- The serenity/reqwest tracing events flooding the console were cut with a target filter: the sink
  now only lets through records targeted at `discord_bot*` filtered by level; from foreign crates
  only warn/error show (a gateway error etc. isn't lost).
- ANSI color: ERROR red+bold, WARN yellow, INFO green, DEBUG/TRACE dim; the timestamp dim.
  TTY-detecting (no color when writing to a file), `LOG_RENK=on|off` forces it.
- 10 context-free `ai hatası: {e}` messages were given context (`ai [uret_akis] [{kanal}]:`,
  `ai [haber_tanit]:`, `ai [uyandim]:` ...); the "akis yarıda kesildi" warning got a channel
  prefix.
- Verification: 34 tests, clippy 0 warnings, release build.

## 2026-09-02 · Step 7 · final: docs + verification
- AGENTS.md refreshed (test count, sleep rule, id-based persons, new open items).
- Docs updates for all steps done (akislar, moduller, sabitler, kararlar, promptlar,
  durum-dosyalari, README).
- Verification: 34 tests, clippy 0 warnings, release build fine.

## 2026-09-02 · Step 6 · sleep mode: listen + accumulate + evaluate on waking
- Messages were already landing in raw memory while asleep; now `bellek_dongusu` runs a night
  observation every 2 hours and feeds it into the mind (marked `son_gece_gozlem`).
- The news cycle picks news while asleep and puts it in `stok_haber` (doesn't throw it away); on
  the first turn awake it goes out as "morning news" (`haber_gonder`, the shared send path).
- On the wake transition: if a tag is pending, a definite reply via `UYANDIM`, and on error the
  list is put back (nothing lost). If there's no tag, the `uyanis.md` agent scores the night's
  messages (`{"ilgi":0-10,"konu"}`); interest ≥5 triggers a "morning greeting" to the last channel
  via `uyanis-cevap.md`.
- News selection got the rule "topics about Nişantaşı University take priority" (haber-sec.md).
- Verification: 34 tests, clippy 0 warnings.

## 2026-09-02 · Step 5 · target selection + old start-from-scratch removed
- `Sohbet.son_gelenler` (name + message id, 20): who's written since the bot went silent; clears
  once the bot replies. If 2+ different people wrote, a mini-call to `hedef-sec.md` (40 tokens)
  picks the target → the reply gets attached to that person's message, the instruction gets an
  "address them" note.
- `AkisSonuc::Eski` removed: even if a new message arrives while generating, the stream still
  completes, the message isn't deleted; the new message is handled on the next turn.
- Dead-field cleanup: `Sohbet.etiketli` and `AkisBaglam.gelen` dropped.
- Verification: 34 tests (including `hedef_ayikla` for JSON/plain text/unknown name), clippy 0
  warnings.

## 2026-09-02 · Step 4 · reply willingness
- The fixed dice roll (`SANS × stage`) is gone: a tag/reply/name is always answered, other
  messages get a mini model call (`promptlar/isteklilik.md`, ~80 tokens): last 12 messages +
  profile + index → `{"puan":0-10,"sebep"}`; threshold `ISTEK_ESIGI`=6, ±1 for stage confidence,
  +2 while traveling.
- Rate limit: at most one call every 2 min per channel (`Durum.son_degerlendirme`).
- On a failed call, a fallback dice roll (`SANS=0.35`). `YOLDA_SANS_CARPANI` was removed (travel's
  effect is now in the threshold shift).
- Verification: 33 tests (including isteklilik_puan clamp/decoration), clippy 0 warnings.

## 2026-09-02 · Step 3 · id-based mind + second-precision timestamps + memory cycle
- Person files are now `kisiler/<id>.md`; `Kisi` fields: id, kullanici_adi, eski_adlar + history.
  When a name changes, the old name drops into `eski_adlar`, memory doesn't fragment. Clean start:
  old slug files in the directory are skipped.
- `Durum.ad_id` (name→id) and `kullanici_adlari` (id→username) are fed on every message and during
  the startup scan; `gunlukcu` converts names to ids from these, skipping+logging anything that
  can't be resolved.
- All records now use `tarih_saat()` with second precision (event/topic/person/archive/agenda).
- Memory cycle: a closed chat's transcript and a 6-hour observation drop into `bellek_kuyruk`;
  `bellek_dongusu` (every 10 min, not gated on sleep) processes them through günlükçü+profilci
  (+eleştirmen for a finished chat) in order. If the queue exceeds 50, the oldest is dropped
  (warn).
- Verification: 32 tests, clippy 0 warnings.

## 2026-09-02 · Step 1+2 · log simplification + 12-message limit removed
- **Step 1:** the info log now only carries critical events: slept/woke, PANIC/error, a mind write
  (günlükçü), stage transitions, startup/shutdown. Agent updates, gezgin, and message scanning
  moved down to debug.
- **Step 2:** `MAX_MESAJ`/`VEDA_ESIGI`/`BEKLEME` deleted; the goodbye and last-message prompts
  removed; there's no channel ban (`yasakli`/`girebilir_mi`) anymore. A chat closes silently
  `SOHBET_ZAMAN_ASIMI` (30 min) after the last message: `Durum.son_aktivite` + a minute-tick
  `zaman_asimi_kapat` (leaves busy channels alone, the closed transcript goes to günlükçü+
  eleştirmen).
- Verification: 31 tests, clippy 0 warnings.

## 2026-09-02 · Step 0 · the `dev/` folder set up
- Session memory: `dev/README.md`, `dev/ilerleme.md`, `dev/yol-haritasi.md`.
- Pointers added to `AGENTS.md` and `CLAUDE.md` (the first thing to read after a compact).
- Purpose: being able to pick back up after context balloons and gets compacted.

## 2026-09-02 · b4ae7a0 · Observability (Agent 3)
- `log` + a hand-rolled sink (`src/loglama.rs`, `LOG_SEVIYE` env var, default info).
- Panic hook: panics land in the log with a backtrace (fewer silent deaths in spawned cycles).
- 48 `println!/eprintln!` converted to leveled macros.
- Token usage metrics: `stream_options.include_usage`, `Kullanim`/`Metrik`, shown in `!durum`.
- Stream summary logs (chunk/first-chunk/total duration/done).
- Verification: 31 tests, clippy 0 warnings, release build.

## 2026-09-02 · 01be248 · Thinking UI
- A live word counter in hide mode: "Düşünüyorum... Şu ana kadar N kelime düşündüm."
- A "Show Thinking Process" button at the end of the reply → interaction_create → an ephemeral
  code block shown only to the clicker.
- In show mode, thinking appears both as a spoiler and as a code block.
- Discord Components (button) used since a spoiler alone can't provide real hiding.

## 2026-09-02 · 2e5eb17 · Command module + `!düşünme`
- Commands moved to `src/komut.rs` (`impl Bot`, the `use super::*` convention).
- `!düşünme göster/gizle/aç/kapat` mode (persisted in `durum/dusunme.md`), `!yardım`/`!help`.
- A "Düşünüyorum..." message while thinking; while off, requests go out without reasoning
  (`reasoning_kapat`).
- No newlines in thinking (`tek_satir`).

## 2026-09-02 · b1665d8 · Chat replies stream
- The reply no longer arrives all at once: the message opens with the first delta, edited at
  `AKIS_DUZENLEME` (1.2 s) intervals.
- Thinking goes into a spoiler unclipped (`reasoning` + `reasoning_content` fields).
- A reply over 1900 splits into a new message at a sentence/space boundary (`bol`), no truncation.
- The `cevap_butcesi!()` macro: `None` in release (unbudgeted), `Some(2000)` in debug.
- `kisalt`/`cevap_olcusu` removed; routing to a custom router via `API_ADRES`.

## 2026-09-02 · 72b7f4a · Merge PR #1 (Speretta/main)
- Merged from the fork into krxi/discord-bot. History joined into a single line.

## Analysis report (5 agents) — summary
Five parallel agents scanned the code; the gist of their reports is in the risk list in
`yol-haritasi.md`. Main findings: a global 60 s timeout can cut off a stream (P0), `mesgul` leaks
on panic, file writes aren't atomic + agents race each other, cycles die silently on panic, the
person key is the display name (not the id).

## 2026-09-02 · zihin-ss branch · `!zihin` panel screenshot
Emin: "when you type !zihin it should post a screenshot in a modern web-UI style." `src/
zihin_gorsel.rs` added: text is laid out as SVG, rasterized to PNG with `resvg` (0.48,
default-features off) — pure Rust, no Chrome/browser/server. Inter Regular/SemiBold/Italic
embedded under `fonts/` (SIL OFL, `fonts/LICENSE`).
- Panel: a browser chrome strip + title/chips + a 5-box counter strip + a 7/12–5/12 grid (left:
  People, Events · right: Topics, Agenda, Self, Temperament). Canvas 1280 px, rasterized at 2x.
- Reading happens in two stages: `zihin_verisi` copies the locked fields, `dosyalari_oku` reads
  files outside the lock; PNG generation happens in `spawn_blocking` (rule 1 preserved).
- `!zihin` sends the image as an attachment, single-line title. Falls back to the old embed card
  on failure.
- `cargo run -- zihin` subcommand: generation without Discord (to see the design + verify it).
- Tests: text wrapping, XML escaping, dropping emoji, no panic on empty data, with real data a
  valid PNG signature + under 8 MB, event-line parsing. 58 tests total, clippy 0 warnings.
- **Not verified:** the image has never been seen live on Discord. Text width isn't measured from
  real glyphs, it's estimated from Inter's letter/em ratio (deliberately rounded up — wraps early
  rather than overflowing).

## 2026-09-03 · Codebase translated to English
User's request: translate README.md and all `src/**/*.rs` to English, the bot's way of operating
(Turkish personality/behavior) stays unchanged. Scope confirmed in clarification rounds: file
names get translated too, code references inside AGENTS.md/docs/dev/ get updated to the new names.

- **Scope.** 42 `.rs` files (~8500 lines) + `build.rs`: identifiers (function/struct/enum/const/
  static/field/local variable), file+directory names (`src/bot/`, `src/command/`), comments,
  `.env` variable names. Every `git mv` + content translation was done file by file, git rename
  detection was preserved (`git status` shows "RM").
- **Core glossary** (from docs/sozluk.md, used as the baseline for consistency): durum→state,
  hafiza→memory, sohbet→chat, gundem→agenda, gelisim→growth, seyahat→travel, uyku→sleep,
  ajanlar→agents, promptlar→prompts, loglama→logging, komut→command, dongu→cycle (⚠ "loop" is a
  Rust keyword, couldn't be used), saglayici→provider, dokum→transcript, uret→generate,
  analiz→analyze, cevap_parcala→parse_reply, soy→strip_name, kirp→trim/kirp_hata→trim_error,
  and dozens more function/type names (full list: git diff or docs/moduller.md).
- **Critical findings (caught before hitting them):**
  - `Mesaj` → translated to `ChatMessage`, not `Message`: `use serenity::all::*` already carries
    its own `Message` type, that would have collided.
  - Struct fields tied to the model's JSON output (`isteklilik_coz`/`parse_willingness`'s
    `puan`/`sebep`, `hedef_ayikla`/`extract_target`'s `hedef`, `ruh_hali_ayikla`/`extract_mood`'s
    `durum`/`yogunluk`, günlükçü/diarist's `Kayit`/`Record`'s `olay`/`kisiler`/`isim`/
    `puan_degisimi`/`not`/`bilgiler`/`etiketler`/`konular`/`ad`/`kendim`) were **not translated**
    — since the prompts stay in Turkish, the model produces JSON with these field names, and serde
    matches on field name; if translated, it would silently come back empty/0.
  - Literal field prefixes in the `durum/` file format (`"kullanici_adi:"`, `"puan:"`, `"etiket:"`,
    `"not:"` etc.) and directory names (`kisiler/`, `konular/`, `olaylar/`, `arsiv/`, `kanallar/`,
    `durum/`, `resimler/`) didn't change — they needed to stay compatible with existing user data
    on disk.
  - Everything that reaches Discord stayed Turkish: slash command names/descriptions/option
    labels, embed titles/field text, button/menu labels, model output, `!durum`'s category labels
    (`"sohbet"`, `"isteklilik"`, `"profilci"` etc. — these show up in the `/durum` embed). Debug
    trace text (lines going into `self.debug_note`) is the exception: counted as developer
    diagnostics, translated to English.
  - `promptlar/*.md` (30 files) were left untouched — directory, file names, and content alike.
- **`.env` variable name changes** (the user has to update their own `.env` by hand, no backward-
  compat shim was added): `SAGLAYICI→PROVIDER`, `KANALLAR→CHANNELS`, `HABER_KANALI→NEWS_CHANNEL`,
  `DEBUG_KANALI→DEBUG_CHANNEL`, `RESIM_ANALIZI→IMAGE_ANALYSIS`, `API_ADRES→API_URL`,
  `LOG_SEVIYE→LOG_LEVEL`, `LOG_RENK→LOG_COLOR`. The CLI flag `cargo run -- sohbet` → `cargo run --
  chat`; the terminal bench's fallback speaker name `emin` → `misafir` (at the user's request, so
  a real person's name isn't the fallback value).
- **Verification:** `cargo build` clean, `cargo test` 76/76 green (up from 75, +1; tests were
  translated too, logic unchanged), `cargo clippy --all-targets` 0 warnings, `cargo fmt` applied.
  Not tried on live Discord (it never had been anyway).
- **Docs updated:** AGENTS.md (item 8's content reversed: now says "identifiers are English" + all
  the code references), README.md (full translation), docs/moduller.md (full reference update),
  docs/kararlar.md (a new dated entry added, old ones untouched — per the rule). docs/sozluk.md's
  purpose changed: no longer a code glossary, now a glossary of the runtime vocabulary that stays
  Turkish.
- **Next step (if any):** updating code references inside docs/akislar.md, docs/mimari.md,
  docs/durum-dosyalari.md, docs/promptlar.md, docs/sabitler.md, docs/gelistirme.md is ongoing/still
  to do — like moduller.md, these are "current state" reference docs, not chronological logs like
  kararlar.md/ilerleme.md.

---

## 2026-09-03: professional `///` doc comments on every function

- User's request: after the code translation was done, the comment lines should be "professional"
  — every function should document the objects/functions it uses, the input it takes, the output
  it returns, which other functions use it the same way, and which structs hold which data. In the
  scope-clarification question the user picked **"every function (full scope)"** — not just
  public/cross-module ones, genuinely every function.
- `///` doc comments in `Input:`/`Output:`/`Uses:`/`Used by:` form were added to every function/
  struct across the 42 `.rs` files + `build.rs` (dependencies and callers are referenced along
  with their file name).
- Since the same four-field template doesn't make sense for `#[test]` functions (75 of them,
  `bot/tests_1..4.rs` + `mod test` blocks in other files) — no input/output/caller — a one-line
  "what this verifies" description was added instead — still within "every function" scope, just
  in a lighter form suited to tests. `src/prompts.rs` (only `pub const` declarations) and the
  `include!` bodies of `src/command.rs`/`src/command/registration.rs` weren't touched since they
  contain no functions.
- A side issue caught during this pass: `clippy::doc_lazy_continuation` — a prose paragraph coming
  right after a bullet list (`- ...`) with no blank line in between was being read as an
  "unindented continuation" of the list's last item (`types_message.rs`, `types_chat_state.rs`,
  `types_bot.rs`, 9 warnings across 3 files). Fix: a blank `///` line added between the list and
  the following paragraph.
- **Verification:** `cargo build` clean, `cargo test` 76/76 green, `cargo clippy --all-targets`
  0 warnings, `cargo fmt` applied (produced no extra changes).

---

## 2026-09-03: src/bot/ split into 7 subfolders

- User's request: `src/bot/` had 38 files in a single folder, that's not good, said it should be
  grouped.
- Moved via `git mv` into 7 topic-based subfolders: `types/` (5), `text/` (4), `provider/` (11),
  `chat/` (3), `cycle/` (6), `handler/` (3), `tests/` (5). Each folder has an aggregator file
  sharing its name (e.g. `types/types.rs`), which collects its siblings via a relative
  `include!("...")` — since `include!`'s path always resolves relative to its own file's folder
  (not relative to main.rs), the in-folder includes needed no changes. `setup.rs` stayed folderless
  since it's a single file. The 7 `include!("bot/...")` lines at the top of `main.rs` were updated
  to the new paths (`bot/types.rs` → `bot/types/types.rs` etc.).
- Side note: while doing this, `///` lines added AFTER `#[test]`/`#[tokio::test]` during the
  earlier doc-comment pass (a doc comment coming AFTER the attribute — compiles, but not the usual
  order) were noticed; the doc comment and attribute swapped places in 76 spots (doc comment now
  comes first, `#[test]` after — idiomatic order).
- **Verification:** `cargo build` clean, `cargo test` 76/76 green, `cargo clippy --all-targets`
  0 warnings, `cargo fmt` applied. `docs/mimari.md` and `docs/moduller.md` (current-state reference
  docs) were updated to reflect the new folder layout; old flat `bot/xxx.rs` path references in
  `src/travel.rs`, `src/bot/types/types_settings.rs`, `docs/sabitler.md`, `docs/gelistirme.md`,
  `README.md` were replaced with the new folder paths. Past entries in `docs/kararlar.md`/
  `dev/ilerleme.md` were left untouched per the rule (a chronological log isn't retroactively
  corrected).

---

## 2026-09-03: reactions on the bot's own messages are now evaluated too

- User's request: the bot should now also evaluate emoji reactions dropped on its own messages.
- `GatewayIntents::GUILD_MESSAGE_REACTIONS` added (`main.rs`); `Handler::reaction_add`
  (`src/bot/handler/handler_event.rs`) added as a new serenity event callback.
- It only cares about reactions on **the bot's own message**, dropped by a human (not a bot),
  inside a server (not a DM), and passing the `GUILD_ID`/`CHANNELS` filter — everything else exits
  early. Since the `Reaction` event carries neither who the reactor is nor the reacted-to message's
  text, both are fetched over HTTP via `add_reaction.user`/`.message`. A reaction on a message with
  empty text (embed-only — a card/status message) is skipped.
- `reaction_label` (a new helper) makes the emoji human-readable: unicode as-is, a custom server
  emoji as `:name:` (`ReactionType`'s own `Display` gives Discord's raw `<:name:id>` mention form,
  which would mean nothing to the model).
  The result becomes a line like `"(reaction 💀) reacted to the message \"...\""`, dropped into
  `remember` (raw memory) and `channel_note`; if a chat is open in that channel, the same line is
  also appended to `chat.history` (so the model sees it as context on its next reply). **Doesn't
  deliberately trigger a reply on its own** — no willingness evaluation, no new message goes out;
  if the bot spoke on every reaction it'd be spam/wasted tokens. A trace is logged if `debug` is
  on.
- New unit test: `reaction_label_formats_emoji` (unicode/custom emoji/nameless emoji).
- **Verification:** `cargo build` clean, `cargo test` 77/77 green, `cargo clippy --all-targets`
  0 warnings, `cargo fmt` applied. Not tried on live Discord (the new intent + HTTP fetches have
  never been seen, see AGENTS.md "Known risks").
- **Docs:** a "A reaction was dropped" section added to docs/akislar.md, a `reaction_add` line to
  docs/moduller.md, a "what it does" item to README.md, an unverified note to AGENTS.md; the
  `cargo test` command line updated to 77.

---

## Note — verification commands
```
cargo fmt && cargo clippy --all-targets && cargo test && cargo build --release
```
`AGENTS.md` rule: clippy 0 warnings expected. Identifiers are English and ASCII (`thought`,
`trim`) — only the bot's Turkish way of operating (prompts/, durum/ file formats, everything that
reaches Discord) is exempt, see AGENTS.md item 8.

- 2026-09-06: graphify knowledge graph added (`graphify-out/`, 965 nodes / 1713 edges, 75 communities). Tracked: graph.json, GRAPH_REPORT.md, manifest.json; cache/html/cost ignored. Usage in AGENTS.md "Knowledge graph" and docs/development.md.

## 2026-10-11 · v1.1.0 — self-host packaging and globalization

- **Self-host packaging:** Docker image `ghcr.io/krxi/discord-bot` for amd64/arm64, a root
  Compose service with `.env`, persistent `durum` volume, and read-only `photos` mount.
  The `v*` release workflow packages Linux x86_64/arm64, macOS Apple Silicon, and Windows
  x86_64 binaries and publishes the multi-architecture image. CI checks fmt, clippy, tests,
  and a Docker build. Publication is a release-workflow action, not a locally verified download.
- **First-run setup:** `discord-bot init` asks for token, provider, key, model, language, owner,
  and UTC offset; writes `.env` (mode `600` on Unix); and prints an invite link with permissions
  `67226688`. Missing-token startup directs users to the wizard; startup without a server logs
  an invite link.
- **Globalization:** English is now the default. `OWNER_ID` replaces the hard-coded favorite,
  keeps the owner liked, and gates model changes. `TZ_OFFSET` defaults to UTC and controls the
  sleep schedule. `NEWS_RSS` overrides the RSS feed (BBC World for English, Sözcü for Turkish).
  The Turkish holiday travel calendar is disabled in English mode.
- **Upgrade from 1.0:** add `BOT_LANG=tr`, `TZ_OFFSET=3`, and `OWNER_ID=<your id>` to the existing
  `.env` to retain the previous language, schedule, and owner behavior. Keep/back up the
  existing memory; a new Docker volume does not automatically import a host `durum/` folder.
- **Docs and launch preparation:** English and Turkish READMEs now lead with a plain-language
  explanation, Docker/binary/source setup, honest cost/privacy notes, a Mermaid data-flow
  diagram, and an FAQ. The beginner setup guide is linked prominently. `docs/launch.md`
  contains manual-only HN, Reddit, and X drafts plus a launch checklist. MIT licensing,
  contribution guidance, issue templates, and the VHS demo tape are repository additions;
  there is no recorded demo GIF yet.
- **Evidence boundary:** setup, defaults, scheduling, ownership, persistence, command names,
  CI, and release targets were checked against repository code/configuration. OpenRouter's
  published limits were fetched directly. No RAM benchmark, fixed monthly price, credit-card
  requirement claim, or telemetry guarantee was adopted from market research.
- **Requested README test-count check:** `cargo test --locked` reported 93 passed, 0 failed,
  0 ignored on 2026-10-11. No unlocked fallback was needed and Cargo.lock was not edited by
  the documentation task. This is local test evidence, not a live Discord or Docker smoke test.
- **Integration evidence (reported by the integrator):** the BBC World RSS feed parsed into
  32 items. This was not rerun by the documentation task.
- **Market research applied:** both research inputs informed the social-first hook,
  Docker-first setup, bounded-recall explanation, local-memory/Discord/provider distinction,
  qualified setup-time language, real-demo checklist, and evidence-based follow-up guidance.
  Competitor star counts and launch traction were not turned into product claims.
- **Pending starter work:** custom emoji reactions, English travel calendar, English sleep
  weekday/mood keywords, and the obsession keyword hook.

## 2026-10-11 · release notes published once

- Replaced the duplicated v1.1.0 release body with one set of highlights, upgrade settings,
  and the full commit-list link. The existing tag and four release archives are unchanged.
- Binary matrix jobs now upload `bin-<target>` artifacts. A single `publish` job waits for
  all binaries, downloads their archives, and generates release notes once.
- `actionlint` passed. This publication path will be exercised by the next release;
  v1.1.0 was not retagged.

## 2026-10-11 · multi-platform extraction, domain ingress (step 1)

- Step 0 committed as `27a80ba`: all 93 baseline tests retained; 116 tests passed
  with `cargo fmt --check`, locked clippy with warnings denied, and locked tests.
  Existing vendored Serenity deprecation warnings remain dependency warnings.
- Shared Arc-backed typed identities and reversible per-component canonical
  encoding now distinguish platform/native scope from configured connections.
  Domain ingress carries structured users, rooms, replies, mentions, attachments
  and reactions; Discord SDK decoding is separate from personality decisions.
- Chat bookkeeping uses borrowed/shared domain keys. The terminal bench no longer
  pretends to be a Discord room. v1 numeric storage remains confined to the memory
  boundary until the next atomic migration; other platforms are not enabled here.
- Native Discord renderer fixtures additionally capture reply-user pings, thought
  output, off-mode settings button's persisted `kapali` value (distinct from slash
  option `kapat`), and stealth final-only output. These existing quirks are retained.
