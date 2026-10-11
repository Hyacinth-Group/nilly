# Decisions and rationale

This is a dated historical record, not the current behavior reference. Some entries were
superseded; consult [architecture](architecture.md), [configuration](configuration.md) and
[state formats](state-files.md) for maintained contracts. Earlier session notes remain in Git
history, not the distributed documentation.

- **2026-09-01 · Python → Go → Rust.** The language changed twice at Emin's request. Settled on
  Rust; serenity 0.12 + tokio. The Go version is in git history (`git log --all -- main.go`).
- **No SDK for OpenRouter, raw JSON with reqwest.** A single `sor_ham` function; for image input
  (`image_url`) it's easy to build the request body by hand, dependencies stay minimal, and what's
  sent is visible.
- **Prompts as `.md` + `include_str!`.** Emin's request; editing text is kept separate from
  editing code, and the heading line gives the model context. Cost: a change requires a rebuild.
- **Personality isn't static, agents write it.** Core rules are fixed in `kisilik.md`; temperament
  (hoca), corrections (eleştirmen), opinions and knowledge (günlükçü), agenda view (gezgin) come
  from files. Rationale: the request that "the bot build its own personality" — personality doesn't
  grow from a single prompt.
- **Agents are personality-less (`analiz`).** In profile-extraction and selection tasks, persona
  added noise.
- **Opinion JSON → person files.** `kanaatler.json` was a single file, kept growing, and was sent
  on every reply. Second-brain architecture: the index goes on every reply, a person file only when
  that conversation needs it.
- **Nothing is ever deleted, it's summarized; the raw piece goes to the archive.** A rule from
  Emin's own second brain.
- **Limits are enforced in code.** The model can't be trusted on score/length/format; clamp,
  truncate, "don't touch it if it hasn't shrunk."
- **Only one reply generated per channel at a time (`mesgul`).** So spam doesn't inflate the API
  bill and replies don't overlap each other. Messages arriving meanwhile fall into history and are
  seen next turn.
- **Mentions are disabled.** The model could write `@everyone`; the only exception is the welcome
  ping.
- **No replies to bots/webhooks/DMs.** Bot-bot loop.
- **The person key is the display name, not the id.** The model sees names in the transcript, not
  the id; the file name should be readable. Cost: two people with the same display name collide (a
  known gap).
- **The favorite user is hardcoded (+10).** Emin's request; no matter what the model says.
- **Date/time without an external library.** The Hinnant algorithm is 15 lines; not worth a chrono
  dependency. No Turkish daylight saving time, fixed +3.
- **Sleep and travel hold no state.** The calendar and clock are enough; restart consistency comes
  for free. Only the sleep plan (random ±45 min and the insomnia dice roll) lives in memory and is
  re-rolled on restart (accepted).
- **Insomnia chance depends on personality.** If tension-related words appear in `kendim`+`huy`, 7%
  → 20%. The dice roll wasn't delegated to the model; the model is bad at randomness.
- **While traveling, agents keep running, but news/jokes stop.** Learning shouldn't be interrupted,
  but someone "checking from their phone" doesn't post news.
- **The hack joke forbids asking for links or information.** So the joke doesn't resemble real
  phishing.
- **Images come from the `photos/` folder, outside git.** Discord CDN links die within a day;
  personal screenshots shouldn't leak into the public repo.
