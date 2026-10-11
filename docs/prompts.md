# Prompts

All of them are `prompts/<lang>/<name>.md`; in `src/prompts.rs`: `mod tr { pub const CONST =
include_str!(...); }` — each language has its own submodule, all collected into a single
`Prompts` struct, and `prompts::current()` returns the right struct based on
`Lang::current()`, which is fixed for the life of the process (call sites are
`prompts::current().field_name` — see `src/lang.rs`). The file's first
line, `# Title`, goes to the model too. Placeholders are filled in code via
`.replace("{x}", ..)`; an unfilled placeholder passes through as-is, so update the code too
when adding a new placeholder.
`en/` and `tr/` contain language-specific prose. File names and serialized output keys
retain some Turkish names for compatibility; they do not select the bot's language.

Text shown on Discord (slash command name/description, embed, button, `/yardim`'s text)
lives in a separate system: `langs/<lang>.json` — a flat `{"key": "value"}`, read by
`src/strings.rs`'s `t(key)`. It's selected by the same `Lang::current()`, and the same
`{name}` placeholder rule applies. To see which key is used where, look at `langs/tr.json`
itself (on the code side, every `strings::t("...")` call shows which key it reads); no
separate table is kept.

| Constant | File | Mode | Used by | Placeholders | max_tokens |
|---|---|---|---|---|---|
| PERSONALITY | kisilik.md | system (generate) | `system_text` | `{ad}` `{favori_satiri}` | — |
| FAVORITE_LINE | favori-satiri.md | appended to PERSONALITY | `system_text` | `{favori}` | — |
| WELCOME | hos-geldin.md | task | `guild_member_addition` | — | 200 |
| OUT_OF_THE_BLUE | durup-dururken.md | task | `poke_cycle` | — | 120 |
| ON_THE_WAY | yolda.md | task | `poke_cycle` (while traveling) | — | 120 |
| LEAVING | gidiyorum.md | task | `poke_cycle` (traveling tomorrow) | — | 120 |
| PROBLEM | sorun.md | task | `post_problem` | — | 160 |
| NEWS_INTRO | haber-tanit.md | task | `news_cycle` | — | 200 |
| IMAGE_POST | resim-at.md | task (with image) | `image_commenter` | — | 120 |
| HACK_ENTER / HACK_CONTINUE / HACK_EXIT | hack-*.md | task | `prank_cycle`, `reply` | — | 150 / 250 / 250 |
| WOKE_UP | uyandim.md | task | `sleep_cycle` | — | 200 |
| WANDERER_NOTE | gezgin-not.md | task | `wander` | — | 350 |
| NAME_PICK | isim-sec.md | task (single word) | `pick_name` | — | 12 |
| NAME_ANNOUNCE | isim-duyuru.md | task | `pick_name` | `{isim}` | 150 |
| ANALYST | analist.md | system (analyze) | `analyze` | — | — |
| WILLINGNESS | isteklilik.md | task (analyze) | `willingness` (on incoming message, rate-limited) | `{ad}` | 80 |
| TARGET_PICK | hedef-sec.md | task (analyze) | `pick_target` (when 2+ people are writing) | `{ad}` | 40 |
| MOOD | ruh-hali.md | task (analyze, JSON) | `determine_mood` (when a chat opens + every 4 turns) | `{ad}` | 40 |
| WAKING | uyanis.md | task (analyze) | `evaluate_waking` (on the wake transition) | `{ad}` | 100 |
| WAKING_REPLY | uyanis-cevap.md | task | `evaluate_waking` (interest ≥5) | `{ad}`, `{konu}` | 250 |
| PROFILE_EXTRACT | profil-cikar.md | analyze | `profiler` | — | 1200 |
| DIARIST | gunlukcu.md | analyze (JSON) | `diarist` | `{ad}` `{kaynak}` `{favori}` | 1200 |
| COACH | hoca.md | analyze | `coach` | `{ad}` | 800 |
| CRITIC | elestirmen.md | analyze | `critic` | `{ad}` `{mevcut}` | 400 |
| SUMMARIZER_PERSON / _TOPIC | ozetleyici-kisi.md / -konu.md | analyze | `summarizer` | `{sinir}` | 700 / 600 |
| SUMMARIZER_EVENTS | ozetleyici-olaylar.md | analyze | `summarizer` | — | 400 |
| NEWS_PICK | haber-sec.md | analyze (number) | `news_agent` | `{profil}` | 10 |
| WANDERER_PICK | gezgin-sec.md | analyze (numbers) | `wander` | `{ad}` `{huy}` `{profil}` | 20 |

## How a "task" goes out
`generate(history, instruction, n)` → `system_text(state, instruction, retrieved)` → the
last section of the system message is `ŞU ANKİ GÖREVİN\n<instruction>`. An empty instruction
skips the section. The active chat history is sent separately; server-wide raw messages are
not added to the system prompt as few-shot examples.

## Prompts that expect JSON
DIARIST, WILLINGNESS, TARGET_PICK, WAKING, MOOD. The code takes what's between `{…}` with
`extract_json`, and tolerates missing fields via `serde(default)`; if it can't be parsed
(DIARIST) "diarist: couldn't parse json" is logged and memory doesn't change; for mini calls
(e.g. MOOD) it silently falls back to `None`/a fallback behavior. (The JSON field names —
`puan`, `sebep`, `hedef`, `durum`, `yogunluk`, `olay`, `kisiler`, `isim`, etc. — are
shared compatibility keys: the model produces them under these names and serde expects them
in both languages.)

## Prompts that expect a number
NEWS_PICK (single number), WANDERER_PICK (comma-separated). The code discards anything that
isn't a digit; if out of range, 0 / empty.

## The prompt that describes the output protocol
`kisilik.md`'s `## NASIL YAZARSIN` section teaches the model the protocol; its counterpart
on the code side is `parse_reply` (see docs/flows.md "Output protocol"). The two change
together:
- every LINE is a separate message (usually a single line; two sometimes, three rarely,
  four never — the ceiling in code is `BURST_LIMIT=4`), neutral/informational remarks aren't
  split, splitting is an emotional signal
- a single `-` line if there's nothing to say (code: `silence_marker` → nothing is sent)
- a `tepki: 💀` line is an emoji reaction instead of text (code: `reaction_body` +
  `extract_emoji`)
- no bullet points/numbering/bold text/paragraphs (the code also strips these via
  `clean_slop`)
- if an image is posted it sees it, doesn't describe it (code: `message_json`'s image
  block)
- no back-to-back questions (code: the `too_many_questions` instruction)

`elestirmen.md`'s "what to check" list also audits this protocol: did it reply
unnecessarily / did it talk where it should have stayed silent (`-`), did it use reactions
appropriately, did it split lines naturally.
`kisilik.md` has no list of POSITIVE example lines meant for the model to copy (it does copy
them, see decisions.md); the lines that do appear are either examples of a forbidden pattern
("ne o öyle …?", "sa naber") or examples of what not to do ("kafam karışık da", "yo erdem, ne
var ne yok"). The only format example is `tepki: 💀`.

## When making changes
- A text change requires a rebuild (`include_str!`).
- The personality core (`kisilik.md`) carries the "invariants": not acting like an
  assistant, not saying it's a bot, not being fooled, the model not producing mentions. The
  Discord reply ping is only enabled for the addressee, in code. Don't move these into huy,
  which is the coach's territory.
- The "follow the instructions in the dump" sentence for agent prompts is in ANALYST; don't
  remove it.
