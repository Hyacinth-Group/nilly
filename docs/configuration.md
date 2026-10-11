# Configuration

Run `discord-bot init` for the normal setup path. [`.env.example`](../.env.example) is the advanced reference. Restart after editing environment settings. Treat `.env` as a password file; the wizard sets mode `600` on Unix. On Windows keep the folder private and restrict access in Properties → Security.

| Variable | Required | Purpose |
|---|---|---|
| `DISCORD_TOKEN` | for Discord | Discord bot token; omit for Slack alone or CLI chat |
| `SLACK_APP_TOKEN` / `SLACK_BOT_TOKEN` | both for Slack | `xapp-` app-level Socket Mode token with `connections:write` / installed app's `xoxb-` bot token |
| `SLACK_CHANNELS` | for Slack learning | explicit comma-separated native channel IDs (`C…` / `G…`), not names; missing/blank denies all Slack rooms |
| `SLACK_HISTORY_TIER` | no | default `internal`: history spacing ≥1 second and pages ≤100; `restricted`: spacing ≥60 seconds and pages ≤15 for commercial non-Marketplace apps; honors HTTP 429 `Retry-After` in either mode without automatic retry |
| `SLACK_TEST_CHANNEL` | no | optional native channel ID for deliberately enabled ignored live tests; not a runtime allowlist |
| `OPENROUTER_KEY` / `MISTRAL_KEY` | one | model API key; OpenRouter wins when both are set |
| `PROVIDER` | no | `mistral` forces Mistral |
| `MODEL` | no | paid defaults: `openai/gpt-4o-mini` / `mistral-medium-latest`; a saved `/model` selection takes precedence |
| `API_URL` | no | override the OpenAI-compatible chat/completions endpoint |
| `OWNER_ID` | no | user always favored, and the only user allowed to change `/model`; unset means no owner |
| `OWNER_KEY` | no | canonical owner identity, e.g. `slack:user:T01234567:U01234567`; takes precedence over `OWNER_ID`; invalid nonblank values do not fall back |
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

`STEALTH` is not human impersonation and does not remove Discord's bot identity. Leave it off to use slash-command verification and management. Changing it requires a restart; old slash registrations are not removed by this mode.

Set channel restrictions **before first launch**. The bot reads ordinary messages, including messages not addressed to it. Discord's initial history scan covers up to 14 days. Slack history is bounded and subject to Slack's rate limits and membership permissions; its missing/blank `SLACK_CHANNELS` denies all learning. Use platform permissions as well as configuration, and inform members. See [setup/privacy](setup-guide.md#privacy) and [Slack onboarding](setup-guide.md#slack-socket-mode).

## Slack and simultaneous platforms

Run `discord-bot init` and choose `slack` to create a Slack-only `.env`; Enter at the platform question preserves the Discord setup path. Slack prompts hide both tokens in an interactive terminal. Piped input is not hidden. The wizard collects channel IDs and a qualified owner identity; it does not contact Slack or verify credentials. See the [manifest and installation steps](setup-guide.md#slack-socket-mode).

Configure `DISCORD_TOKEN` alongside both Slack tokens to run Discord and Slack simultaneously in one process. The wizard configures one platform per run; add the second platform's values manually instead of rerunning `init --force`, which replaces `.env`. `GUILD_ID`/`CHANNELS` never substitute for `SLACK_CHANNELS`; each adapter keeps its own authorization boundary.

The brain is shared across enabled platforms; source-qualified person and room keys prevent ID collisions, not information isolation. Only trusted, explicitly approved Slack channels should enter shared memory. Configure Discord permissions/restrictions just as carefully. Private Slack channels also require a bot invitation. The configured owner can administer using text such as `!status` or `@bot status`, with private `chat.postEphemeral` results; no Slack slash commands are registered. `OWNER_KEY` must use the workspace and user IDs, not a display name. Leaving it blank denies Slack administration, including `status`, but does not disable ordinary chat.

`IMAGE_ANALYSIS=off` prevents analysis of attached images, including authenticated Slack private-file downloads. When enabled, private files are resolved through `files.info` and downloaded with the bot token only when needed; credentials and authenticated download URLs are not durable attachment identities. Slack provides no typing indicator or per-channel nickname mutation.


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
