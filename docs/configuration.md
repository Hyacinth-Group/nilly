# Configuration

Start the package and open its printed `http://localhost:8787/setup#token=...` link for normal browser setup. The panel starts before bot credentials are required; its administrator password is separate from platform tokens. See [setup and storage migration](setup-guide.md#5-download-and-start-the-bot). `discord-bot init` remains the [headless CLI path](setup-guide.md#headless-cli-setup), with interactive secrets hidden. [`.env.example`](../.env.example) is the advanced reference. Restart after saving Settings or editing environment settings. Treat `.env` (Compose: `config/.env`) as a password file; typed saves use mode `600` on Unix. On Windows keep the folder private and restrict access in Properties → Security.

| Variable | Required | Purpose |
|---|---|---|
| `DISCORD_TOKEN` | for Discord | Discord bot token; leave blank for Telegram/Slack/Matrix alone or CLI chat |
| `TELEGRAM_TOKEN` | for Telegram | Telegram bot token from `@BotFather` `/newbot`; leave blank for other platforms alone |
| `TELEGRAM_ROOMS` | for Telegram access | approved comma-separated native negative group IDs or canonical Telegram RoomKeys; blank denies all rooms |
| `SLACK_APP_TOKEN` / `SLACK_BOT_TOKEN` | both for Slack | `xapp-` app-level Socket Mode token with `connections:write` / installed app's `xoxb-` bot token; a partial pair is rejected even with another platform configured |
| `SLACK_CHANNELS` | for Slack learning | explicit comma-separated native channel IDs (`C…` / `G…`), not names; missing/blank denies all Slack rooms |
| `SLACK_HISTORY_TIER` | no | default `internal`: history spacing ≥1 second and pages ≤100; `restricted`: spacing ≥60 seconds and pages ≤15 for commercial non-Marketplace apps; honors HTTP 429 `Retry-After` in either mode without automatic retry |
| `SLACK_TEST_CHANNEL` | no | optional native channel ID for deliberately enabled ignored live tests; not a runtime allowlist |
| `OPENROUTER_KEY` / `MISTRAL_KEY` | one | model API key; OpenRouter wins when both are set |
| `PROVIDER` | no | `mistral` forces Mistral |
| `MODEL` | no | paid defaults: `openai/gpt-4o-mini` / `mistral-medium-latest`; a saved `/model` selection takes precedence |
| `API_URL` | no | override the OpenAI-compatible chat/completions endpoint |
| `OWNER_ID` | no | Discord owner user ID; unset means no Discord owner; not a Telegram, Slack or Matrix identity |
| `OWNER_KEY` | for non-Discord admin; Matrix invitations | canonical source-qualified owner, e.g. `telegram:user:42`, `discord:user:42`, `slack:user:T01234567:U01234567` or `matrix:user:%40owner%3Amatrix.org`; takes precedence over `OWNER_ID`; invalid nonblank explicit values never fall back |
| `TZ_OFFSET` | no | sleep schedule's hours from UTC, default `0`; fractional offsets such as `5.5` work |
| `BOT_LANG` | no | `en` (default) or `tr`, fixed for the process |
| `NEWS_RSS` | no | default BBC World for `en`, Sözcü for `tr` |
| `FIRECRAWL_KEY` | no | page extraction/search; page reads use plain downloads if absent |
| `NEWS_CHANNEL` | no | news channel ID; falls back to the server's system channel |
| `GUILD_ID` / `CHANNELS` | no | Discord-only single allowed server / comma-separated channel IDs; blank means every accessible Discord server/channel |
| `DEBUG_CHANNEL` | no | destination for enabled decision traces |
| `IMAGE_ANALYSIS` | no | image reading defaults on; `off` disables it at startup |
| `STEALTH` | no | reduced bot UI, default off: skips slash registration/interactions and UI embeds, hides thinking, posts completed replies rather than live edits |
| `LOG_LEVEL` / `LOG_COLOR` | no | `error/warn/info/debug/trace` (default `info`); color defaults on in a terminal |
| `PANEL_ENABLED` | no | browser administration defaults on; `off` requires valid bot/model settings at startup |
| `PANEL_BIND` / `PANEL_PORT` | no | default `127.0.0.1` / `8787`; Compose binds inside the container and publishes localhost only |
| `PANEL_CONFIG_PATH` | no | editable dotenv path, default `.env`; Compose sets `/data/config/.env` |
| `PANEL_PUBLIC_URL` | no | explicit public HTTPS origin when using a correctly configured reverse proxy |

Settings uses typed validation and atomic file replacement, preserving unrelated settings/comments. Token/key fields are write-only: blank keeps the saved value; use the explicit clear control to remove it. External process environment values take precedence over file values and appear read-only. Compose must mount the writable `config` **directory**, not a single dotenv file, and should not keep `env_file` injection for browser-editable fields. Save and restart to activate changes. Do not expose port 8787 publicly; use an [SSH tunnel](setup-guide.md#accessing-a-remote-server). The fixed `discord-bot-data` volume requires explicit [migration](setup-guide.md#existing-compose-installations) from older project-prefixed volumes.

Both headless and panel runtime startup derive all configured platform
connections from the same immutable typed settings snapshot. Secrets remain
write-only in Settings: saved plaintext tokens/passwords are not returned in
HTML, configuration APIs or logs. The five-step browser wizard is
**Discord-only**; Telegram, Slack and Matrix use protected Settings plus a
process restart, or CLI init.

`STEALTH` is not human impersonation and does not remove Discord's bot identity. Leave it off to use slash-command verification and management. Changing it requires a restart; old slash registrations are not removed by this mode.

Set room restrictions **before first launch**. Discord reads ordinary messages and up to 14 days of accessible history on its first scan, including messages not addressed to it. Telegram learns new updates and locally retained text only, not old group history. Slack history is bounded, rate-limited and subject to membership permissions; missing/blank `SLACK_CHANNELS` denies all learning. Matrix requires explicit room IDs or owner-approved invitations and rejects encrypted rooms/content without a plaintext fallback. Use platform permissions as well as configuration, and inform members. See [setup/privacy](setup-guide.md#privacy) and each platform's onboarding below.

## Telegram

The headless wizard offers `discord` (default), `telegram`, `slack` and, when compiled with its feature, `matrix`. Telegram-only output
contains `TELEGRAM_TOKEN`, `TELEGRAM_ROOMS` and optional `OWNER_KEY`; no
`DISCORD_TOKEN` is required. The initial five-step browser wizard remains
**Discord-only**. Telegram users complete **Claim and protect**, then visit
**Settings → Platform** for the Telegram token and room allowlist, configure the
provider/model credentials, set the qualified owner in **Privacy**, save, and
**restart the process**. Telegram Settings do not perform a `getMe` credential
test or start a connection live. Alternatively create the Telegram configuration
with CLI init before launching the panel. Both tokens may be set to connect both
networks to the same brain. Approve only rooms whose members trust this shared
memory boundary.

```dotenv
TELEGRAM_TOKEN=<private token from BotFather>
TELEGRAM_ROOMS=-100123,telegram:room:-456
OWNER_KEY=telegram:user:42
OPENROUTER_KEY=<private model key>
```

`TELEGRAM_ROOMS` is **default deny**, unlike blank Discord filters. Discovery,
membership, mentions and administrator status do not approve a room. Native
group/supergroup IDs include their minus sign; do not turn them into positive IDs.
Canonical keys are source-qualified, for example `telegram:room:-100123`. A private
chat can be explicitly approved with its canonical key. Restart after changing
approval. `GUILD_ID`, `CHANNELS`, `NEWS_CHANNEL` and `DEBUG_CHANNEL` are Discord
settings, not Telegram approval aliases. `OWNER_KEY` must identify your numeric
Telegram user ID, not `@username`, a chat ID, or the bot's ID; leave it blank to
deny Telegram administration.

Disable group privacy with `@BotFather` `/setprivacy` → **Disable**, then remove
and re-add the bot to existing groups. Alternatively make it a group
administrator; administrators receive ordinary messages regardless of privacy.
Give it permission to send messages. Receiving human reaction updates additionally
requires administrator status; only emoji allowed by the chat can be sent as
reactions. Paid/custom emoji are not advertised by this adapter. A `getMe`
response from a separate API probe proves account authentication only, not group
visibility. Account privacy metadata and discovered room capabilities can
identify limited visibility; unknown visibility is not evidence of ordinary
group-message access. Overview's limited-visibility warning is not a claim that
undiscovered rooms are fully readable.

The runner uses outbound HTTPS `getUpdates` long polling and works behind NAT
without a public port or webhook. Do not run two pollers with the same token;
remove any existing webhook before polling. Telegram keeps pending updates for
at most 24 hours; this is not a group-history API. The adapter cannot change a
per-group nickname. Plain output splits at **4096 Unicode scalars**, with
thread/reply routing kept native. Forum topics retain their native thread IDs,
but `TELEGRAM_ROOMS` approves the entire chat, **not individual topics**. There is
no topic-specific allowlist or topic creation/closure/administration interface.

Image analysis follows `IMAGE_ANALYSIS=off`: disabled images are not resolved or
downloaded. Enabled images are resolved through `getFile` and fetched from the
validated Telegram file endpoint; token-bearing URLs never enter model prompts,
durable identities or logs. Outgoing local files use multipart uploads rather
than exposing local paths or asking Telegram to fetch arbitrary URLs. Telegram's
hosted API currently limits downloads to 20 MB and general-file uploads to 50 MB.

For a tokenless **offline** smoke, run the bundled Telegram fixtures and
wizard identity/approval checks. The live probe below remains ignored:

```sh
cargo test --locked telegram
```

For a separate credential-free startup/authentication-failure smoke with an
already-built release binary, isolate it from any real `.env` or saved memory:

```sh
bot="$PWD/target/release/discord-bot"
scratch="$(mktemp -d)"
(cd "$scratch" && env -i HOME="$HOME" PATH="$PATH" PANEL_ENABLED=off TELEGRAM_TOKEN=123:fake OPENROUTER_KEY=x "$bot")
```

Expect an authentication failure such as HTTP 401, with no real group traffic.
This fake-token check **contacts Telegram and requires network access**; it is
not an offline test or a successful-authentication claim. Keep the temporary
state separate from production data. A fake model key is sufficient because
startup must reject Telegram authentication before model/chat work.

The opt-in `getMe` probe is ignored by the normal suite and authenticates only;
it does not send messages or prove group permissions. This is an optional command,
not a claim that a live probe, group conversation or deployment has been exercised:

```sh
TELEGRAM_TOKEN=... cargo test --locked telegram_live_get_me -- --ignored
```

Use a private test token through your secret manager; avoid saving it in shell
history. No live probe is implied by passing offline tests.

Official references: [Telegram Bot API](https://core.telegram.org/bots/api)
and [privacy mode](https://core.telegram.org/bots/features#privacy-mode).

## Slack and simultaneous platforms

Run `discord-bot init` and choose `slack` to create a Slack-only `.env`; Enter at the platform question preserves the Discord setup path. Slack prompts hide both tokens in an interactive terminal. Piped input is not hidden. The wizard collects channel IDs and a qualified owner identity; it does not contact Slack or verify credentials. See the [manifest and installation steps](setup-guide.md#slack-socket-mode).

The five-step browser wizard remains **Discord-only**. For Slack, complete **Claim and protect**, then use protected **Settings → Platform** for both tokens, native channel approval and the history tier. Configure the provider/model credentials and language, set the qualified owner in **Privacy**, save and **restart the process**. Settings does not perform a live Slack credential test or start Slack without restarting. Both Slack tokens are required together: a partial pair is rejected even if Discord or Telegram is also configured.

Configure `DISCORD_TOKEN`, `TELEGRAM_TOKEN` and/or the Matrix settings alongside both Slack tokens to run several platforms simultaneously in one process. The wizard configures one platform per run; add other platforms through protected Settings or edit the private file rather than rerunning `init --force`. `GUILD_ID`/`CHANNELS` never substitute for another platform's room approval; each adapter keeps its own authorization boundary.

The brain is shared across enabled platforms; source-qualified person and room keys prevent ID collisions, not information isolation. Only trusted, explicitly approved Slack channels should enter shared memory. Configure Discord permissions/restrictions just as carefully. Private Slack channels also require a bot invitation. The configured owner can administer using text such as `!status` or `@bot status`, with private `chat.postEphemeral` results; no Slack slash commands are registered. `OWNER_KEY` must use the workspace and user IDs, not a display name. Leaving it blank denies Slack administration, including `status`, but does not disable ordinary chat.

`IMAGE_ANALYSIS=off` prevents analysis of attached images, including authenticated Slack private-file downloads. When enabled, private files are resolved through `files.info` and downloaded with the bot token only when needed; credentials and authenticated download URLs are not durable attachment identities. Slack provides no typing indicator or per-channel nickname mutation.


## Matrix (plaintext only)

The optional native **`matrix-sdk` 0.19.1** dependency uses the `matrix` Cargo
feature, **enabled by default**, including in the Docker image. Source builds
require **Rust 1.96** (MSRV). Builds can opt out with `--no-default-features`
while retaining Discord, Telegram and Slack. Explicit `--features matrix` also
works. CLI init offers Matrix only when compiled with the feature; a build
without it reports nonblank Matrix configuration as an explicit configuration
error, never silently ignoring it. Discord remains the CLI wizard default.

The panel still starts **before** platform credential validation. Matrix users
complete **Claim and protect**, then use protected **Settings → Platform** for
native SDK settings, **Privacy** for the qualified owner, and provider/model
settings. Save and **restart the process**; neither Settings nor CLI init
performs a live Matrix authentication test. Interactive CLI token/password input
uses hidden `rpassword` entry; piped input is not hidden. CLI answers use the
same typed patch/save path as Settings.

| Variable | Required | Purpose |
|---|---|---|
| `MATRIX_HOMESERVER` | yes | HTTP(S) base URL without embedded credentials, query or fragment, e.g. `https://matrix.org`; prefer HTTPS |
| `MATRIX_USER` | yes | dedicated bot's full Matrix user ID, e.g. `@mybot:matrix.org` |
| `MATRIX_ACCESS_TOKEN` / `MATRIX_PASSWORD` | one | access token preferred; password login produces a saved token and restores it on restart without creating new devices; keep both secret |
| `MATRIX_ROOMS` | no | explicit comma-separated native room-ID allowlist, e.g. `!one:matrix.org,!opaqueRoomId`; modern opaque IDs are supported, but aliases and canonical keys are not room IDs |
| `OWNER_KEY` | for invitations/admin | canonical percent-encoded owner identity, e.g. `matrix:user:%40owner%3Amatrix.org` for `@owner:matrix.org` |

```dotenv
MATRIX_HOMESERVER=https://matrix.org
MATRIX_USER=@mybot:matrix.org
MATRIX_ACCESS_TOKEN=replace_with_the_bot_accounts_secret_token
MATRIX_ROOMS=!approved_room:matrix.org
OWNER_KEY=matrix:user:%40owner%3Amatrix.org
OPENROUTER_KEY=replace_with_your_model_key
BOT_LANG=en
```

For password login, replace the `MATRIX_ACCESS_TOKEN` line with
`MATRIX_PASSWORD=...`. Do not use your personal Matrix account as the bot.
The wizard accepts a raw owner ID and writes its canonical key for you. Native
`@`, `:`, and other reserved characters in an `OWNER_KEY` component must be
percent-encoded using uppercase hex; raw `matrix:user:@owner:matrix.org` is invalid.
An invalid nonblank `OWNER_KEY` never falls back to `OWNER_ID`.

**Access is explicit.** Blank `MATRIX_ROOMS` does not mean all joined rooms.
Only allowlisted rooms or invitations sent by the configured owner are approved;
previously joined unapproved rooms are ignored. Owner invitation approvals are
persisted in `matrix/<connection>/rooms.json` under the state directory.
Use a dedicated plaintext room and inform members before approval. Approved
rooms share the same brain, so notes learned in one room can influence another.
Changing a room's membership is not a memory-isolation boundary.

**Encrypted rooms and encrypted attachments are unsupported.** SDK crypto is
disabled: there is no crypto store, no decryption capability, and no plaintext
fallback. Room capabilities and Overview describe only actually readable
plaintext rooms, never encryption support. Create a new room with encryption
disabled rather than trying to downgrade an encrypted room. Plaintext is
visible to the homeserver; selected content and notes also go to the configured
model service. Sync is outbound HTTP(S), so the bot works behind NAT without a
public webhook, inbound port, or tunnel.

As the owner, use prefix commands such as `!status` and `!help` (`!durum` /
`!yardim` with Turkish UI). Matrix administration is owner-only. The bot invites
the owner to a dedicated private **plaintext admin room** for command results:
accept that invitation and run `!status` there. Only bot/owner may be joined or
invited; encryption or extra members make the route unavailable. This room is
excluded from conversational memory and persisted in `admin-room.json`.
Private results, including mind/settings detail, are denied in public rooms
rather than exposed as ordinary text; Matrix has no Discord-style ephemeral
response and there is no public fallback. The homeserver can still read
plaintext admin results. Keep `STEALTH` off for command verification.
Run room-targeted actions (for example `!news` or `!reset`) from an approved group;
their results still go to the admin room. Shared-brain actions such as `!status`
also work inside the admin room. Unknown `!bot <action>` requests return shared
localized help; unrelated exclamations are not administrative commands.


Session/device identity, sync progress and the SDK SQLite state live under the
state directory, not inside redb. Protect these files like `.env`; keep them
across restarts to preserve the bot device and avoid replaying sync from scratch.
See [state files](state-files.md#matrix-session-and-sync-state) and the
[beginner Matrix setup](setup-guide.md#matrix-plaintext-room-setup).

### Disposable Matrix integration test

The two ignored live tests require a disposable homeserver/account, not real
rooms or personal credentials. Set `MATRIX_TEST_HOMESERVER`, `MATRIX_TEST_USER`
(full `@id:server`) and `MATRIX_TEST_PASSWORD` privately in the test process, then:

```sh
cargo test --features matrix matrix_live -- --ignored
```

The tests make real network requests. One confirms device/token restoration after
restart; the other creates a room, exercises sending, editing, reactions,
redaction, typing, nickname and history, enables encryption, and verifies that
the adapter refuses subsequent plaintext delivery. Do not commit these
environment values or reuse production session files.


## Terminal-only tools

```sh
cargo run -- chat
```

A model key is needed, not a Discord token. Enter `name: text`, then `!quit` or Ctrl-D. This bench reads existing memory but does not save conversational changes; initial setup may create the database. It is not a Discord demo.

```sh
cargo run -- migrate-durum [--from <dir>] [--to <redb-path>] [--dry-run] [--force]
```

Imports the legacy Markdown tree into redb without moving or deleting the source. See [state files](state-files.md).

## Upgrading from 1.0

To preserve the previous Turkish language, UTC+3 schedule, and owner model control, add:

```dotenv
BOT_LANG=tr
TZ_OFFSET=3
OWNER_ID=<your Discord user ID>
```

Back up saved data before upgrades. For the new Compose volume name, follow [existing-installation migration](setup-guide.md#existing-compose-installations).

Code constants and limits: [constants.md](constants.md). Commands: [commands.md](commands.md).
