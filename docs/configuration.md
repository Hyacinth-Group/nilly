# Configuration

Run `discord-bot init` for the normal setup path. [`.env.example`](../.env.example) is the advanced reference. Restart after editing environment settings. Treat `.env` as a password file; the wizard sets mode `600` on Unix. On Windows keep the folder private and restrict access in Properties → Security.

| Variable | Required | Purpose |
|---|---|---|
| `DISCORD_TOKEN` | for Discord | Discord bot token; leave blank for a Matrix-only deployment or CLI chat |
| `OPENROUTER_KEY` / `MISTRAL_KEY` | one | model API key; OpenRouter wins when both are set |
| `PROVIDER` | no | `mistral` forces Mistral |
| `MODEL` | no | paid defaults: `openai/gpt-4o-mini` / `mistral-medium-latest`; a saved `/model` selection takes precedence |
| `API_URL` | no | override the OpenAI-compatible chat/completions endpoint |
| `OWNER_ID` | no | user always favored, and the only user allowed to change `/model`; unset means no owner |
| `OWNER_KEY` | for Matrix approval/admin | canonical source-qualified owner; takes precedence over `OWNER_ID`; e.g. `matrix:user:%40owner%3Amatrix.org` |
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

`STEALTH` is not human impersonation and does not remove Discord's bot identity. Leave it off to use slash-command verification and management. Changing it requires a restart; old slash registrations are not removed by this mode.

Set channel restrictions **before first launch**. The bot reads ordinary messages and up to 14 days of accessible history on its first scan, including messages not addressed to it. Use Discord permissions as well as configuration, and inform members. See [setup/privacy](setup-guide.md#privacy).

## Matrix (plaintext only)

The `matrix` Cargo feature is **enabled by default**, including in the Docker
image. Minimal Discord-only builds can opt out with `--no-default-features`.
Explicit `--features matrix` also works; selecting Matrix in `init` cannot add
support to a binary built without the feature. Discord remains the wizard default.

| Variable | Required | Purpose |
|---|---|---|
| `MATRIX_HOMESERVER` | yes | homeserver base URL, e.g. `https://matrix.org`; use HTTPS except for a disposable local test server |
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

**Encrypted rooms and encrypted attachments are unsupported.** The SDK crypto
feature is disabled; an encrypted room never becomes a readable plaintext route.
Create a new room with encryption disabled rather than trying to downgrade an
encrypted room. Plaintext is visible to the homeserver; selected content and
notes also go to the configured model service. Sync is outbound HTTP(S), so the
bot works behind NAT without a public webhook, inbound port, or tunnel.

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
