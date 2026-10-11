# Flows (event → what happens, step by step)

The native-platform notes below describe the shared adapter path. The detailed
message/reaction walkthroughs, Discord UI, slash interactions and streaming
presentation later in this document retain their Discord-specific behavior;
Discord intents, guild filters, embeds, spoilers and buttons are not requirements
for the other adapters.

## Native platform ingress

- **Slack Socket Mode:** reserve capacity in the bounded 256-entry event worker
  queue **before** acknowledging an Events API envelope; publish it to the worker
  only **after** the ACK succeeds. A full queue leaves the envelope unaccepted and
  unacknowledged, eligible for Slack redelivery. Saturation does not stop the
  adapter or discard its already accepted backlog, and the socket loop continues
  handling frames, including heartbeat pings, without waiting for enrichment.
  Worker event deduplication applies to accepted payloads. ACK means acceptance,
  not completed command/model work; neither redelivery nor exactly-once processing
  is guaranteed.
- **Matrix native commands:** approved conversational rooms and the private
  administrative room both use the same 16-entry, single-consumer command queue.
  Admission waits for bounded capacity and observes shutdown rather than silently
  dropping a command on overflow. If shutdown interrupts admission, the current
  sync batch is not committed past that unaccepted command. Orderly shutdown
  closes admission and drains accepted commands. The last 256 accepted event IDs
  are persisted for bounded replay deduplication, not as a crash-durable work
  queue or exactly-once execution guarantee. The committed sync cursor, SDK rewind
  and replay limits are described in [Matrix session and sync state](state-files.md#matrix-session-and-sync-state).
- **Matrix notices:** `m.notice` is excluded during normalization, even if it
  mentions or replies to the bot. It enters neither conversational learning nor
  automatic replies, and is not parsed as a native command. Ordinary supported
  `m.text`, `m.emote` and `m.image` events retain their normal handling.

## Shared reply routing, feedback and live protocol

- Before publishing any part, the streaming and non-streaming reply senders call
  `route_reply(&self, room: RoomRef, reply_to: Option<&MessageKey>) -> PlatformResult<RoomRef>`
  **once for the whole outgoing layout**. This is a synchronous adapter operation;
  the default returns the owned room unchanged, without extra copying/allocation.
  Slack retains an existing thread, or resolves a top-level reply target into the
  destination thread. Every answer line, chunk and Show-mode thought part then
  uses that same route. Thread routing is independent of a first-part-only visual
  reply reference; later parts do not fall back into the main channel.
- For Slack reaction feedback, `reactions.get` looks up the exact channel and
  message timestamp, including bot replies in threads that channel history does
  not return. It uses the existing `reactions:read` scope, validates the returned
  target, and supplies its author, normalized text and actual thread to the
  shared reaction handler. Eligible reactions to this connection's own bot
  messages enter raw memory, that conversation's channel history and any open
  chat as social feedback; they never trigger an automatic reply.
- Native Telegram and Slack live renderers apply shared `stream_slice` and then
  `parse_reply` **before** platform chunking/publication. Only parsed reply lines
  are published: silence markers and `tepki:` reaction-control lines are not
  transient public text or notifications that have to be deleted later. Explicit
  Show-mode thought presentation is separate from reply-protocol text; it is not
  passed through the reply parser. Platform text limits and thought presentation
  still come from each adapter, not Discord's spoiler/button UI.

## A message arrives (Discord walkthrough)
0. Every message (the bot's own included, via `send`) lands in the channel's history:
   `channel_note` → memory (60 lines) + `durum/kanallar/<id>.md`. When a new chat opens, the
   last 10 lines become its seed (`start_chat`), so context isn't lost even if the chat has
   ended or the bot restarted.
0. **Commands never enter this flow.** The bot is managed only through slash (`/`) commands;
   `Handler::message` no longer parses text as a command, every message goes straight into the
   steps below. Slash commands have a separate flow: Discord sends `Interaction::Command`,
   `interaction_create` (main.rs) looks the name up in the `command::definitions()` table and
   calls the matching handler — see the "Slash commands" section.
1. `Handler::message`: returns for bot/webhook/DM; returns for a guild/channel outside
   `GUILD_ID`/`CHANNELS` when those are set. `content_safe` (mentions become `@name`,
   `@everyone` is neutralized).
1b. **Image attachment:** the URL of the first attachment in `msg.attachments` whose
    `content_type` starts with `image/` is taken. The early-return condition is now "neither
    text nor an attachment", not "text is empty": a message that's only an image still gets
    processed. The text going to memory/channel note/chat line is marked: `[resim] <text>` if
    there's text, `[resim attı]` if not. The URL only goes into the chat history's
    `ChatMessage.image` field and stays **only on the latest user message** (adding a new line
    sets earlier entries' `image` to `None`: a discord cdn link has a short lifetime, and
    resending an old image every turn burns tokens). If `message_json` sees this field, the
    request body's `content` becomes a `[{text},{image_url}]` array instead of plain text (same
    shape as agents.rs's `image_commenter`).
2. Under lock: was it tagged? (mention list ∪ the replied-to message is the bot's ∪ the bot's
   name appears in the text)
3. `remember` (raw memory), `last_channel`, favorite's name.
4. If it posted news and 2 hours have passed, close that chat silently (no ban).
5. **If asleep:** if tagged, queue it (≤20), no reply, return.
6. If tagged, or it's an **ongoing dialog** (a chat is open AND the sender of that chat's last
   user message is the same person who sent this one — genuinely talking to them), reply
   directly. Otherwise (no chat, or SOMEONE ELSE wrote in the channel), a **willingness
   evaluation**: at most once every 2 minutes per channel, a mini model call (`isteklilik.md`,
   ~80 tokens) produces `{"puan":0-10}` from the last 12 messages + profile + index; joins the
   chat if above the threshold (`WILLINGNESS_THRESHOLD`, ±1 by stage confidence, +2 while
   traveling). A fallback dice roll (`CHANCE`) if the call fails. This prevents auto-replying to
   EVERYONE in an already-open chat's channel — only to its actual counterpart.
7. If a chat is open, add the user line to history (last 20).
8. Outside the lock: `reply`.

## A reaction was added (Discord walkthrough)
`Handler::reaction_add` (needs the `GUILD_MESSAGE_REACTIONS` intent) fires on every reaction,
but only cares about ones on the **bot's own message**, from a human, in a guild (not a DM), and
passing the `GUILD_ID`/`CHANNELS` filter — everything else returns immediately. The `Reaction`
event carries neither who reacted nor the reacted-to message's text: both are fetched over HTTP
via `add_reaction.user`/`.message`. A reaction on a message with empty text (embed-only) is
skipped — a reaction to a card/status line doesn't count as "to the bot's own words".
`reaction_label` makes the emoji readable: unicode as-is, a custom guild emoji as `:name:` (not
Discord's raw `<:name:id>` mention form). The result lands as a
`"(tepki 💀) \"...\" mesajına tepki verdi"` line in `remember` (raw memory) and `channel_note`;
if a chat is open in that channel it's added to `chat.history` too (the model sees this as
context in its next reply). **It never triggers a reply on its own** — no willingness
evaluation, no new message goes out; the bot only notices, and may bring it up in its next
natural reply. Logged as `tepki: <name> → <emoji>` if `debug` is on.

## Output protocol (every reply with personality goes through this)
The model writes not plain text but a **line-based protocol**; `parse_reply` decodes it
(on text that already had `strip_name` applied, no re-stripping):
- **Every line is its own discord message.** Blank lines are dropped, at most `BURST_LIMIT` (4)
  lines go out; the rest are dropped (debug log). A line over 1900 characters is split within
  itself by `split`.
- **A `tepki: 💀` line does NOT go out as text**, it becomes a reaction emoji on the message
  being replied to. Case and "tepki :" spacing are tolerated; the first emoji run after the
  colon is taken (letters, whitespace, and one character from a known emoji block — U+2600–27BF,
  U+2B00–2BFF, U+1F000–1FAFF, singles like ©/®/™ — plus a trailing variation
  selector/ZWJ/keycap, up to 8 chars). The definition is deliberately narrow: `—`, `…`, `→`,
  and typographic quotes are not emoji, Discord returns a 400 for those. A custom `:kekw:`-style
  emoji, and a line with no emoji at all, are silently dropped. The first reaction wins.
- **Silence:** a line that's just `-` (or `"-"`, `'-'`, `[sus]`, `(sus)`) sets the `silent` flag
  and doesn't go out as a line. Nothing is sent only if `silent` is set alone.
- **Stray bits and slop:** a line starting with `'` (the continuation of a previous message) is
  dropped; `clean_slop` strips a leading `- `/`* `/`• ` bullet and `**`/`__` marks (the
  backtick itself and its CONTENTS are preserved: `` `__init__` `` isn't mangled). A `1. `/`2) `
  number prefix is stripped only when the reply has **≥2 numbered lines** (a real list) — a
  single line like "3. sınıftayım" is a Turkish ordinal. A line identical to one already seen
  this turn doesn't go out a second time. **A short line is never filtered out**: "he", "yok",
  "la" are natural reactions.
- The form that lands in history and the channel note is `Reply::protocol_text()`: lines joined
  by `\n`, with `tepki: 💀` at the end if present. So the model sees its own format again next
  turn.

## reply (one chat turn, Discord streaming presentation)
```
lock ── busy? exit ── chat exists? ── pick instruction ── busy=1 ── release lock
wait 0.15-0.35s ── fresh history + last message + pending arrivals ── mood (every 4 turns) ── research(link/news/lookup) ── target selection (if 2+ people wrote) ── question cap (too_many_questions) ── typing…
generate_stream(stream, budget: reply_budget!) ── (error: busy=0, exit)
send_stream: the message opens on the first MEANINGFUL content (as long as the layout is still empty, "first" isn't spent; stream_slice holds back a short partial line) ── edited every STREAM_EDIT_INTERVAL (1.2s) ── while thinking (answer hasn't started): show="Düşünüyorum...", hide=live word counter, silent/off=nothing (the message never opens until the answer starts) ── once the answer starts, the same message is edited as it streams ── show: thinking is one newline-less line, both a spoiler and a code block ── hide: thinking isn't in the message, a "Düşünce Sürecini Göster" button appears at the end of the answer (interaction_create opens an ephemeral code block for whoever clicks it, thought storage holds 50 messages) ── silent: reasoning is requested (runs in the background) but never collected/shown, no button either ── off: the request carries no reasoning ── the discord reply attaches only to the first message
the part shown WHILE streaming: finished lines (followed by \n) + the last partial line, but only once it passes HALF_LINE_THRESHOLD (12) characters (stream_slice) ── so a half-formed "tep" doesn't become a message that then gets deleted
once streaming ENDS, parse_reply:
  silent ∧ no line ∧ NO REACTION EITHER → the temporary messages opened are deleted, StreamResult::Silent (nothing enters history, the counter doesn't advance, last_activity isn't refreshed, the fallback generate is NOT called; hacked still decrements) ── if "-" and "tepki: 💀" arrive together it's not silence, the emoji still lands
  nothing at all → StreamResult::Empty → non-streaming fallback generate + line-based repeat filtering + send_reply
  is_repeat is LINE-BASED: lines identical to one of the last 5 bot lines are dropped; if no line is left and there's no reaction either, regenerate once, and if that repeats too (or the new reply has neither a line nor a reaction), delete what was opened and Empty
  the final layout is written with write_stream (extra messages are deleted) ── if there's a reaction, create_reaction on context.reaction_target's message (error is a warn log, the stream doesn't stop; a reaction alone is still a valid reply)
if 2+ different people wrote in a row, a TARGET_PICK mini call picks the target person; the reply is linked to their message, and "address them" is noted in the instruction
if a new message arrives during generation, the stream still finishes (no starting over); the new message is handled on the next turn
lock ── busy=0 ── every visible line goes into own_messages one at a time, all of them into channel_notes with a SINGLE file write (the reaction as a "bot: tepki: 💀" line) ── the assistant line = protocol_text ── counter++ ── hacked-- ── release lock
… exit if no new message, otherwise loop again
```
Instruction priority: hack continuing > hack exiting > empty. Added on top: mood, an internet
finding, a target-person note, the question cap.

## Question cap
`too_many_questions(state, channel)`: if ≥2 of the channel history's last 4 bot lines (`tepki:`
lines don't count) end in `?`, "Bu sefer soru sorma; düz laf et ya da sus." is added to the
instruction. Code measures, the model enforces — no cutting/trimming. Both `reply` and the CLI
chat mode apply it.

## send_lines (the non-streaming paths)
`strip_name` + `parse_reply` → `send_reply` (the body; paths already holding a resolved `Reply`
call it directly) → lines go out one at a time as separate messages. Between them,
`300 ms + 15 ms × character` (capped at 1500 ms) of delay plus `broadcast_typing` — none of the
stream's own pacing applies here, so three messages don't all land at once. The discord reply
attaches only to the first line; the ping too, but **only after the protocol is decoded**,
prepended as `<@id> ` to the first line's front at send time — pasting it in beforehand made `-`
and `tepki:` lines unrecognizable. A reaction is sent and written to the channel note in
protocol form if a target was given; **without a target, the reaction is dropped** (so a
reaction that wouldn't be visible in the channel isn't counted as "sent"). **Nothing is sent**
for `silent`, or for a reply with nothing left to send; `None` is returned — the opening senders
(poke, sorun, news teaser, welcome, woke up, waking reply, on the road, leaving, name
announcement) skip that turn, no chat opens. The `protocol_text` it returns becomes the chat's
opening seed text. `run_prank` sends the image and text in a single message, so it keeps only
the protocol's first line.

## Chat lifecycle
- Opening sources: random drop-in, a tag, welcome, news sharing, poke, prank, waking up, a
  message from the road, a leaving announcement. Ones with an opening start with `counter=1`.
- No message limit and no farewell: a chat closes silently 30 minutes after the last message
  (`close_timed_out` on the minute tick), no channel ban. A closed chat's transcript goes to the
  diarist and the critic.
- The ban only blocks *dropping in* — a tag always gets a reply.
- If the model call errors, the counter doesn't advance and the chat stays open.

## generate (every call with personality)
1. `name` (before ": ") and text are parsed out of the `user` lines in history.
2. `memory::keywords(texts)` → ≤40 words.
3. Locked: `memory::retrieve(participants, name_to_id, keywords, raw memory, 20)` → the
   budgeted context; `system_text`.
4. `ask` → `clean` (name prefix, quotes, 1900).
Chat replies don't use this; `generate_stream` builds the same system and opens a stream
(`send_stream` writes it), with no cutting.

## CLI chat (`cargo run -- chat`)
A terminal rig for trying out the protocol without ever connecting to Discord
(`src/chat_cli.rs`).
```
main: is the first argument "chat"? → Bot::setup() (DOESN'T NEED DISCORD_TOKEN, only a model key)
  no key → "chat mode failed to start: <reason>" + exit code 1
bot_name empty (ready never fires here) → growth.name, then "bot"
start_chat(ChannelId::new(1)) — seeded from the real durum/ files, personality stays realistic
loop: a stdin line "name: text" (no colon, or either side empty → sender is "misafir") · !quit or EOF → exit
  remember + channel history (IN MEMORY ONLY, append_history) + a user line added to chat history
  question-cap instruction ── generate (NO streaming) ── strip_name ── parse_reply
  output: each line as "bot_name: line" · reaction as "[reaction 💀]" · silence as "(silent)" · nothing as "(empty)" · a model error as "(error: …)" and the loop continues
  protocol_text appended to history, counter++
```
Nothing is written to the actual state: `channel_note` is replaced with the in-memory
`append_history`, and neither the agents nor the cycles ever run in this mode. (One exception:
since `Bot::setup()` is shared with the live path, it still creates the empty
`durum/{kisiler,konular,olaylar,arsiv,kanallar}` and `photos/` folders.) **Unverified:** there's
no real model key on this machine, live back-and-forth has never been observed (see AGENTS.md
"Known gaps").

## Slash commands (command handler → embed → detail modal)
`ready` → registers with every guild (`modal::register_commands`, idempotent): the list is
derived from the `command::definitions()` table (name/description/options in one source, never
hand-kept in two places) → the user runs the slash command → `interaction_create(Command)` →
looked up by `cmd.data.name` in the table, the matching handler is called. Every command returns
an **embed**, never plain text:
- Local/fast commands (`durum, yardim, ayarlar, zihin` default view, `sifirla, dusunme, model`
  query, `debug`) reply directly with `CreateInteractionResponse::Message`
  (`send_response`/`reply_info`, embed via `modal::info_embed`).
- Commands that make a network/model call (`haber, sorun, gez, saka, hack, ajanlar, uyan, uyu,
  zihin test:true, model id change`) could exceed Discord's 3-second initial-reply limit, so they
  acknowledge instantly with `defer` (`Defer`), then write a short result embed with
  `report_result` (`edit_response`) once the work is done — the actual content (news/joke/etc.)
  already went to the channel via its own `Bot::send` call, this is just an "OK" note.

`/zihin` card: three columns (People/Topics/Events) + a person select menu at the top,
Topics/Events/Bot-summary buttons at the bottom.
Picking a person from the menu or pressing a button → `interaction_create(Component)` → the
matching **detail modal** (`person_modal` / `topics_modal` / `events_modal` / `summary_modal`);
each section in its own labeled field, nothing dumped into one box.
If the user submits the modal → `interaction_create(Modal)` → a brief ephemeral confirmation; no
input is collected.
`/zihin test:true` replaces the old panel-diagnostic path (see "Mind chain" below).

## On connecting to a guild
`guild_create` (once per guild) → in the background: a 14-day backfill scan (permitted
channels, pages of 100) → the last 2000 lines of raw memory → profiler → coach (if temperament
is empty). Not re-scanned on a reconnect.
`guild_create` also sends a one-line version announcement to the default channel, once per
process (`Handler.announced`): `geldim · v1.0.0 (69e2851, 2026-09-02) · model … · düşünme …` —
this doesn't get written to memory or the channel note (so the bot doesn't mistake it for its
own words). Not in `ready`, because the guild cache isn't populated yet at that point.

## The 6-hour round (news_cycle)
not awake → skip · traveling → profiler, coach, skip · profiler → diarist("observation", last
300) → coach → skip if a chat is already open in the channel → news_agent (HN 12 + Sözcü 12, not
yet posted) → selection → teaser (`generate`) → send → open a chat, wait 2 hours for comments,
mark the news as "posted".

## Poke (hourly)
25% (`PROBLEM_SHARE`): `post_problem` to the default channel (a made-up code problem + a
question), a chat opens. Otherwise, the flow below.
not awake → skip · traveling: skip if it already wrote today, 25% → `ON_THE_WAY` · travels
tomorrow: `LEAVING` once · otherwise 30% → `OUT_OF_THE_BLUE` · skip if no `idle_channel` →
`generate(last 40 lines)` → send → open a chat.

## Prank (every 3 hours, 10%)
awake ∧ not traveling ∧ an idle channel ∧ `photos/` isn't empty → 30% hack (`HACK_ENTER` text +
image, chat `hacked=3`: 2 turns of `HACK_CONTINUE`, 1 turn of `HACK_EXIT`) · 70% a plain image
(`image_commenter`: the model actually looks at the image).

