# Configuration

Start the package and open its printed `http://localhost:8787/setup#token=...` link for normal browser setup. The panel starts before bot credentials are required; its administrator password is separate from platform tokens. See [setup and storage migration](setup-guide.md#5-download-and-start-the-bot). `discord-bot init` remains the [headless CLI path](setup-guide.md#headless-cli-setup), with interactive secrets hidden. [`.env.example`](../.env.example) is the advanced reference. Restart after saving Settings or editing environment settings. Treat `.env` (Compose: `config/.env`) as a password file; typed saves use mode `600` on Unix. On Windows keep the folder private and restrict access in Properties → Security.

| Variable | Required | Purpose |
|---|---|---|
| `DISCORD_TOKEN` | one platform token | Discord bot token; leave blank for Telegram alone |
| `TELEGRAM_TOKEN` | one platform token | Telegram bot token from `@BotFather` `/newbot`; leave blank for Discord alone |
| `TELEGRAM_ROOMS` | for Telegram access | approved comma-separated native negative group IDs or canonical Telegram RoomKeys; blank denies all rooms |
| `OPENROUTER_KEY` / `MISTRAL_KEY` | one | model API key; OpenRouter wins when both are set |
| `PROVIDER` | no | `mistral` forces Mistral |
| `MODEL` | no | paid defaults: `openai/gpt-4o-mini` / `mistral-medium-latest`; a saved `/model` selection takes precedence |
| `API_URL` | no | override the OpenAI-compatible chat/completions endpoint |
| `OWNER_ID` | no | Discord owner user ID; unset means no Discord owner; not a Telegram identity |
| `OWNER_KEY` | no | canonical owner UserKey, e.g. `telegram:user:42` or `discord:user:42`; takes precedence over `OWNER_ID`; invalid explicit values never fall back |
| `TZ_OFFSET` | no | sleep schedule's hours from UTC, default `0`; fractional offsets such as `5.5` work |
| `BOT_LANG` | no | `en` (default) or `tr`, fixed for the process |
| `NEWS_RSS` | no | default BBC World for `en`, Sözcü for `tr` |
| `FIRECRAWL_KEY` | no | page extraction/search; page reads use plain downloads if absent |
| `NEWS_CHANNEL` | no | news channel ID; falls back to the server's system channel |
| `GUILD_ID` / `CHANNELS` | no | single allowed server / comma-separated channel IDs; blank means every accessible server/channel |
| `DEBUG_CHANNEL` | no | destination for enabled decision traces |
| `IMAGE_ANALYSIS` | no | image reading defaults on; `off` disables it at startup |
| `STEALTH` | no | reduced bot UI, default off: skips slash registration/interactions and UI embeds, hides thinking, posts completed replies rather than live edits |
| `LOG_LEVEL` / `LOG_COLOR` | no | `error/warn/info/debug/trace` (default `info`); color defaults on in a terminal |
| `PANEL_ENABLED` | no | browser administration defaults on; `off` requires valid bot/model settings at startup |
| `PANEL_BIND` / `PANEL_PORT` | no | default `127.0.0.1` / `8787`; Compose binds inside the container and publishes localhost only |
| `PANEL_CONFIG_PATH` | no | editable dotenv path, default `.env`; Compose sets `/data/config/.env` |
| `PANEL_PUBLIC_URL` | no | explicit public HTTPS origin when using a correctly configured reverse proxy |

Settings uses typed validation and atomic file replacement, preserving unrelated settings/comments. Token/key fields are write-only: blank keeps the saved value; use the explicit clear control to remove it. External process environment values take precedence over file values and appear read-only. Compose must mount the writable `config` **directory**, not a single dotenv file, and should not keep `env_file` injection for browser-editable fields. Save and restart to activate changes. Do not expose port 8787 publicly; use an [SSH tunnel](setup-guide.md#accessing-a-remote-server). The fixed `discord-bot-data` volume requires explicit [migration](setup-guide.md#existing-compose-installations) from older project-prefixed volumes.

`STEALTH` is not human impersonation and does not remove Discord's bot identity. Leave it off to use slash-command verification and management. Changing it requires a restart; old slash registrations are not removed by this mode.

Set room restrictions **before first launch**. Discord reads ordinary messages and up to 14 days of accessible history on its first scan, including messages not addressed to it. Telegram learns new updates and locally retained text only, not old group history. Use platform permissions as well as configuration, and inform members. See [setup/privacy](setup-guide.md#privacy).

## Telegram

The headless wizard offers `discord` (default) or `telegram`. Telegram-only output
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
