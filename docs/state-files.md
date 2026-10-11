# durum/ records

Runtime memory; not committed to git. The bot loads from here on restart.

**Memory storage**: logical memory records live in `durum/hafiza.redb`
(redb — pure Rust, ACID transactions; why this instead of rusqlite/plain-JSON:
`docs/decisions.md`). Adapter session files are separate, as described below.
Records keep path-like keys and their existing markdown
bodies, but schema v2 qualifies durable user/room/space identities by platform.
For example `kisiler/discord:user:42.md` and `kanallar/discord:room:7.md` are
active database keys. `platform::keys` is the sole identity-to-record mapping.
`arsiv/` remains real files for human inspection; new canonical archive filename
segments are escaped for filesystem safety, while historical archives retain
their original names and raw content. Filesystem import uses
`cargo run -- migrate-durum` and emits canonical keys, never v1 numeric aliases.
Some persisted keys retain Turkish names for compatibility; they do not select the bot's language.
Person records use platform-qualified user identities. Topic records use slugs.

| Record | Writer | Reader | Limit / maintenance |
|---|---|---|---|
| `INDEX.md` | mutation transaction, explicit refresh, startup | every reply (system message), coach | ≤40 people, ≤30 topics, ≤3 months; derived record, don't edit by hand; browser pagination has no such truncation |
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

## Matrix session and sync state

Matrix stores native runtime state beneath the state directory (normally
`durum/`), separately from the shared memory database:

| Path | Contents / purpose |
|---|---|
| `matrix/<connection>/session.json` | saved authenticated Matrix session, including access token and stable device/account identity |
| `matrix/<connection>/sync.json` | last sync token, so a restart resumes incremental sync rather than starting from scratch |
| `matrix/<connection>/rooms.json` | rooms approved by owner invitations; environment allowlisted rooms are explicit approval too |
| `matrix/<connection>/admin-room.json` | private plaintext owner-only administrative result route; excluded from conversational learning |
| `matrix/<connection>/commands.json` | last 256 accepted native command event IDs, preventing command replay on reconnect/restart |
| `matrix/<connection>/store/` | Matrix SDK SQLite state/cache; SDK crypto is disabled |

`<connection>` is the configured credential-independent connection identity,
not the password, token, display name, or room ID. These are real files, not
redb record keys. Never run two bot processes against one Matrix store.

**Protect the whole state directory like a password file.** In particular,
`session.json` contains an access token that can act as the bot. Restrict Unix
directory/file permissions and Windows access control, and keep backups private.
Do not upload sessions, sync/store files, or real conversation data in bug reports.
The SQLite cache can contain room/account data even though it has no crypto keys.

Preserve this directory when restarting/replacing a container or binary.
Restoring the saved session keeps the device identity stable; deleting it can
require a fresh login/device and loses sync continuity. If credentials leak,
stop the bot, revoke the token/device using the homeserver/client account tools,
and reconfigure login privately. Deleting local files alone does not revoke a
server-side session or erase shared-brain notes and backups.