## Agenda wandering (10 min after startup, then every 4 hours)
rss first 20 → pick (`WANDERER_PICK`, temperament+profile) → up to 3 pages (`read_page`:
firecrawl or plain) → `generate(WANDERER_NOTE)`, the bot's own journal entry → `gundem.md` (12
entries, the oldest goes to the archive) → `State.agenda` = last 3 → the "GÜNDEM" section of
every reply, and coach's input.

## Sleep (every minute)
`/uyan`: `forced_awake_until` runs until the current plan's end (deleting the plan doesn't work
— it gets rebuilt a minute later and puts the bot back to sleep). `/uyu [hour]`: a temporary
plan, the forced flag is reset.
`update`: if there's no plan for yesterday+today, build one (20% if tense, 7% otherwise, for a
sleepless night). The awake→asleep / asleep→awake transition is logged. Being asleep never
enters the chat prompt as an in-character excuse.
**Listening continues while asleep:** messages still go into raw memory; `memory_cycle` runs a
night observation into the mind every 2 hours; the news round picks news while asleep but
doesn't post it, stashing it in `stashed_news`.
**On waking:** if there's a pending mention, a definite reply via `WOKE_UP` (put back on error,
never lost). With no mention, the `uyanis.md` agent evaluates the night's messages
(`{"ilgi":0-10,"konu"}`); if interest ≥5, a morning line via `uyanis-cevap.md` to the last
channel talked in. A stashed news item is posted as "morning news" on the first turn awake.

