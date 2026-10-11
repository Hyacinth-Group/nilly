# durum/ records

Runtime memory; not committed to git. The bot loads from here on restart.

**Storage**: everything outside `arsiv/` lives in a single file, `durum/hafiza.redb`
(redb — pure Rust, ACID transactions; why this instead of rusqlite/plain-JSON:
`docs/decisions.md`). The table below still shows the old file paths because that's by
design: each record is stored in redb keyed by its old relative path string
(`"kisiler/1.md"`, `"profil.md"`, ...), with the value being an exact copy of the text
the file used to hold — field names, limits, and formats below remain unchanged, only the
container changed (see the module comment in `src/memory.rs`). `arsiv/` is the one
exception: it's still real `.md` files, because it's for human eyes only, the bot never
reads it again (see the table below). Migrating from an old `durum/` tree is done with
`cargo run -- migrate-durum` (`src/migrate.rs`).
Some persisted keys retain Turkish names for compatibility; they do not select the bot's language.
Person records use Discord user IDs. Topic records use slugs.

| Record | Writer | Reader | Limit / maintenance |
|---|---|---|---|
| `INDEX.md` | `refresh_index` (diarist, summarizer, startup) | every reply (system message), coach | ≤40 people, ≤30 topics, ≤3 months; derived record, don't edit by hand |
| `profil.md` | profiler | every reply, news_agent, wanderer, coach | regenerated every 6 hours (max_tokens 1200) |
| `huy.md` | coach | every reply, wanderer selection, sleep (tension) | evolves every 6 hours (800 tokens) |
| `duzeltmeler.md` | critic | every reply | rewritten at the end of every chat (400) |
| `kendim.md` | diarist (if the `kendim` field is filled) | every reply, coach, sleep (tension) | single block, overwritten |
| `gundem.md` | wanderer | every reply (last 3), coach (last 3) | 12 entries; older ones go to `arsiv/gundem.md` |
| `kisiler/<id>.md` | diarist, summarizer | `retrieve` (people in the chat), index | >1800 chars → summarized, target 1000; the old version goes to `arsiv/kisiler/<id>.md` |
| `konular/<slug>.md` | diarist, summarizer | `retrieve` (keyword match), index | >1500 → summarized, target 800 |
| `olaylar/YYYY-AA.md` | diarist, summarizer | `retrieve` (last 8), index | >6000 → the oldest 60% of lines collapsed to 3-5 lines; moved-out lines go to `arsiv/olaylar/YYYY-AA.md` |
| `arsiv/…` | archive | human | append-only, headed `## tarih öncesi` |
| `kanallar/<id>.md` | `channel_note` (every message, including the bot's own) | startup, `start_chat` seed | last 60 lines; the database record is replaced on write |
| `model.md` | `/model` | startup (`main`, overrides env MODEL) | single-line model id |
| `dusunme.md` | `/dusunme kip:göster/gizle/sessiz/kapat` | startup (`State::load`; defaults to göster if file absent) | `goster`, `gizle`, `sessiz`, or `kapali` |
| `debug.md` | `/debug durum:aç/kapat`, settings panel | startup (`State::load`; defaults to off if file absent) | `acik` or `kapali`; when on, decision traces get posted to the channel |
| `gelisim.md` | check_growth, pick_name | startup (`growth::load`) | lines `dogum: unix` `sohbet: n` `mesaj: n` `evre: i` `isim: ad` |

## Formats

### kisiler/<id>.md
File name is the Discord user id (a name change doesn't cause a split).
```
# Emin
id: 259669117248864257
kullanici_adi: kaju
eski_adlar: önceki görünen ad
puan: +3
etiket: rust, oyun
not: rust'ı övdüm diye üç mesaj laf soktu

## Bildiklerin
- yks'ye hazırlanıyor
- otosaray diye bir projesi var

## Son olaylar
- 2026-09-01 22:14:03: rust vs go tartışması, bot kaçtı
```
`puan` ranges -10..10 (`OWNER_ID`, if configured, is fixed at +10). `etiket` ≤6, lowercase.
`not` is a single sentence, an opinion. `eski_adlar` ≤5. `Bildiklerin` is non-repeating
durable facts. `Son olaylar` is one line per finished chat, with second-precision
timestamps (`date_time`). Parsing is done by `Person::parse`: `# ` heading, the `id:`
`kullanici_adi:` `eski_adlar:` `puan:` `etiket:` `not:` fields, the `## Bildik…` and
`## Son…` sections, and `- ` lines. Unknown lines are ignored. Name→id resolution goes
through `State.name_to_id`; a record that can't be resolved is skipped for that round
(logged). Old slug files are not read and can be deleted over time.

### konular/<slug>.md
```
# otosaray projesi
etiket:

- 2026-09-01: emin model eğitimi için veri toplamaya karar verdi
```
The `etiket:` line is left empty for now; `retrieve` searches for the keyword across the
entire content.

### olaylar/YYYY-AA.md
```
- 2026-09-01 #genel: synthetic example of a finished conversation
```
Once summarized, summary lines that don't start with `- ` appear at the top of the file;
`- ` lines are raw records (`refresh_index` only counts `- ` lines).

### gundem.md
```
## 2026-09-01 14:20
(diary entries in the bot's own voice, up to 10 lines)
```

## Slug rule
`memory::slug`: lowercase; ç→c ğ→g ı→i ö→o ş→s ü→u â→a î→i û→u; non-alphanumeric collapses
to a single `-`; `bilinmeyen` if empty. This is used for topics, not person keys.
Person records are keyed by Discord user ID; display names remain readable fields inside records.

## Budget (memory that goes into one reply)
`retrieve`: person files (≤4 × ≤1200) → topic (≤2 × ≤800) → last 8 events → ≤12 lines from
raw memory (≥2 keywords, ≤200 chars). Total ≤6000 characters; stops at whichever section
doesn't fit. The index and profile are not counted in the budget (they're the fixed part
of the system message).

## Retention and deletion

Channel history has a rolling 60-line limit; retrieval is bounded separately. Summarized records
can retain derived facts and archived raw text after a chat ends. Archives are not searched by the bot.
Selective member deletion is not supported. `/reset` clears channel/chat state, not person/topic
records or archives. Operator backups may retain older data. See [setup privacy](setup-guide.md#privacy).