- **`durum/` is outside git.** It contains personal data (notes about friends).
- **The repo is public** (Emin's decision).
- **Growth stages advance by day + chat thresholds, only forward.** A newly arrived bot shouldn't
  talk like an old-timer from day one; the stage changes both tone (prompt section) and boldness
  (chance multipliers).
- **It picks its own name, once, at the "settled" stage.** The Discord nickname changes (requires
  permission), the old username stays in tag detection so it still understands when people address
  it by the old name.
- **Mistral support without a separate SDK.** The API is OpenAI-compatible; only the
  address/key/model change. Selection comes from `.env`; if both are present, OpenRouter wins,
  `SAGLAYICI=mistral` forces Mistral.
- **2026-09-02 · Three layers against slop.** On the first live night the bot wrote like a
  therapist, 4-5 sentences ("relax, have a coffee, go with the flow"). The fix wasn't trusting the
  prompt: (1) cut in code, with `kisalt` to 2 sentences and 2x the group's average length;
  `max_tokens` 90; (2) human pace: 2-6 sec reading time, "typing…" + 45 ms per character; (3) a
  forbidden-pattern list in the prompt, real examples, and 12 of the group's own recent messages in
  every reply as a "length and tone example."
- **No few-shot example sentences.** On the first night the bot copied the example reply in the
  prompt ("napıyım yavaş mı yazayım") word for word. The example pairs were removed; the tone
  example now comes only from the group's own real messages (`ornek_mesajlar`). A "don't force
  slang" item was added.
- **Reply reference depends on context** (2026-09-02): sending every reply as a Discord reply
  looked robotic; when talking one-on-one it writes plainly, in a crowd/when tagged/when a message
  slips in between it replies.
- **The reply goes out as a Discord reply.** The request "tag whoever it's replying to"; instead of
  writing the name, `reference_message` + `replied_user`, so who's being addressed is clear in a
  crowded channel. History is re-fetched after a short reading allowance; in a message flood it
  doesn't reply to a stale line.
- **The model can be changed at runtime, `!model`.** Only the FAVORITE; verified against the
  OpenRouter list; `durum/model.md` overrides the env. Test commands (`!haber` etc.) are for
  everyone, it's their server.
- **2026-09-02 · Channel history to disk, new conversations seeded.** "Goldfish memory": when a
  chat closed at 12 messages and on restart, everything vanished — the bot's own messages weren't
  even in the raw memory. Now the last 60 lines per channel (including the bot) live in
  `durum/kanallar/`, and a new conversation opens seeded with the last 10 lines.
- **Length has three tiers.** An ordinary remark is 2 sentences; a question/medium message is 3; a
  serious topic ("tell me", "what do you think", 150+ characters) is 5 sentences and up to 600
  characters. Tokens are tiered too (90/140/220).
- **`!uyan` doesn't delete the plan, it forces wakefulness.** When the plan was deleted it would be
  rebuilt a minute later and put the bot back to sleep.
- **Chance of chiming in went 0.10 → 0.35, new stage ×0.7.** "It doesn't care if we're not talking
  about the bot."
- **The "sikko" problem.** 25% of the banter turns dumped made-up code problems into the software
  channel; it opens conversation instead.
- **System message split into fixed + variable, fixed block gets `cache_control`.** Token cost:
  personality, temperament, profile, index, agenda, notes are in the fixed block (only changes when
  an agent runs); example messages, fetched content, time and task are in the variable block.
  Anthropic/Gemini cache the fixed block, OpenAI caches the prefix itself.
- **2026-09-02 · cache_control made conditional on the target address (not the model name).**
  Originally it checked whether the model name contained "claude"/"anthropic"/"gemini"; once the
  user said they'd be using GLM on OpenRouter, it turned out that was the wrong question: in a
  request going to OpenRouter, cache_control can be added safely no matter the model — the field is
  part of OpenRouter's own unified schema, it decides on its own side whether it works for a given
  model, and ignores it on models that don't support it. The real risk is Mistral's native API or a
  custom router given via `API_ADRES`: they don't offer this guarantee and may reject the whole
  request over an unknown field. `onbellek_destekler(api_adres)` now only checks for the
  `openrouter.ai` address; the provider-specific assumption is consolidated in one place.
- **2026-09-02 · isteklilik/hedef_sec also moved into the fixed+variable block.** Previously, going
  through `analiz()`, the profile+index were embedded into the user message on every mini call and
  resent at full price (once per 2 min per channel, the most frequently triggered call). Now
  `sor_bolumlu` is called directly: profile+index (isteklilik) or the instruction (hedef_sec) go in
  the fixed block, only the recent messages go in the variable/user message.
- **2026-09-02 · A token ceiling for chat replies even in release (CEVAP_TAVANI=3000).** Previously
  in release, `max_tokens` wasn't sent at all ("let the model talk until it's done"); an ordinary
  reply stays well under this, but runaway cases like repetition/loops could grow the cost
  unbounded. The ceiling isn't low enough to cut off an ordinary reply, it only stops a runaway.
- **2026-09-02 · Token metrics broken down by call type.** `Metrik.kategoriler: HashMap<&str,
  Kullanim>`; every `sor_ham`/stream call carries a category tag (`"sohbet"`, `"isteklilik"`,
  `"profilci"`...). `!durum` now also dumps the categories burning the most tokens.
  `Kullanim.prompt_tokens_details.cached_tokens` is read (`onbellek_token`) when the provider
  reports it — so whether the prompt cache is actually hitting can be seen from the log/`!durum`
  (still unverified live, see AGENTS.md known gaps).
- **2026-09-02 · `durum/taranan.md` made persistent.** `guild_create` fires again on every `ready`;
  `taranan` was in-memory, so on every process restart the 14-day history of every channel in every
  server was re-fetched from the API (the complaint "it re-fetches messages from scratch every time
  it connects"). Now it's written to disk and read on startup; a server is only scanned on first
  join.
- **2026-09-02 · Scope narrowing via GUILD_ID/KANALLAR (.env, optional).** By default the bot
  worked in every server/channel it had access to; if both are empty the behavior stays the same.
  Once set, `message`, `guild_create`, `guild_member_addition`, `varsayilan_kanal` all filter
  (including scanning, so the API isn't hit needlessly).
- **2026-09-02 · The `mesgul` flag made RAII (`MesgulGuard`, merged with PR #2).** There were
  manual `mesgul.remove` calls at 7 different exit points; if an `.await` in between panicked, the
  channel would stay locked "busy" forever (only cleared by a restart). Now it's guaranteed via
  `Drop`; the manual remove calls were removed.
- **2026-09-02 · HTTP client timeout split (P0 closed).** A single `.timeout(60s)` could cut off a
  long stream mid-way (see docs/roadmap.md, Agent 2). `connect_timeout(10s)` + `timeout(180s)`: a
  bad connection is eliminated quickly, and the total duration is wide enough to fit CEVAP_TAVANI
  even on the slowest provider.
- **2026-09-02 · Automatic retry on a model whose reasoning can't be disabled.** Live bug: the GLM
  reasoning variant (`z-ai/glm-5.3-flash`, OpenRouter) returned a 400 "Reasoning is mandatory ...
  cannot be disabled" for the `"reasoning":{"enabled":false}` field sent in "turn off thinking"
  mode — chat could no longer reply at all in that channel. `reasoning_kapat` now returns (`bool`)
  whether it actually added the fields; when `sor_ham`/`sor_ham_akis` recognize this error
  (`reasoning_zorunlu_hatasi`) they strip the fields and retry once more. Note: on the same model,
  small `max_tokens`-budget mini calls (isteklilik 80, hedef_sec/ruh_hali 40, haber_sec 10, etc.)
  can still have hidden reasoning eat the whole budget and produce "empty reply from model" — this
  is a separate problem, not solved by the code; reasoning-mandatory models are fundamentally in
  tension with this architecture.
- **2026-09-02 · Open chat now auto-replies only to the ongoing dialogue, not to everyone in the
  channel.** User complaint: live, the bot was replying to every message (something separate from
  the reply-to fix). Root cause: in the `message` handler, `acik` (is there a chat open in this
  channel) alone meant "no need to evaluate, reply directly" — once a chat was open, EVERYONE's
  message in the channel got a direct reply, regardless of who they were talking to. Now there's
  `devam_eden_diyalog`: if the owner of the last user message in the chat has the same name as the
  sender of this message (really talking to it) it continues automatically; if someone different
  wrote (or it's gone cold) it still goes through the willingness evaluation (same 2-min rate
  limit). A tag always takes priority. Doesn't need a separate command like `!uyan`, it's the
  `message` handler's own logic.
- **2026-09-02 · `hafiza::yaz` made atomic (temp file + rename).** `fs::write` wrote straight to
  the target file; if the process crashed/was killed (or two agents wrote to the same file near
  simultaneously), half-written/corrupt content could remain on disk. Now it writes to a
  `<target>.tmp.<pid>.<counter>` file and swaps it in atomically with `fs::rename`; a reader never
  sees a half-written file.
- **2026-09-02 · Background loops wrapped with `dongu_bekci`.** With `tokio::spawn(dongu())`, if a
  loop panicked that function never ran again until the process restarted (the panic hook only
  logs, it doesn't restart). `dongu_bekci(ad, || dongu(...))` logs every panic/unexpected return,
  waits 5 sec, and respawns the same loop.
- **2026-09-02 · `soy` counts characters, not bytes.** `metin[onek.len()..]` — `onek.len()` was a
  byte length, but `to_lowercase()` changes the byte length for some letters (Turkish uppercase İ →
  "i̇", 2 bytes → 3 bytes); with the comparison in lowercase and the slicing on the original, it
  could land outside a char boundary and panic. `.chars().skip(n)` is always safe.
- **2026-09-02 · Sleep-themed leftovers in `durum/huy.md` cleaned up, a preventive rule added to
  `hoca.md`.** User complaint: "I ran !uyan but it still says it's tired/sleepy." Root cause: the
  hoca agent (the huy.md producer) had mistaken the frequent `!uyan`/sleep banter during testing
  for a permanent TRAIT ("lazy, sleepy... damn I overslept... sick of being woken up") and written
  it down — it has nothing to do with the bot's ACTUAL sleep schedule (code, affected by `!uyan`),
  it was purely a word-collision confusing things. Also the NATURALNESS section (which is supposed
  to name patterns to avoid) was instead inventing "PATTERNS" and imposing fixed lines. Added to
  `hoca.md`: use only the 5 headings, don't write sleep/waking-themed phrasing, NATURALNESS doesn't
  propose new catchphrases; the existing `durum/huy.md` was cleaned up by hand.
- **2026-09-02 · Mood (RUH_HALI, `ruh_hali_belirle`).** The request "mimic human moods during
  discussion"; instead of a heavy new agent, the same lightweight mini-call pattern as
  `isteklilik`/`hedef_sec`. A taxonomy of cognitive/fear/positive/depressive/anger/social-reasoning
  categories is in the prompt; it looks at the conversation's own history and returns
  `{"durum","yogunluk"}`. To limit cost it's called not on every message, but only when a chat
  opens and every 4 turns (`Sohbet.sayac`); intensity <3 counts as neutral and returns None (not
  every conversation is dramatic). The result is kept in `Sohbet.ruh_hali` (not persistent,
  evaporates with the chat), added to the instruction as "YOUR CURRENT MOOD"; the personality
  prompt has a rule "don't announce it, let it seep into the tone" (so it doesn't come out as
  literally saying "I'm confused").
- **Repetition guard.** If a reply matches one of the bot's last 5 messages, it's regenerated once;
  if it's still the same, it stays silent.
- **Internet access on request.** A link → the page; "news/what's up/what happened" → Sözcü RSS;
  "look into it/check/google it" → Firecrawl search (if a key is set, otherwise RSS). Findings are
  added to the task as "WHAT YOU JUST PULLED FROM THE INTERNET."
- **"Never back down" and "don't refuse requests" are in the prompt.** The "what does that have to
  do with anything / yo what's up" reflex is banned; when asked to rank, choose, or predict, it
  uses opinion scores; "I can't" is banned. Identity: Nişantaşı University, a white Tofaş.
- **A hidden thought line.** To cut down on off-topic replies ("what does that have to do with
  anything") on small models, a single "DÜŞÜNCE:" (THOUGHT:) line comes before the chat reply (who,
  what they want, what reply fits), then "CEVAP:" (REPLY:); the code sends only the reply
  (`cevap_ayikla`). +70 tokens. Temperature 0.8.
- **2026-09-02 · Speed simplified again.** The hidden DÜŞÜNCE/CEVAP (THOUGHT/REPLY) round and the
  artificial "typing" wait after the reply was ready were both removed; the two of them were adding
  up to noticeable delay live. The typing indicator stays on while the model runs, reply budgets are
  70/100/140 tokens and temperature 0.7. If a new message arrives during generation, the old reply
  is dropped and regenerated with the current context instead of being sent.
- **Reply reference on every reply again.** The request to tag whoever it's addressing outranks
  conditional behavior; a normal chat reply always attaches to the last user message in the
  snapshot and pings it.
- **2026-09-02 · Reply reference made conditional again (reverses the decision above).** The
  request "putting reply-to on every message is robotic, it should only reply-to when needed, like
  a real human"; `Sohbet.son_etiketlendi` was added (records the tag/name/reply check when a
  message is pushed). In `cevapla`, the base `yanit` is `son_mesaj` only if it's tagged or
  `bekleyenler.len() > 1` (more than one message slipped in between), otherwise `None` → a plain
  message. In a crowd (when `hedef_sec` found one) it still overrides this.
- **Sleep state isn't a conversation line.** The sleep plan decides whether to reply or not in
  code; the instruction "you couldn't sleep, you're in that mood" no longer goes into the prompt
  during an active chat. Live, the bot was pulling the conversation toward itself for no reason,
  saying things like "what do you expect from sleep."
- **Raw server messages aren't few-shot.** The active chat history already goes to the model; 12
  raw sentences picked from other channels were removed from the system prompt because they carried
  over slang and patterns.
- **ICE fandom is in the core personality, with a limit.** Emin's request; an absurd gag, like
  rooting for a football team. Targeting nationality/ethnicity/religion/immigration status,
  threats, and glorifying violence are banned in the prompt; hoca can't remove this item (it's in
  the core `kisilik.md`).
- **2026-09-02 · Chat replies stream.** The reply doesn't arrive all at once: the message opens
  with the first delta, and is edited at an interval of `AKIS_DUZENLEME` (1.2 sec) (there's no real
  stream for bots on Discord, editing is the only way; the interval leaves headroom against the
  edit rate limit). News/welcome/banter stay without streaming; only chat replies use it.
- **Thinking shown uncut, in a spoiler.** If the model returns `reasoning` or `reasoning_content`
  (OpenRouter reasoning models, Qwen, etc.), it's shown in `||...||` spoiler blocks throughout the
  reply and never truncated; thinking longer than 1896 characters spills into a new spoiler
  message. A model that doesn't produce it has no block. Only the reply goes into the record;
  hoca/eleştirmen never see the thinking. Thinking is collapsed to a single flowing line
  (`tek_satir`); no newline is emitted per thought.
- **Thinking-mode command (`!düşünme`).** Three modes: show (thinking alongside the reply in a
  spoiler), hide (thinking is produced but not shown; while thinking a single "Thinking..." message
  is shown, and once the reply starts the same message is edited to stream it), off (requests are
  sent without reasoning via `reasoning.enabled=false` + `enable_thinking=false`, no tokens spent).
  The mode persists in `durum/dusunme.md`; `Durum::yukle` reads it. In show/hide, a placeholder is
  sent before the reply starts so the user knows it's coming.
- **Commands in a separate module (`src/komut.rs`).** Test/admin commands moved out of main.rs;
  follows the convention that `impl Bot` can be spread across the same crate. `!yardım`/`!help`
  lists all commands.
- **Code-side truncation removed (`kisalt` deleted).** The prompt keeps the reply short; code only
  kicks in at Discord's physical limit: a reply over 1900 chars is split into a new message at a
  sentence/whitespace boundary (`bol`), nothing is discarded.
- **2026-09-02 · Reply budget via a macro, based on build profile.** `cevap_butcesi!()`: in
  release, `None` → no max_tokens sent in the request, the model talks until it's done (reasoning
  models already finish their own thinking anyway); in debug, `Some(2000)` → cost protection during
  a dev session. Fixed-budget calls (agents, news, welcome...) keep passing `Some(n)`.
- **`API_ADRES` from `.env`.** Overrides the provider address; routes to your own
  (OpenAI-compatible) router. Key/model selection stays in the `SAGLAYICI` logic.
- **2026-09-02 · The 12-message limit and farewell removed.** User report: the limit produced
  strange behavior. Now there's no message-count limit, no farewell/last-message instructions, and
  no channel ban; a chat closes silently 30 min after the last message (`SOHBET_ZAMAN_ASIMI`), and
  its transcript still goes to günlükçü and eleştirmen. The `Durum.son_aktivite` map is refreshed
  (user message, chat opening, bot reply).
- **2026-09-02 · Log noise cut + colored output.** User report: the console was flooded with
  serenity's internal tracing events (recv, do_heartbeat, ratelimit dumps). With no tracing
  subscriber, events were falling through to the `log` facade; the sink now filters by target: only
  `discord_bot*` records pass through `LOG_SEVIYE`, foreign crates only get warn/error. ANSI color
  in the terminal (ERROR red, WARN yellow, INFO green, DEBUG dim); color auto-disables when writing
  to a file, `LOG_RENK=on|off` forces it. Stage info was added to "ai error" logs (like `ai
  [uret_akis] [kanal]: ...`).
- **2026-09-02 · Sleep mode: listen, accumulate, evaluate on waking.** User report: the bot was
  going deaf while asleep. Now, while asleep, messages still go into raw memory, the memory cycle
  runs a nightly observation every 2 hours (processes it into the mind), and the news cycle picks
  and stocks news (doesn't discard it). On waking: if there's a tag, a guaranteed response (on
  failure the list is put back, nothing is lost); otherwise the `uyanis.md` agent scores how much
  what was written overnight interests the bot, and if ≥5 it comes back with a morning greeting.
  Stocked news goes out on the first awake cycle. The rule "topics about Nişantaşı University take
  priority" was added to news selection (identity is in kisilik.md).
- **2026-09-02 · Target selection added, start-over removed.** User report: when different people
  wrote one after another, the bot forgot the earlier messages and gave a jumbled reply. Fix: (1)
  `Sohbet.son_gelenler` keeps who's written (name+message id) since the bot last spoke; if there
  are 2+ different people, a `hedef-sec.md` mini call chooses who to address, the reply attaches to
  that message, and a note "address them" goes into the instruction; the list clears after
  replying. (2) the `AkisSonuc::Eski` start-over mechanism was removed: even if a new message
  arrives during generation, the stream completes, and the new message is handled in the next turn.
- **2026-09-02 · Reply willingness evaluated by the model.** User report: the bot felt obligated to
  reply to every message. The fixed dice roll (`SANS × evre`) was removed; a tag/reply/name still
  always gets a reply, other messages go through a mini model call (`isteklilik.md`, ~80 tokens,
  via the `analiz` path) that scores 0-10 using the last 12 messages + profile + index. Threshold
  is `ISTEK_ESIGI` (6), stage boldness ±1, +2 while traveling. At most one call per
  `DEGERLENDIRME_ARALIGI` (2 min) per channel; if the call fails, the old fallback dice roll
  (`SANS`) takes over.
- **2026-09-02 · Memory made id-based + timestamps to the second + a memory queue cycle.** Person
  files are `kisiler/<id>.md`; `id`, `kullanici_adi`, `eski_adlar` fields were added (a name change
  doesn't split memory). Name→id translation goes through `Durum.ad_id`; a record that can't be
  resolved is skipped for that turn and logged. Clean start: old slug files aren't read. All
  records now use `tarih_saat()` with second-level precision. Agents are no longer inline: a closed
  chat's transcript and the 6-hour observation drop into `bellek_kuyruk`, processed by
  `bellek_dongusu` (10 min, not blocked by sleep checks); if the queue exceeds 50 the oldest is
  dropped.
- **2026-09-02 · Modals + /zihin.** Slash commands (/durum /yardim /zihin) open a modal, `!`
  message commands remain in parallel as plain text (both at once, user's decision). The mind modal
  is open to everyone. Discord's constraints shape the design: a modal has at most 5 components,
  each TextInput value ≤4000 characters → `sigdir` cuts overflow at a line/whitespace boundary and
  leaves a note; title/label ≤45. The mind's 5 slots: bot summary / people split in two halves (by
  mtime order) / topics / events+agenda. `!zihin` gives an index summary + a pointer to `/zihin`
  instead of dumping 5×4000 into the channel. Modal submissions aren't stored, a short ephemeral
  confirmation is returned. Guild commands are registered idempotently on every ready (they show up
  instantly, not delayed like global ones).
- **2026-09-02 · HTTP timeout + retry + mechanical hardening.** A global 60-sec client timeout was
  cutting off long thinking streams: removed, replaced with `connect_timeout` (15 sec) +
  `read_timeout` (120 sec, resets on every read → also covers the first token); no total-duration
  limit. On transient errors (network, 429, 500/502/503/504), back off 2 and 4 sec with 2 retries
  (`sor_ham` and `sor_ham_akis`, streaming only before it opens). `reasoning_kapat` is now
  provider-specific: OpenRouter gets `reasoning.enabled`, no parameter goes to Mistral, others get
  `enable_thinking:false` (sending both at once broke some providers). `MesgulGuard` (RAII): the
  channel's busy flag is released on every exit, panics included. `soy` is char-safe (a byte slice
  could panic on Turkish names) + `kucult` drops the İ→i̇ combining dot. Typing was taken out of
  the edit loop (rate limit); done once before the model call.
- **2026-09-02 · Background agents turn off reasoning independent of the mode.** Live log: with
  the mode on "hide," `reasoning_kapat` only kicked in when the mode was "Off" — `sor_ham` (the
  non-stream path used by profilci/hoca/günlükçü/gezgin/isteklilik/ruh_hali) never reads/shows the
  `reasoning_content` field anyway — small `max_tokens` budgets (20-1200) went entirely into
  thinking and returned `content: null`, leaving kisiler/konular/olaylar empty with an "empty reply
  from model" error. `reasoning_kapat` now takes an `herhalukarda: bool` (regardless): `sor_ham`
  always passes `true` and disables it regardless of mode, `sor_ham_akis` (stream, chat) passes
  `false` and keeps the old behavior (disable only when the mode is Off).
- **2026-09-02 · A "silent" mode added to thinking mode.** User request: even in "hide" mode, the
  live word counter while thinking ("thought for X words") was annoying; a mode was wanted that
  gives the reply directly with no trace left — but with the reasoning model still thinking in the
  background. The fourth mode, `Sessiz` (Silent): in `reasoning_kapat`, this mode doesn't count as
  Off, so reasoning is requested normally (not disabled on the stream path), only
  `gonder_akis`/`akis_gorunum` never collect/show the thinking at all — no placeholder, counter,
  spoiler, or "Show Thinking Process" button; the on-screen appearance is exactly the same as Off
  mode (nothing is sent until the reply starts); the difference is that in Off, reasoning never
  enters the request at all, while in Silent it actually runs, just hidden.
- **2026-09-02 · Small budgets get raised to a floor on a reasoning-mandatory model.** Live log:
  even after the previous turn's fix (`z-ai/glm-5.3-flash`, OpenRouter), this model/endpoint
  doesn't allow disabling reasoning at all ("Reasoning is mandatory ... cannot be disabled"). The
  code caught this, stripped the fields, and retried with reasoning left on, but never touched the
  budget: on 20-token-budget mini-calls like `gezgin_sec`, reasoning again ate the entire budget
  and left `content: null` — this time it returned 200, so it never even entered the earlier
  error-catching path, and came back straight with "empty reply from model." Two changes: (1)
  `butce_tabanini_uygula(govde, taban)` — if `max_tokens` is set and below the floor
  (`REASONING_ZORUNLU_TABAN`=500), raises it; otherwise (a budget-less call) leaves it alone;
  called during the mandatory-reasoning retry. (2) In `sor_ham`, getting a 200 with empty/null
  content is no longer an immediate error: the budget is raised to the floor (if possible) and it's
  retried once more, giving up once `AI_YENIDEN_DENEME` is exhausted. The same budget floor is
  applied in `sor_ham_akis` on the mandatory-reasoning branch too (the stream side has no
  empty-content retry, `gonder_akis` already handles a short/empty reply separately).
- **2026-09-02 · Encouragement of harassment/insults removed from the personality, aligned with
  server rules.** Emin's request: the "WHEN INSULTED" section was pointing the bot at a weakness
  from the person's file and telling it to answer profanity/put-downs with profanity/put-downs —
  this directly conflicted with the server's harassment/insult [Level 2] and hostility [Level 2]
  rules. Sharp-tongued/never-backing-down stays, but targeting and weakness/trauma/family-abuse
  material were removed. Also abbreviated swear words ("aq", "amk", "mk") were banned — if it's
  going to swear, it writes the word out in full, no hiding behind an abbreviation (Emin's
  additional request). A new `SINIRLAR` (LIMITS) section condenses the server's shared rule set
  (harassment/hate speech, NSFW/illegal, personal data, political/religious propaganda, deliberate
  misinformation, spam, outbursts of anger) into a short bullet list; this is in the core
  `kisilik.md`, and the temperament hoca writes can't override it (see the ICE fandom decision,
  same principle).
- **2026-09-02 · Identity: Nişantaşı University → ITU physics, the Tofaş dropped.** Emin's
  request, "a better identity." The school/department changed in `kisilik.md`; the "prioritize news
  related to the university" rule in `haber-sec.md` was also updated to the same school so the two
  wouldn't stay inconsistent (one mentioning ITU while the other still looked for Nişantaşı news).
  The white Tofaş detail was removed.
- **2026-09-02 · Memory writes + loop watchdog + scan ordering.** `hafiza::yaz` is atomic (temp
  file + rename) and serialized with `YAZMA_KILIDI`; `ekle` is now a real append (it used to
  rewrite the whole file via read+write → now OpenOptions append). If günlükçü's JSON can't be
  parsed, the raw output is rescued into `arsiv/gunlukcu-<kaynak>.md` (the model's work isn't
  thrown away). Loops start with `dongu_bekle`: on panic, log it + restart 5 sec later (the panic
  hook already writes a backtrace; the watchdog prevents a silent death). Graceful shutdown: a
  `KAPANIYOR` (AtomicBool) signal, loops return at the top of their tick, the watchdog doesn't
  restart them. Expired news chats are cleaned up on the minute tick (comment window elapsed + no
  activity). The startup scan is prepended ahead of memory: live messages arriving while the scan
  runs stay queued behind it, preserving chronology and the live ones.
- **2026-09-02 · PR merges + conflict resolutions.** Remote PRs (token optimization, multi-provider
  generality, discussion behavior, prod-readiness; then silent mode, reasoning safety, identity
  alignment) were merged into the local branch. The watchdog stayed as one function: the local
  `dongu_bekle` skeleton (`KAPANIYOR`-aware — doesn't restart while shutting down) + a 5-sec sleep
  on both restart branches (hot-spin protection). `hafiza::yaz` kept the local body
  (`YAZMA_KILIDI` + a fixed `.tmp`; a pid+counter unique name would have allocated with format! on
  every write and accumulated orphan files); the real append `ekle` was kept.
- **2026-09-02 · CEVAP_TAVANI 3000 → 4096.** On models that produce reasoning, thinking tokens also
  count against the `max_tokens` budget; 3000 could clip a long thought + reply.
- **2026-09-02 · Discussion-behavior fix: willingness is also applied in open chat.** The PR's
  `devam_eden_diyalog` logic was correct but only half-finished: in phase 3, `cevap_ver = acik`
  ignored the willingness result (in an open chat, if someone else wrote, a reply still went out
  even if the score fell below the threshold; the call was just burning tokens for nothing). Now
  `cevap_ver = acik && katil`; the message enters history but no reply is sent. Name comparison
  switched from `eq_ignore_ascii_case` to `kucult` (for Turkish İ/i̇). The redundant `sayac == 0 ||`
  in the `ruh_hali` condition was dropped (0 % 4 == 0 already).
- **2026-09-02 · Hot-path allocation cleanup.** `soy` now returns a `&str -> &str` slice: on every
  stream edit the whole text is no longer cloned and lowercased (prefix comparison only touches the
  first characters). `bol`/`kesim_noktasi` work with byte offsets, no intermediate
  take/skip/collect allocation per turn. `temizle` does an in-place `truncate` at the limit.
  `kanal_not` / `son_mesajlar` / `dokum` concatenate directly into a String without an intermediate
  `Vec` collect. The `getir` budget loop uses a running counter (each section was scanning from the
  start, O(n²)); topic files are carried in the score tuple (the top two aren't read a second
  time). `dizin_yenile` reads the topic file once, with a lightweight title resolver for people
  (`kisi_baslik`, no Vecs of knowledge/events are built). `konu_ekle` does its check+title+line in
  a single lock region (closes a race where concurrent calls could duplicate the title/drop a
  line). `sohbet_sistemi` builds no temporary String for `contains`.
- **2026-09-02 · The reply is now a line-based protocol (a line = a separate message).** Emin's
  request: "it should be able to react like a normal human while chatting; let's remove the limits
  in its personality." The model no longer writes plain text but a line protocol, decoded by
  `cevap_parcala`. The rationale isn't just a taste preference: Discord's own API team, while
  rejecting a "ChatGPT-style streaming message" request, says people on the platform send "multiple
  shorter messages," not long essays
  (<https://github.com/discord/discord-api-docs/discussions/6310#discussioncomment-6519016>);
  reference implementations also split at line boundaries and send in sequence
  (<https://honcho.dev/docs/v2/guides/discord#message-sending>,
  <https://github.com/0xranx/golembot/blob/ce48b37c8e1eb267548d352d56e34836714e0c01/docs/channels/discord.md>).
  The ceiling `PATLAMA_SINIRI=4` is not a target: in a real IM corpus, a person's back-to-back
  message run averages **1.7 messages**, **42%** of runs are multi-message, and average message
  length is 5.4 words (Baron 2010, 23 conversations / 2185 transmission units,
  <https://scholarworks.iu.edu/journals/index.php/li/article/view/37586/40137>) — meaning "split
  every reply into three" would be wrong; most replies should be a single line, and the prompt says
  so too.
- **2026-09-02 · Review fixes: "is there anything to send" became the single test.** Once the
  protocol went line-based, several spots still carried the old (single-message) assumption; all
  were pulled onto the same rule: (1) `gonder_cevap` drops the reaction if there's no reaction
  target, and returns `None` if there's really nothing to send — otherwise a chat would open and
  the 30-min timeout counter would start while nothing appeared in the channel. (2) In
  `gonder_akis`, a `-` + `tepki: 💀` (reaction) combination doesn't count as silence, the emoji
  still fires (the prompt explicitly suggests using both together). (3) The welcome ping is no
  longer glued onto the text upfront, it's added to the first line at send time: `<@id> -` was
  hiding the silence marker, `<@id> tepki: 💀` was hiding the reaction line. (4) `sohbet_baslat`'s
  opening dedup is line-based (the opening drops into history line by line, exact equality never
  matched). (5) In `cevapla`'s fallback branch, repeat filtering is line-based. (6) Command
  detection looks at the raw text: in a message with an image, the text was `[resim] !durum`, so
  commands were being swallowed. (7) `dokum` puts a name prefix on EVERY line of a bot reply,
  otherwise eleştirmen would count sub-lines as belonging to the group.
- **2026-09-02 · Numbering prefixes are only stripped from a real list.** `slop_temizle` was
  unconditionally removing a "1. "/"2) " prefix; in Turkish, a leading ordinal at the start of a
  line is common ("3. sınıftayım" — "I'm in 3rd grade," "2. el araba" — "used [2nd-hand] car"), and
  meaning was silently dropping out of the message. Now `cevap_parcala` looks at the whole reply:
  if there are two or more numbered lines, it counts it as a list and strips the prefixes, and
  leaves a single line untouched. For the same reason, `**`/`__` stripping doesn't reach INSIDE
  backticks (`` `__init__` `` must stay intact) — the "don't touch backticks" rule already read
  this way.
- **2026-09-02 · Splitting isn't neutral, it's an emotional signal: neutral/informational remarks
  don't get split.** The multi-message version of the same statement reads as more intense emotion
  than the single-message version (M=5.89 vs 5.65, p<0.05, d=0.36-0.50); putting the same words on
  separate lines WITHIN a single message does NOT produce this effect (p>0.10) — the effect comes
  from being separate messages (<https://pmc.ncbi.nlm.nih.gov/articles/PMC11867088/>). There's
  counter-evidence too: sending an information-heavy message in a back-to-back burst drops the
  sender's likability by 19.6% (n=805, 25.6% for under-40s)
  (<https://www.lyngolab.com/texting-back-to-back.html>). The two together resolve into a single
  rule: information/explanation stays on one line, enthusiasm/annoyance/banter can be split. This
  rule wasn't enforced in code (code only sets a ceiling), it was written into `kisilik.md` — the
  model knows the content type, the code doesn't.
- **2026-09-02 · Silence marker `-` (AkisSonuc::Sus).** If the model writes a single line `-`,
  nothing is sent; it's not written to history, the counter, or `son_aktivite` either, and the
  fallback `uret` isn't called (otherwise the "chose to be silent" decision would get punched
  through by the second call). Rationale: a human doesn't feel obligated to reply to every message
  in an open chat; also "no reply" isn't an AI tell, "trying to keep an answer ready for everything"
  is (K1 Table 2 meta-category, <https://arxiv.org/html/2405.08007v1>). Willingness pre-filtering
  decided whether to enter the channel at all; this now allows staying silent even after entering.
- **2026-09-02 · Emoji reactions are a reply type (`tepki: 💀`).** It doesn't go out as line text,
  it lands as a `create_reaction` on the replied-to message; a reaction alone also counts as a
  valid reply. The target is carried via `AkisBaglam.tepki_hedefi` — a separate field is needed
  because `yanit` is conditional (only tag/crowd). Honesty note: a first-hand applied source for
  this **couldn't be found** (ra-muhendislik.md §10); the decision rests on Emin's request and on a
  reaction being cheap/reversible. Custom emoji (`:kekw:`) aren't supported, only Unicode; a
  failure only lands in a warn log, it doesn't stop the flow. Discord's emoji routes are subject to
  a separate and vague quota (<https://discord.com/developers/docs/topics/rate-limits>), so at most
  one reaction is fired per turn (the first `tepki:` wins). ra-muhendislik.md §10 suggests choosing
  from the server's actual emoji list (a whitelist) instead of letting the model pick freely;
  **deliberately not adopted**: pulling the server's emoji list brings a separate state and refresh
  job via `guild_create`, and a reaction is already a cheap, reversible side effect. Instead of a
  whitelist, extraction was narrowed with `emoji_basi`/`emoji_devami` (only known emoji blocks;
  marks like `—`, `…`, `→` don't count as a reaction — if they did, the request would come back
  400). The risk of a nonsensical emoji will be watched live. Unused sources: shapes
  (docs.shapes.inc) and Frontiers in 2021 — present in the surveyed material, but not decisive in
  any of these decisions.
- **2026-09-02 · The "line shorter than 3 characters" filter removed, slop cleanup put in its
  place.** The old rule was eating natural reactions like "yeah," "nah," "lol"; in real IM, **21.8%
  of messages are a single word** (Baron 2010). `slop_temizle` took the filter's place:
  bullet/number prefixes and `**`/`__` marks are stripped (backticks are left alone). Rationale:
  43% of the reasons behind an "is this AI" verdict are linguistic style, 10% are
  information/reasoning; output formatting (markdown) is listed directly as an AI tell (K1 Table 2,
  <https://arxiv.org/html/2405.08007v1>; in the three-party test too, style is the most common
  reason class, <https://www.pnas.org/doi/10.1073/pnas.2524472123>).
- **2026-09-02 · A question ceiling: code measures, the model applies it.** If two of the bot's
  last 4 lines ended in `?`, the instruction gets a "don't ask a question this time"; there's no
  hard cut/truncation, the model can still ask if it wants to. Rationale: in a three-party Turing
  test, one of the most accurate decision reasons was "how it handled questions" — "it kept turning
  the question back around" (<https://www.pnas.org/doi/10.1073/pnas.2524472123>); answering a
  question with a question is an LLM reflex and gives it away directly. An instruction was chosen
  over a hard cut because a counter-question genuinely keeps the conversation going; a ceiling was
  wanted, not a ban.
- **2026-09-02 · Image attachments go to the model (only the most recent one).** `Mesaj.resim` +
  `mesaj_json` build a multi-part `content` (the same shape as `resimci` in ajanlar.rs); a message
  that's purely an image is processed too, with the text marked as `[resim] …` / `[resim attı]`
  ("[image]" / "[sent an image]"). When a new user line is added, the `resim` field of earlier
  entries is set to `None`: Discord CDN links are short-lived and resending an old image every turn
  burns tokens for nothing. The prompt forbids describing it ("I see X in the image" — it never
  says this) — because describing is an assistant reflex, a human comments on it or reacts to it
  instead.
- **2026-09-02 · Inter-line delay on non-stream paths, no delay on the stream path.**
  `gonder_satirlar` puts `300 ms + 15 ms × character count` (capped at 1500 ms) + typing between
  lines; without a delay between parts, three messages land in the same second, which looks *less*
  human than a human (ra-muhendislik.md §1 pitfall list). Reference Turing-test implementations
  also tie delay to character count (<https://arxiv.org/html/2405.08007v1>,
  <https://www.pnas.org/doi/10.1073/pnas.2524472123>); the ms coefficients are **not given** in the
  publications, the three constants here weren't measured, just picked roughly. NO extra delay on
  the stream path: the stream's own pace already gives a human typing speed, and Emin doesn't want
  delay there (see the "Speed" decision).
- **2026-09-02 · CLI chat mode (`cargo run -- sohbet`).** Trying out personality changes on the
  live server is costly and irreversible; the rig runs it through the same `uret` + `cevap_parcala`
  path without ever connecting to Discord (no token needed). `Bot::kur()` was split out of `main`
  for this reason: both paths should see the same setup. It doesn't write to disk (`gecmise_ekle`
  in memory instead of `kanal_not`), but it does read `durum/` files — the rig is meaningless
  without a realistic personality.

- **2026-09-02 · Command UI: embed card + detail modals (like a web page).** User complaint: the
  old 5-section mind modal showed content that was empty/poor, as if everything had been dumped
  into a single text box. New design: `/durum` `/yardim` `/zihin` return only an **embed card**
  visible to the caller (sectioned, colored, with a footer); the `/zihin` card has a person select
  menu (≤25, value=id) + Topics/Events/Bot-summary buttons. A menu/button opens a **detail modal**:
  for a person, Identity/Impression/Tags/What-it-knows/Recent-events as separate labeled fields;
  events get a field per month (last 3 months — closes the old "this month only" gap); topics show
  recently changed + others; the bot summary shows Status/Tokens/Self/Agenda. Empty sections are
  skipped in the modal. `!zihin` now sends the same card to the channel (the raw INDEX dump is
  gone); since a modal can't be opened from a channel message, detail is pointed to `/zihin`. The
  old `modal_zihin`/`bolumler`/5-slot design was removed.
- **2026-09-02 · Version shown in !durum and posted to the channel on restart.** Emin's request:
  make it clear which code is running. Version = Cargo.toml + the short commit `build.rs` grabs
  from git at build time (with a `+` suffix if the working tree is dirty) + the build date; no
  external library, `?` if git/date are unavailable. The announcement fires on `guild_create`, not
  `ready` (the cache is populated there, `varsayilan_kanal` can be found), once per process; it's
  not written to memory so the bot doesn't mistake the "version" chatter for its own words.

- **2026-09-02 · `!zihin` switched from a mind card to a panel screenshot.** Emin's request: "when
  you type !zihin it should post a screenshot that looks like a modern web UI." The embed card was
  cramped inside Discord's boxes; a panel image is both more readable and understandable at a
  glance on a phone. Details that require interaction (person menu, section buttons, modals) were
  left in `/zihin` — a channel message can't carry components anyway.
- **2026-09-02 · The image is produced with resvg, not a headless browser.** The alternative was
  HTML + Chrome/Puppeteer: a heavy install, dependent on the machine it runs on, and it would
  attach a 200 MB browser to the bot process. `resvg` is pure Rust; we build the SVG ourselves, a
  PNG comes out, no external process. Cost: SVG doesn't wrap text — line breaking, truncation, and
  layout are computed by hand in `zihin_gorsel.rs`. `default-features` is off (only `text` +
  `system-fonts`); jpeg/gif decoders are unnecessary.
- **2026-09-02 · The font is embedded (Inter, SIL OFL).** Relying on the system font would make
  output vary machine to machine, and a server might have no font at all. Inter
  Regular/SemiBold/Italic live under `fonts/`, baked into the binary with `include_bytes!` (~1.2
  MB); the license sits in `fonts/LICENSE` (an OFL requirement). If embedding fails it falls back
  to `load_system_fonts` and prints a `warn`.
- **2026-09-02 · Emoji aren't drawn, they're dropped.** Inter has no emoji glyphs; if not dropped,
  tofu boxes show up in the panel. `temizle` strips symbols above U+2190 and control characters.
- **2026-09-02 · Agent calls made resilient on a reasoning-mandatory model.** Emin, from
  production: "the mind system isn't working, I think it's from reasoning" (glm-5.3-flash,
  kisiler/konular/olaylar at 0). The gap found in the code: the 400 "mandatory" retry only raised
  the budget to 500, it never touched the 1200-token günlükçü call; when thinking ate the whole
  budget, `content` stayed empty, the JSON couldn't be parsed, and nothing got written to the mind.
  Three layers: (1) on retry, budget max(2×, 1500) and, on OpenRouter, `reasoning.effort=low`; (2)
  when content is empty, for categories expecting JSON, JSON content found inside the thinking
  field now counts — never for prose calls (so hoca doesn't mistake a thinking dump for huy); (3)
  error messages and info logs make the chain visible, `!zihin test` tries it without waiting 40
  min. Not verified with a live GLM.
- **2026-09-02 · Debug mode.** Emin: "it's scoring relevance so it doesn't jump on every message;
  it should have a debug for that." `!debug` drops the reasoning behind decisions (willingness
  score/threshold/reason, target, mood, question ceiling, silence/reaction/line count, closing) as
  a single line into the channel. It doesn't enter memory (so the bot doesn't mistake it for its
  own words), and being a bot message it doesn't go through the handler; when off, not even the
  format! is built. DEBUG_KANALI requires a separate channel; otherwise it's the same channel — so
  it works with no setup.
- **2026-09-02 · A button-based settings panel.** Emin: "let's manage settings by pressing
  buttons." Commands still exist; the panel calls the same paths (single source of truth:
  `DusunmeKip`, `debug_ayarla`, `uyandir/uyut`), and after a button press it's refreshed in place
  with `UpdateMessage`. Model switching isn't in the panel (favorite-only permission, list
  verification stays in the command).
- **2026-09-02 · Mind image: review fixes.** Letter-width buckets were pulled up to the ceiling of
  Inter's hmtx measurements (uppercase names were overlapping the @username); the PNG is no longer
  written to disk and read back, the bytes go out as an attachment straight from memory (two
  channels calling `!zihin` at once were racing on the same file); the mood chip now looks at the
  most recently active chat instead of HashMap order. Real glyph measurement (skrifa) wasn't done,
  it's on the pending list.

- **2026-09-02 · The panel image was abandoned, `zihin_gorsel.rs` deleted entirely.** Emin: "it
  looks bad — keep the embed clean instead, and put a button for parts that don't fit so a modal
  opens for the user" — the panel PNG wanted the exact opposite of the two decisions above (image +
  resvg). `/zihin` already carried the embed+button+select+modal structure
  (`modal::zihin_embedleri/zihin_bilesenleri`); rather than rebuilding that, a single path was
  kept. SVG drawing, the embedded Inter fonts (`fonts/`), the `resvg` dependency, the `cargo run --
  zihin` CLI, and 6 tests were removed entirely — there's no longer a separate visual layer for the
  panel.
- **2026-09-02 · Commands are now only slash, in one registration table, instead of `!`/text.**
  Emin: "put together a command manager and move all commands under it, and have every command
  return embed output instead of plain text," then: "disable exclamation-mark commands entirely,
  the bot should work with only slash commands." `Bot::komut` (one big `match`, each arm writing
  its own `soyle()` plain-text) and the `!`/`/` text-capture block inside `Handler::message` were
  removed. In their place, the `komut::KomutTanimi` table: name, description, Discord options
  (`CreateCommandOption`), and the runner (`fn(&Bot,&Context,&CommandInteraction) ->
  Pin<Box<dyn Future<...> + Send>>`, registered with the `komut_gir!` macro) all in one place.
  Registration (`modal::komutlari_kayit`) and dispatch (`interaction_create`) both read from the
  same table — the command name isn't kept by hand in two places. Every command that needs a text
  reply now returns an embed via `modal::bilgi_embed` (no plain `content`). Discord interactions
  want an initial response within 3 sec; commands that make a network/model call
  (`haber/sorun/gez/saka/hack/ajanlar/uyan/uyu/zihin test/model id değişimi`) first give instant
  acknowledgment via `ertele` (`CreateInteractionResponse::Defer`), then once the work is done
  write a short result embed with `sonucu_bildir` (`edit_response`); the actual content
  (news/joke/etc.) was already going to the channel via its own `Bot::gonder` call, and that didn't
  change. None of the old text-commands' argument/permission logic changed (e.g. `/model id` is
  still FAVORITE-only, `/dusunme kip` still accepts the same `DusunmeKip::arg_ile` strings — the
  slash option values were deliberately kept identical to these).
- **2026-09-02 · `main.rs` (4695 lines) split into `src/bot/*.rs`, with `include!` instead of
  `mod`.** Emin: "you can move the functions in main.rs into a separate folder and split related
  ones file by file." Using real `mod`s was tried and abandoned: Rust visibility works by the
  module **tree**, "same file" or "same crate" isn't enough — moving structs like `Durum`/`Bot`
  into a sibling module would have required making almost every field/method `pub(crate)`, plus
  updating the `use super::*` imports in `ajanlar.rs, gundem.rs, komut.rs, modal.rs, sohbet_cli.rs,
  uyku.rs` — without a compile check along the way (user's request: move fast without intermediate
  builds), making a change this wide and error-prone safely wasn't possible. `include!("bot/x.rs")`
  was chosen instead: it pastes the text in place at compile time, there IS NO module boundary —
  the result behaves exactly like the original single file, visibility/`use super::*`/
  `cevap_butcesi!` macro scope included; the other 6 files weren't touched at all. Split by topic
  into 7 files: `tipler` (constants+structs), `metin` (pure text/protocol helpers), `saglayici`
  (the AI call+send layer — the "send" layer that was supposed to be separate in the initial plan
  was left in the same file because it was interleaved inside the same `impl Bot` block), `sohbet`
  (the `cevapla` loop), `dongu` (growth+memory scanning+background loops+actions — these were also
  intertwined in the text, separating them would have needed mid-block surgery), `handler` (the
  Discord event handler), `kurulum` (`Bot::kur` + startup). Tests were left in main.rs at first,
  then moved to `src/bot/testler_*.rs` in the "200 lines" pass below. Single verification: once at
  the end, `cargo check` + `cargo test` + `cargo clippy` + `cargo fmt` — all clean, 75 tests.
- **2026-09-02 · The 200-line rule: the 7 files above + `komut.rs` split into finer slices.**
  Emin: "even though we split main, 700 lines means we need to split it further," and "it'd be
  better if no file were longer than 200 lines." The same `include!` pattern was repeated, with two
  extra technical constraints: (1) when splitting a single large `impl Bot` block (the AI-call
  layer, `saglayici.rs`) into pieces, each piece has to be a SELF-BALANCED set of items —
  `include!` can't take "half" text inside an impl block (only an opening or only a closing brace)
  (`non-impl item macro in impl item position` error); the fix was splitting into six separate
  `impl Bot { ... }` blocks and `include!`-ing each at the top level (Rust allows an inherent impl
  to be reopened repeatedly). (2) `impl EventHandler for Handler`, on the other hand, has to be a
  SINGLE block (a second impl for the same trait+type is an E0119 compile error) — so
  `handler_event.rs` (struct+ready+guild_create+guild_member_addition+message+interaction_create)
  stayed at 423 lines, unsplit. Similarly, since cutting a single function through the middle would
  hurt readability, two files slightly exceeded 200: `sohbet_cevapla.rs` (261, the single `cevapla`
  method) and `testler_3.rs` (204, test grouping). Around 50 new files total (`src/bot/*.rs`,
  `src/komut/*.rs`); verification again just once at the end (`cargo check` + `test` + `clippy` +
  `fmt`), 75 tests green.
- **2026-09-02 · `RESIM_ANALIZI` (.env, fixed at startup).** Emin: "photo scanning should be
  toggleable via .env, and never changeable by a command afterward." A `Bot.resim_analizi: bool`
  field was added (to `Bot`, not `Durum` — deliberate: `Durum` carries fields that commands can
  change, `Bot` carries settings fixed for the process's lifetime, like
  `guild_id`/`izinli_kanallar`/`debug_kanali`). It's only read in `Bot::kur()`
  (`kapali/off/hayir/0` → false, otherwise defaults to true); no slash command/button writes to
  this field, the only way to change it is restarting the process. In the `message` handler,
  attachment scanning never runs while it's off (`resim` is always `None`), the message is
  processed as if it had no image attachment at all — the bot's own joke images
  (`saka_yap`/`resimci`) aren't affected by this, that's a separate feature.
- **2026-09-02 · General token/performance sweep: no new change was needed.** Emin: "optimize
  token usage." `sistem_metni` was already split in two — fixed (personality+temperament+profile+
  index+agenda+self+corrections, with cache_control) / variable (fetched content+time+task) — and
  the mini calls (isteklilik/hedef_sec/ruh_hali) already share the same fixed block (see the "Token
  optimization" entry above). Background agents (`analiz()` — profilci/gunlukcu/hoca/elestirmen/
  gezgin_sec) already run on the small, personality-less `ANALIST` system and never carry the heavy
  `kisilik.md`; budgets (20-1200 tokens) are already kept small relative to task size. Making
  further "optimizations" blindly, without live telemetry (how much each category burns under real
  traffic), would likely be either ineffective or risk breaking the current fine-tuning — if a
  concrete bottleneck is reported (e.g. the measured cost of a specific command/flow), it'll be
  addressed then.
- **2026-09-03 · The codebase (src/**/*.rs, README.md) translated from Turkish to English.** User
  request. Scope: identifiers (function/struct/enum/const/file/directory names), code comments,
  `.env` variable names (`SAGLAYICI→PROVIDER`, `KANALLAR→CHANNELS`, `HABER_KANALI→NEWS_CHANNEL`,
  `DEBUG_KANALI→DEBUG_CHANNEL`, `RESIM_ANALIZI→IMAGE_ANALYSIS`, `API_ADRES→API_URL`,
  `LOG_SEVIYE→LOG_LEVEL`, `LOG_RENK→LOG_COLOR`), the CLI flag (`cargo run -- sohbet` → `cargo run
  -- chat`). Deliberately left in Turkish: `prompts/*.md` (directory+file name+content — the bot's
  personality), `durum/` file formats (field names, file names — the ones that need to match the
  model's JSON output or existing on-disk data, e.g. the `isim`/`puan`/`not` fields of the
  `Person`/`Record` types), and everything surfaced to Discord (slash command
  names/descriptions, embed text, button/menu labels, model output, except debug trace text — that
  counts as developer diagnostics and was translated to English). Rationale: international
  readability of the codebase; the bot's Turkish personality/behavior didn't change at all, only
  the language of the people reading the source did. Verification: `cargo build` + `cargo test`
  (76 green) + `cargo clippy` (0 warnings) + `cargo fmt`; not tried live on Discord in that session.
  This is a historical verification claim, not a current test report.
- **2026-09-03 · Moved from `durum/` markdown files to redb (`durum/hafiza.redb`).** User request:
  an embedded database instead of markdown-per-file, plus a migrator to carry over the old data.
  Three options were offered, the user chose redb: rusqlite (SQLite) would have brought in a C
  dependency — the project has been deliberately C-free so far (`reqwest` is set up with
  `rustls-tls` instead of the default `native-tls` for exactly this reason); plain structured JSON
  files, on the other hand, gained no real ACID/transaction guarantees. redb: pure Rust, a single
  file, transactional. Design decision: the data model was **not reshaped**, only the container
  changed — every redb value is the exact same text a file held before the migration, and the key
  is that file's old relative path (`"kisiler/1.md"` etc.). This let the ~15 call sites of
  `Person::parse`/`text`, `retrieve`, `keywords`, `trim`, `slug`, and `memory::read`/`write`
  (signatures unchanged) go untouched — what actually shrank the risk wasn't a big redesign, it was
  this minimal-diff approach. `durum/arsiv/` was deliberately not moved into redb: it's for humans
  only (the bot never reads it again) and grows without bound — keeping it in a transactional store
  would be both unnecessary and would defeat its "for humans" purpose. `WRITE_LOCK` (a global Mutex
  + temp-file+rename) was removed: redb write transactions are already serialized internally, which
  is a stronger guarantee (cross-table atomicity is possible too, v1 didn't use it, to keep scope
  small). `files()`'s "most recently changed first" ordering now comes from a timestamp kept in a
  separate `MODIFIED` table instead of file mtime. The migrator (`src/migrate.rs`, `cargo run --
  migrate-durum`) carries the old files' real OS mtime into this table so migration day doesn't
  reset everyone's "most recently changed" order; it doesn't touch the original files (deleted by
  hand). Verification: 85 tests + clippy 0 warnings + clean fmt + `migrate-durum` run with the real
  binary against a made-up file tree and verified by hand by reading it back with `cargo run --
  chat` (model.md came through correctly) — that session had no real production `durum/` data.
  Current storage formats are described in [state-files.md](state-files.md).