## Travel (from the calendar)
`travel::now()` looks today up in the table. Effects: the "ŞU AN" line, drop-in chance ×0.3, no
news/pranks, at most 1 message from the road a day instead of poke, `LEAVING` one day ahead. No
persisted state; only the `last_road_message` and `announced_trip` flags.

## Growth
Every finished chat is `growth.chats++`, every message is `growth.messages++`. `check_growth`:
the stage only ever advances (new → warming-up → settled-in → old-timer) based on day and chat
thresholds. Stage effects: the "GELİŞİM EVREN" section in the system message, drop-in chance ×
stage.confidence, poke × stage.poke. On entering the settled-in stage it picks a name once: the
model gives a single word, the nickname changes in every guild, it becomes `bot_name`, and it's
announced to the group. Counters live in `durum/gelisim.md`; a restart doesn't reset them.

## Finished chat → memory
A chat quiet for 30 minutes is closed by `close_timed_out`; the transcript lands in
`State.memory_queue`. `memory_cycle` (every 10 min, independent of sleep) processes the queue:
`diarist`'s JSON → an `olaylar/AA.md` line (with seconds), person files keyed by id (names
resolved via `name_to_id`; score, note, facts, tags, events), topic files, `kendim.md`,
`INDEX.md` → `summarizer` shrinks whatever's over the limit (archiving it) → `critic` also runs
for a finished chat → `duzeltmeler.md`. The 6-hour round's observation goes through the same
queue.

