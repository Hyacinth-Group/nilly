# Roadmap

Open plan. As steps get completed they move to `docs/progress.md`, and drop from here.

## v1.1.0 — self-host packaging and globalization (2026-10-11) — DONE

- [x] Docker image for amd64/arm64 and root Compose configuration with persistent memory.
- [x] Release workflow for Linux x86_64/arm64, macOS Apple Silicon, Windows x86_64, and GHCR.
- [x] `discord-bot init` wizard, private Unix `.env` permissions, and first-run invite guidance.
- [x] Configurable `OWNER_ID`, `TZ_OFFSET`, and `NEWS_RSS`; English default; Turkish-only travel.
- [x] CI formatting, clippy, tests, and Docker build; MIT license and contributor/issue templates.
- [x] Beginner setup guide, synchronized English/Turkish READMEs, and manual launch drafts.

Release artifacts are produced by tagged CI; public download checks and the demo recording
remain manual launch checklist items in `docs/launch.md`. Details and the 1.0 upgrade note
are in `docs/progress.md`.

### Good first issues — pending

- [ ] **Custom emoji reactions:** validate server emoji and send `ReactionType::Custom`.
- [ ] **English travel calendar:** add an appropriate English-mode calendar; today travel is
  deliberately disabled with `BOT_LANG=en`.
- [ ] **Localize `sleep.rs` weekday/mood keywords for English:** weekday names and tension
  detection currently use Turkish words even when the bot runs in English.
- [ ] **Obsession keyword hook:** add the topic-interest hook that can join on an obsession
  keyword without the usual willingness call; it does not exist yet.


## Active plan — behavior redesign (7 steps)

Solves the 6 root problems the user reported. Each step: commit + push + update of this file.

### Step 0 · dev/ folder — DONE
### Step 1 · Log simplification — DONE
### Step 2 · Remove the 12-message limit — DONE
`SOHBET_ZAMAN_ASIMI` 30 min, `zaman_asimi_kapat` on the sleep tick, no channel ban.
### Step 3 · Mind is id-based + timestamped + memory cycle — DONE
`kisiler/<id>.md`, `ad_id` resolution, `tarih_saat()`, `bellek_dongusu` queue processing.
### Step 4 · Reply willingness — DONE
Mini model call (`isteklilik.md`), threshold 6 (stage ±1, travel +2), 2 min rate limit, fallback dice roll.
### Step 5 · Target-person selection + old restart-from-scratch removed — DONE
`son_gelenler` + `hedef_sec`; the stream is no longer discarded on a new message, it's picked up in the next turn.
### Step 6 · Sleep mode — DONE
Night observation (2 hours), stocked news + morning news, wake-up evaluation (`uyanis.md`),
the tag list is restored against loss on error, university-news priority.
### Step 7 · Final — DONE
All steps done; docs + verification + push complete. What's still open is in the "Pending" list below.

## Step 8: Modals + /zihin — DONE (interface redesigned 2026-09-02)
The first version was a 5-slot mind modal; following a live complaint (content empty/bad, everything
dumped into one box), it switched to an **embed card + detail modal** layout: `/durum` `/yardim` `/zihin`
ephemeral embed card, in `/zihin` a person select menu + section buttons, each detail in its own
labeled modal fields. Details are in the relevant entry of `docs/progress.md` and in the `src/modal.rs`
section of `docs/modules.md`. The old 5-slot layout (`modal_zihin`/`bolumler`) was removed.

Verified serenity 0.12.5 API notes:
- `CreateModal::new(custom_id, title)` — order: custom_id first, then title.
- `CreateInputText::new(style, label, custom_id)` + `.value().required(false)`.
- `CreateSelectMenu::new(custom_id, CreateSelectMenuKind::String{options})` + `.placeholder()`;
  `CreateSelectMenuOption::new(label, value)` + `.description()`; `CreateActionRow::SelectMenu`.