The adapter is plaintext only. No encryption key store, key recovery, or encrypted
room support is implied by the SDK cache. See [Matrix configuration](configuration.md#matrix-plaintext-only).


## Formats

### kisiler/<id>.md
The record key is a platform-qualified `UserKey`; display-name changes never split it.
```
# Emin
id: discord:user:259669117248864257
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
`puan` ranges -10..10 (the qualified `OWNER_KEY`, or legacy Discord `OWNER_ID` if configured, is fixed at +10). `etiket` ≤6, lowercase.
`not` is a single sentence, an opinion. `eski_adlar` ≤5. `Bildiklerin` is non-repeating
durable facts. `Son olaylar` is one line per finished chat, with second-precision
timestamps (`date_time`). Parsing uses the record key as authoritative identity;
historical `id:` lines cannot overwrite it. Other markdown fields and sections
remain unchanged. Unresolved legacy slug records are retained and reported in the
migration manifest, but are not invented as users or silently deleted. Names are
presentation, not a cross-platform identity-linking mechanism.

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
Person records are keyed by platform-qualified user identity; display names remain readable fields inside records.

## Budget (memory that goes into one reply)
`retrieve`: person files (≤4 × ≤1200) → topic (≤2 × ≤800) → last 8 events → ≤12 lines from
raw memory (≥2 keywords, ≤200 chars). Total ≤6000 characters; stops at whichever section
doesn't fit. The index and profile are not counted in the budget (they're the fixed part
of the system message).

## Revisions, manual edits and live caches

Editable records have a monotonic per-key token in `memory_revisions`; existing
records begin at revision **0**, and the first coordinated write creates revision
**1**. `modified` remains a seconds-resolution display timestamp, never a
conflict token. `read_versioned` reads body, revision and timestamp in one redb
transaction. Browser people/topic/month listings return full bodies in bounded
pages (1–100 records), ordered by descending modification time then ascending
canonical key; cursors retain both values. Searches are limited to 200 characters.
Context/index limits do not truncate administrative browsing.

The authenticated `/memory` screen browses complete people, topics and monthly
event documents; event detail includes both summary prose and raw event lines.
`INDEX.md` stays derived and read-only. Structured person edits protect immutable
identity and favorite +10/note, and treat bodies with unknown/noncanonical fields
as read-only rather than dropping material. JSON edits require a strong quoted
numeric `If-Match`; native forms carry a hidden revision. A 412 conflict preserves
the submitted draft beside current text rather than automatically merging it.

Manual replacement requires the displayed revision (`If-Match` for JSON).
A stale draft fails without changing memory or caches. An absent temperament can
be created at expected revision 0; competing creations conflict after revision 1.
Every displaced editable body, including its old revision and timestamp, is
stored verbatim in `memory_history` in the same transaction as its replacement.
Summarization therefore cannot lose raw material when an `arsiv/` filesystem
append fails. Filesystem archives remain inspection-only copies; transaction
history is retained independently. Neither is searched by the bot, and neither
manual edits nor reset erase prior material. This retention increases database
size and is not a member-erasure facility.

Person learning applies score/fact/tag/event deltas to the latest transaction
body, preserving canonical identity, bounded scores/tags, owner favorite rules,
and unknown original fields/subsections. Topic and event additions likewise
append to the latest transaction body. Profiler, coach, self-state, corrections,
agenda and summarizer prose carries the revision used before model work.
Changed revisions discard generated results; a later cycle uses fresh input
instead of silently overwriting the edit. Manual edits are **not permanent pins**:
a fresh agent run may legitimately evolve the newly saved revision.

All editable writers use a synchronous mutation coordinator, entered without a
Bot state/cache guard. Lock order is mutation gate → redb write transaction →
commit/drop transaction → short state/cache publication. INDEX is built from
that write transaction's view (no nested transaction), committed with the body,
then published with affected temperament/profile/corrections/self/agenda caches.
Failed commits and conflicts publish nothing. No model/network work, awaits,
archive append or recursive memory write occurs inside the gate. Model/thinking/
debug controls also publish only after their own durable coordinated commit;
they are not arbitrary editable prose records.


## Retention and deletion

Channel history has a rolling 60-line limit; retrieval is bounded separately. Summarized records
can retain derived facts and archived raw text after a chat ends. Archives are not searched by the bot.
Selective member deletion is not supported. `/reset` clears channel/chat state, not person/topic
records or archives. Operator backups may retain older data. See [setup privacy](setup-guide.md#privacy).

## Identity schema migration and rollback

Startup acquires exclusive redb ownership before state loading. A versioned
migration creates a consistent stopped-instance snapshot database and manifest,
then renames person/room records, converts valid numeric Discord scan entries,
removes superseded active keys and advances the schema in one redb transaction.
Original body bytes and modification timestamps are preserved. Unknown keys and
malformed scan lines remain in place and are reported as unresolved.
Pre-existing destinations, timestamp conflicts, duplicate renames or any backup/
database error abort before cutover; repeated completed migrations are no-ops.
`INDEX.md` is derived again only after successful conversion; its pre-migration
representation remains in the backup.

Rollback requires stopping the new binary, restoring the saved v1 snapshot and
using the old binary. Never run the old binary against the v2 identity scheme.
Archives are not migrated or removed. Retain the backup and manifest alongside
operator backups; this migration is not a member-erasure mechanism.

## Thread routes and captured identities

Unthreaded Discord history retains its canonical `kanallar/discord:room:<id>.md`
record and original text format. New topic/thread conversations use
`kanallar/<RoomKey>/threads/<ThreadKey>.md`; both keys are reversible typed
identities, not display-name slugs. Identical native topic IDs in different
Telegram chats or Slack channels remain different keys.

Each history has an aligned optional-user JSON companion under
`kanal-kimlikleri/` with the same room/thread suffix. History and companion are
written in a single transaction. Legacy history without metadata remains
unattributed; current display-name collisions never reinterpret its authors.
The companion preserves captured UserRef source, automation and identity
stability across restart and supports transcript-local name disambiguation.

## Telegram polling metadata

Telegram stores its next `getUpdates` offset as a `u64` in redb's
`memory_metadata` table under `platform/telegram/<bot-account-id>/offset`.
This is metadata, not a Markdown memory record, a filesystem cursor file or
remote group history. The native bot account ID scopes it independently of
token rotation and connection labels; the token is never part of the key.

The runner checkpoints after normalized events are accepted by bounded ingress
and administrative commands are queued, **not** after model work completes.
Restart resumes from the checkpoint. A crash before checkpoint can replay an
update; a crash after checkpoint can leave accepted/queued work unfinished.
This is not an exactly-once processing guarantee. Local conversation and
captured identity records remain separately persisted as described above.

Back up metadata with the database. Do not delete or manually increase the
offset to “fetch history”: the Bot API cannot fetch old group history, and its
pending update queue expires after at most 24 hours. Telegram records use the
same canonical identities, for example `kisiler/telegram:user:42.md` and
`kanallar/telegram:room:-100123.md`, and only explicitly approved rooms may feed
the shared brain.