## Mind chain (chat → diarist) and diagnostics
`close_timed_out` → info log
`mind: chat closed [channel] (30 min quiet) → queued (n), diarist within 10 min`
→ `memory_cycle` (10 min) → `diarist` → info
`mind: diarist [source]: k person(s), m topic(s), o event(s) written`
or warn `mind: diarist failed [source]: <reason>`. `diarist` now returns
`Result<DiaristSummary, BotError>`.
`/zihin test:true`: hands the channel's last 30 lines straight to the diarist, writes the result
as a single message (without the 40-minute wait).
On a reasoning-mandatory model (glm-5.3-flash), `ask_raw`: a 400 "mandatory" → the fields are
stripped + `reasoning.effort=low` on openrouter + the budget raised to max(2×, 1500); a 200 with
empty content → for JSON-expecting categories (gunlukcu, isteklilik, hedef_sec, ruh_hali,
uyanis) the `{…}` content inside the thought field is counted instead (warn log), not counted
for a plain-prose call; if it's still empty the budget is grown and it's retried once more; the
error message includes category/model/budget/thought length.

## Debug mode (`/debug`, settings panel)
While `State.debug` is on, `debug_note` writes a single line (⚙ …, ≤300 chars) to
DEBUG_CHANNEL, or to the message's own channel if there is none, and info-logs it; it never
enters memory or the channel note. Traces (in English, for developer diagnosis): the message
decision (`tag` / `dialog ongoing` / `willingness p/threshold · reason: … → reply|silent` /
`2min limit` / `fallback die`), the reply turn (`mood`, `target`, `question cap`, `n line(s)
sent · reaction X` / `silent (-)` / `stream empty → fallback generate`),
`sohbet kapandı (30 dk sessiz)`.

## Settings panel (`/ayarlar`)
An embed (version, model, thinking, debug, sleep, travel) + buttons: thinking
show/hide/silent/off (the active one is Primary), debug on/off, wake up / put to sleep (8
hours). Button → `interaction_create(Component)` `setting_*` → `Handler::setting_button`: the
same paths as the commands (`ThinkingMode` + dusunme.md, `set_debug`,
`wake`/`put_to_sleep` + `sleep_transition`) → the panel is refreshed in place with
`UpdateMessage`. The reply is ephemeral (visible only to whoever pressed it).