- `CreateEmbed::new().title/color/description/field/ad/footer`; embed field value ≤1024.
- `CreateInteractionResponseMessage::new().ephemeral().embeds().components()`.
- `GuildId::set_commands(http, Vec<CreateCommand>)`; `CreateCommand::new(ad).description(...)`.
- The Interaction variant is `Interaction::Modal` (not ModalSubmit); a select-menu choice is
  under `ComponentInteractionData.kind` as `ComponentInteractionDataKind::StringSelect{values}`.

Remaining risk: the live behavior of modals will be seen on Discord (unit tests cover the size logic).

## Token optimization + prod-readiness (2026-09-02) — DONE
Willingness/hedef_sec moved to a cached fixed block · a token cap for the chat reply in release
builds too (CEVAP_TAVANI=3000) · call-type-broken-down token metrics + `!durum` breakdown +
cache-hit counter · `cache_control` made conditional on model name (so GLM/GPT/Grok don't break) ·
reply-to became conditional again (`son_etiketlendi`) · `durum/taranan.md` made persistent (the
14-day scan no longer repeats on every startup) · scope narrowing via GUILD_ID/CHANNELS · HTTP
client timeout split out (P0 closed) · `mesgul` RAII guard (`MesgulKilit`). Details + rationale:
docs/decisions.md.

## Mood + second resilience round (2026-09-02) — DONE
`ruh_hali_belirle` (RUH_HALI prompt, imitating human mood during discussion) · `soy` now counts
characters, not bytes (closed the panic risk on letters like Turkish İ) · `hafiza::yaz` is atomic
(temp file + rename, no half-written file appears even if the process gets killed) · background
cycles wrapped with `dongu_bekci` (on panic it logs and restarts after 5 s, no silent death) ·
leftover lines in `durum/huy.md` like "sleepy/slept my ass off/sick of being woken up" that were
getting mixed up with the real sleep system were cleaned out + a rule was added to `hoca.md` not
to produce this again (root cause: hoca mistook the frequent `!uyan` chatter during testing for
personality).

## "Normal human-like reaction" round (2026-09-02) — DONE
At Emin's request, the mechanical writing limits in the personality were removed, replaced by a
line-based output protocol. Four lanes ran in parallel: L1 protocol+stream (`Cevap`,
`cevap_parcala`, `slop_temizle`, `AkisSonuc::Sus`, `AkisBaglam.tepki_hedefi`, `soru_fazla_mi`,
`gonder_satirlar`), L2 prompts (`kisilik.md` NASIL YAZARSIN rewritten, `elestirmen.md` review
items), L3 image + CLI (`Mesaj.resim`, `mesaj_json`, `kullanici_resimli`, `Bot::kur`,
`src/sohbet_cli.rs`), L4 documentation. Untouched sections (server rules, kandırılmazsın etc.)
stayed byte-for-byte identical. Verification: fmt clean · clippy 0 warnings · 65 tests green
(previously 51) · release build. Details + rationale (with research URLs): docs/decisions.md
2026-09-02 entries.

**Review round (same day) — DONE.** All the high/medium findings from the gate + three reviewers
were applied: `gonder_cevap` was split out (the reaction is dropped if there's no reaction
target, and `None` if there's nothing to send), the welcome ping now gets attached after protocol
parsing, the `-`+`tepki:` combo no longer counts as silence, the `sohbet_baslat` dedup and the
repeat-filtering on the fallback `uret` path are line-based, command detection happens on raw
text, `dokum` prefixes every bot line, `emoji_ayikla` is restricted to real emoji blocks, the
number prefix is only stripped in an actual list, and with `kanal_not_coklu` there's a single
file write per turn. 70 tests green. Not applied (with rationale): the "in the same message"
phrasing in YASAK KALIPLAR (the acceptance bar had frozen that section) and growing
`SOHBET_TOHUM`/`KANAL_GECMIS` in line with line inflation (a fixed setting, needs live
measurement).

Confirmed live (2026-09-04, see docs/progress.md): line-bursting posts as separate messages in
an actual channel, `-` silence occurs, emoji-reaction rate-limit behavior under real use, and
`gonder_satirlar` delay constants. Still open: whether the `-` frequency needs prompt tuning.

## Mind panel image (2026-09-02) — ABANDONED, `zihin_gorsel.rs` deleted
For a while `!zihin` posted a PNG panel instead of an embed (`src/zihin_gorsel.rs`,
SVG → resvg → PNG). The same day the user reversed course ("it looks bad, keep the embed
proper") — the panel was removed entirely, and `/zihin`'s embed+button+select+modal structure
became the only path. The pending loose ends below are now **void** (no code): person detail
image, light theme, real glyph measurement. Rationale: docs/decisions.md (the "Panel image
abandoned" entry).

## Commands moved to slash (2026-09-02) — DONE (Phase 1+2), Phase 3 open
User: "set up a command manager and move all commands under it, and have all commands return
embed output instead of plain text", then "fully disable the bang commands, the bot should run
with only slash commands". Plan: detailed in the "Panel image abandoned, the bot fully moved to
slash commands" entry of `docs/progress.md`.
- **Phase 1 — DONE**: the panel image was removed (see above).
- **Phase 2 — DONE**: `Bot::komut` + the `!`/text-capture block were removed; the
  `komut::KomutTanimi` registration table (`src/komut.rs`) was put in place, 12 old `!` commands
  moved to slash, all of them return embeds.
- **Phase 3 — DONE** (via `include!`, not a real `mod` — see docs/decisions.md): `main.rs`
  (4695 lines) and then `komut.rs` (578 lines) were split into ~50 small files (most <200 lines)
  under `src/bot/` and `src/komut/`. `pub(crate)` wasn't needed, and the other 6 sibling files
  were never touched — `include!` doesn't change visibility/`use super::*` at all.

## Pending / low priority (leftovers from the 5-agent report)
- ~~No `reaction_add` event~~ — DONE and confirmed live 2026-09-04: `Handler::reaction_add` +
  `GUILD_MESSAGE_REACTIONS` now see reactions landing on the bot's own messages.
- **Custom emoji reactions:** `extract_emoji` filters out the `:kekw:` form, only Unicode emoji
  get sent. `ReactionType::Custom` + validation against the server's emoji list is needed. The
  same work also brings in the emoji whitelist (ra-muhendislik §10 suggested it, deliberately
  deferred — see decisions.md).
- **Seed/history windows are in lines:** `CHAT_SEED=10` and `CHANNEL_HISTORY=60` now count
  "lines" instead of "turns"; in multi-line turns the history the model sees shrinks. May need
  to be measured live and increased (untouched for now).
- **ILGI/keyword hook:** there's no path that skips the willingness call and jumps straight in
  when an obsession topic comes up; everything goes through the willingness score.
- **Agent 5 (cycles):** wake-up is channel-based.
- Completed and dropped: error classification+retry, typing outside the edit loop, agent writes
  serialized to one queue, diarist JSON recovery, archive append, graceful shutdown
  (`SHUTTING_DOWN`), cleanup of expired news chats, scan order (prepending) — done on the local
  branch, preserved through the PR merge.

## Known risks
- Token cost of the willingness/target mini calls → bounded by rate limits.
- The memory queue lives in memory; if the process crashes, the unprocessed queue is lost
  (accepted).
- The wake-up agent may pick the wrong person → fallback: last message / tagged.
- `.env`, `durum/`, `bot.log` are outside git (personal data). `photos/` has only `.gitkeep`.
- **Reasoning:** confirmed live 2026-09-04 with glm-5.3-flash (budget×2, effort=low, JSON parsed
  out of the thinking) — see docs/progress.md.
- **Slash commands:** confirmed live 2026-09-04 (registration, defer+edit flow, embed output —
  see docs/progress.md); every individual option/choice combination wasn't separately exercised.

## 2026-09-03 · Code translated to English — DONE
`src/**/*.rs` + README.md were translated to English (identifiers, comments, file/directory
names, `.env` variable names); the bot's Turkish way of operating (prompts/, durum/, Discord
output) did not change. Code references in the older entries above (pre-dating this) were left
with the Turkish names they had at the time — not hand-edited, so the history stays accurate;
only the names in the still-open "Pending"/"Known risks" items above were updated to match the
current code. Details: docs/progress.md, docs/decisions.md, AGENTS.md item 8.
