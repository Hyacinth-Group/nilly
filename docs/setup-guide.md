# Your first bot: Discord or Slack

This bot keeps conversation notes and uses them alongside a fixed persona in later replies. You do not need to write code, but setup still requires copying commands into Terminal or PowerShell.

For Discord, follow the numbered guide below. For Slack, use [Slack Socket Mode](#slack-socket-mode); you do not need a Discord account or token. Both platforms can run together with one shared brain, so approve only channels whose members understand that their information may influence replies elsewhere.

## What you need for Discord

- [ ] A Discord account.
- [ ] A Discord server where you are an administrator.
- [ ] A computer or a small server with internet access.
- [ ] A Discord bot token, a model account, and Docker or a release binary.

Keep the computer on for the bot to stay online.

> **Before inviting the bot: data flow and privacy**
>
> - **Stored locally:** person/topic notes, summaries and bounded channel history in `durum/hafiza.redb` (memory database); raw overflow in `durum/arsiv/` (archive), plus your backups.
> - **Sent to your model service:** selected chat text and saved notes for replies and background analysis. OpenRouter also routes to the serving model provider. Images may be sent; image analysis defaults on (`IMAGE_ANALYSIS=off` disables it).
> - **Other services:** Firecrawl receives page URLs and search queries when configured.
>
> Tell members which channels the bot can read. It reads ordinary messages, not only mentions, and observes messages while asleep. Discord's first scan reads up to **14 days** of accessible text history: set `CHANNELS` before first launch; **blank `GUILD_ID` / `CHANNELS` means every accessible Discord server / channel**. Slack requires explicit `SLACK_CHANNELS`; blank denies all Slack learning and history is bounded/rate-limited. Selective member deletion is not supported, and resetting a chat does not erase stored records. [Retention details](#privacy).

## What does it cost?

The bot itself is free, open-source software. You bring your own model account. A **model** is the AI that writes the bot's replies.

**You can try a free model.** The live OpenRouter catalog listed `google/gemma-4-31b-it:free` on 11 October 2026. This is a catalog-verified example, not an end-to-end bot compatibility claim. Availability and limits can change. Copy the exact ID into the wizard's model prompt. Enter alone selects the provider's **paid** default, not a free model.

Free-model requests are rate-limited; consult [OpenRouter's current limits](https://openrouter.ai/docs/api_reference/limits). A **request** is one model call, not one Discord message. Background analysis makes requests too and can exhaust the free allowance before ordinary chat does.

We have not published a measured running cost yet. Replies and background analysis both use your model key. Start with a provider spending limit and check the provider's usage dashboard. A rented server may have its own bill.

## Slack Socket Mode

You need a Slack account, permission to install an app in your workspace (or an administrator who can approve it), a model API key, and a computer/server that stays online. **Socket Mode** opens an outbound WebSocket connection; there is no public webhook server, signing secret, or incoming port to configure.

### 1. Create the app from the supplied manifest

1. Open [Slack's app management page](https://api.slack.com/apps), sign in, and select **Create New App → From a manifest**.
2. Choose the workspace where the bot will operate.
3. Choose **YAML**, replace the example with the full contents of [`slack-app-manifest.yml`](slack-app-manifest.yml), review it, and create the app. You may change the display name; do not silently broaden the scopes.
4. Open **Socket Mode** and check that it is enabled.
5. Under **OAuth & Permissions → Bot Token Scopes**, check this exact list:
   - `app_mentions:read`
   - `channels:history`, `channels:read`
   - `groups:history`, `groups:read`
   - `chat:write`
   - `files:read`, `files:write`
   - `reactions:read`, `reactions:write`
   - `users:read`
6. Under **Event Subscriptions**, check the bot events: `app_mention`, `message.channels`, `message.groups`, `member_joined_channel`, `reaction_added`.

`channels:*` covers public channels; `groups:*` covers private channels the bot has joined. The manifest does not grant direct-message reading, `chat:write.public`, user-token scopes, or slash commands. It contains no credentials. Slack's own `/invite` remains available; it is not a command registered by this bot.

### 2. Install and obtain the two different tokens

1. Under **OAuth & Permissions**, select **Install to Workspace** and approve the requested bot scopes. Follow your workspace's administrator approval process if required.
2. Copy the **Bot User OAuth Token**, beginning `xoxb-`. This is `SLACK_BOT_TOKEN`, used for Slack Web API calls.
3. Under **Basic Information → App-Level Tokens**, select **Generate Token and Scopes**.
4. Name it, add only **`connections:write`**, generate it, and copy the token beginning `xapp-`. This is `SLACK_APP_TOKEN`, used to open Socket Mode.
5. Keep both tokens private. Reinstall the app after changing bot scopes or event subscriptions so the installation receives the new permissions.

**Do not add `connections:write` to bot scopes.** It is an app-level token scope created separately; the manifest cannot create or contain an app-level secret. Rotate leaked tokens in Slack app management. Never paste them in Slack, a screenshot, a Git commit, or a support log.

### 3. Approve channels, invite the bot, and find the owner identity

1. Agree with members which channels may be processed and stored. Start with one trusted test channel.
2. In each approved public or private channel, use Slack's **`/invite @your-bot-name`**. Installation alone does not join channels. A private channel requires an invitation from a member.
3. Open channel details and copy the **channel ID** (or take it from the channel's browser URL). IDs begin `C…` or `G…`; `#general` is a name and is not accepted.
4. Save a comma-separated list, such as `C01234567,G01234567`, for **`SLACK_CHANNELS`**. Missing/blank means **no Slack channel is approved for learning**, not "all channels." Never substitute Discord's `CHANNELS`.
5. Optionally find your own Slack member ID: open your profile's **More/… → Copy member ID**. It normally begins `U…`. Open Slack in a browser; the workspace ID is the `T…` component of `https://app.slack.com/client/T…/C…`.
6. Combine them as **`OWNER_KEY=slack:user:T01234567:U01234567`**, using your actual workspace/member IDs. This is a canonical identity, not your username or email. `OWNER_KEY` takes precedence over legacy Discord `OWNER_ID`; an invalid nonblank key does not fall back. The setting is optional for ordinary chat, but leaving it blank denies Slack administration, including `status`.

All enabled platforms share notes. Source-qualified IDs keep different people's records distinct but do **not** isolate their information from the shared brain. Only trusted, approved allowlisted channels should contribute. Private channel membership does not make model processing local or prevent learned context influencing another approved room.

### 4. Run the wizard and start

Install Docker using [step 4 below](#4-install-docker), then use the same download/wizard commands in [step 5](#5-download-and-start-the-bot). With a downloaded release binary, run:

```sh
discord-bot init
```

Choose **`slack`** at the new platform question. Enter alone chooses Discord and keeps the existing Discord setup behavior.

| Slack wizard question | What to enter |
| --- | --- |
| App-level token | Your `xapp-` token with `connections:write`; hidden on an interactive terminal. |
| Bot token | Your installed app's `xoxb-` token; hidden on an interactive terminal. |
| Approved channel IDs | Native comma-separated IDs from step 3; Enter approves none. |
| Model provider / key / model | OpenRouter or Mistral, your model key, and your selected model ID; see the cost notes above. |
| Language | `en` or `tr`. |
| Owner identity | `slack:user:<workspace ID>:<member ID>`, or Enter for no owner (Slack administration, including `status`, is then denied). |
| UTC offset | Hours from UTC for the sleep schedule, such as `3`, `-5`, or `5.5`. |

The resulting `.env` is Slack-only: it does not require or write `DISCORD_TOKEN`. The wizard does not contact Slack or prove token validity. Piped input is not hidden. New Slack prompts have English/Turkish translations; `BOT_LANG=tr` selects Turkish prompts when launching the wizard. The selected language is saved for subsequent bot runs. Protect `.env` (Unix mode `600`; on Windows restrict folder access).

Start with `discord-bot`, `cargo run --release`, or `docker compose up -d`, depending on installation. **As the configured `OWNER_KEY` owner**, in an invited, allowlisted channel, send **`!status`** or mention the bot followed by **`status`**. An unset owner means this administration check is denied; ordinary chat does not require an owner. Do not use `/status` in Slack: no Slack slash commands are registered. Administration goes through the shared service and returns **private ephemeral messages** (`chat.postEphemeral`), not public status cards or a direct-message fallback.

To run Slack and Discord simultaneously, add `DISCORD_TOKEN` and your Discord channel restrictions to the same `.env`; retain both Slack tokens and `SLACK_CHANNELS`. The wizard configures one platform at a time. Do not rerun `init --force` to add another platform: it replaces the file. Each platform keeps its own room permissions and approval settings.

### History, files, and capability limits

- **Bounded history, not a workspace export:** only explicitly approved rooms the bot can access are eligible. Retrieval is bounded and rate-limited; it is not a guarantee of complete past conversations or a complete thread archive.
- **Slack history rate policy:** internal customer-built apps and Marketplace-approved apps retain **Tier 3** history limits (50+ requests/minute). Commercially distributed, non-Marketplace apps are limited to **1 request/minute** and **15 messages/request** for `conversations.history` and `conversations.replies` under Slack's newer policy. Internal does not mean "anything self-hosted"; check your app's distribution classification. The adapter defaults to **`SLACK_HISTORY_TIER=internal`**, reserving at least 1 second between history calls with at most 100 messages/page. For the commercial non-Marketplace class, set **`SLACK_HISTORY_TIER=restricted`** before launch: at least 60 seconds between calls and at most 15 messages/page. Both modes honor returned HTTP 429 `Retry-After` and report the rate limit without automatically retrying. Do not raise request volume just because your workspace is paid. [Slack policy](https://docs.slack.dev/changelog/2025/05/29/rate-limit-changes-for-non-marketplace-apps), [history reference](https://docs.slack.dev/reference/methods/conversations.history).
- **Private files:** `files:read` permits access only within Slack's authorization boundary. Attachments keep a native file ID; when needed, `files.info` resolves a private download and the bot authenticates it. Do not copy signed/authenticated file URLs into memory. Image analysis may send downloaded images to the model provider; set **`IMAGE_ANALYSIS=off`** before launch to disable attached-image processing, including Slack private files.
- **Supported output:** text and thread-aware replies, uploads within Slack's limits, edits/deletes of the bot's own messages, and reactions using Slack emoji names (including custom names) or a supported known Unicode-to-name mapping. Unknown Unicode reactions are explicitly unsupported, not guessed.
- **Output length:** a raw chunk is at most 3,900 Unicode scalars; Slack escaping can expand text, so rendered chunks are split safely to stay within a 4,000-scalar target. Longer replies may become several messages.
- **Not supported:** a typing indicator, per-channel bot nicknames, account-profile mutation as a nickname fallback, interactive Discord-style menus/modals, and Slack slash commands. An ephemeral result is transient and recipient-only, not durable private storage.

### Optional live tests (not onboarding verification)

Ordinary tests are offline. Live Slack tests are **ignored and environment-gated**; they require real `SLACK_APP_TOKEN` and `SLACK_BOT_TOKEN`. Set optional `SLACK_TEST_CHANNEL` to an invited test channel only when deliberately permitting channel-scoped probes (which may create/edit/react to/delete bot messages). Keep tokens out of command history and logs:

```sh
cargo test --locked slack:: -- --ignored
```

This is an opt-in command, not evidence that a live workspace was tested. Without real credentials, do not report Slack end-to-end verification. The integration owner separately runs the release startup smoke with fake tokens; that checks startup/failure behavior, not a successful Slack connection.


## 1. Create the bot in Discord

Open the [Discord Developer Portal](https://discord.com/developers/applications) and sign in. This is Discord's website for creating bots; you do not need to be a developer.

1. Click **New Application**.
2. Give it a name, such as `My Bot`. Accept the terms and click **Create**. The bot can choose a nickname later.
3. Click **Bot** in the left sidebar.
4. Under **Token**, click **Reset Token** and confirm any security prompt.
5. Copy the token and keep it somewhere private until setup asks for it. A **token** is the bot's password. Never post it in Discord or share it in a screenshot.
6. Still on **Bot**, scroll to **Privileged Gateway Intents**. These switches let Discord send the bot messages and member information.
7. Turn on **Message Content Intent** and **Server Members Intent**. Leave **Presence Intent** off.
8. Click **Save Changes** if it appears.

- [ ] Both required intent switches are on.
- [ ] I have saved my bot token privately.

## 2. Get an OpenRouter key

An **API key** is the password that lets the bot use your AI account.

1. Open [openrouter.ai/keys](https://openrouter.ai/keys).
2. Sign up or sign in.
3. Create a key. Give it a name such as `Discord bot`, and set a spending limit.
4. Copy the key and keep it private. You will paste it into the setup wizard.
5. If you want a paid model, add credits to your OpenRouter account. If you want to start free, choose a current `:free` model instead.

Already using Mistral? You can use your Mistral API key instead. Choose `mistral` when the wizard asks for a provider. The free limits above are for OpenRouter, not Mistral.

## 3. Find your Discord user ID

This optional step tells the bot who owns it. The owner is always liked and can switch models with `/model`.

1. In the Discord desktop app, click the gear next to your name to open **User Settings**.
2. Open **Advanced**.
3. Turn on **Developer Mode**. This only makes extra copy options visible.
4. Return to your server. Right-click your own name in the member list or on a message.
5. Click **Copy User ID**. Save the number for setup.

Your user ID is a number, not your username. You can skip it if you prefer.

## 4. Install Docker

**Docker** runs the bot in a ready-made package, so you do not have to install Rust or build anything.

### Windows or Mac

Install [Docker Desktop](https://docs.docker.com/desktop/). Choose the download for your computer, follow its installer, and open Docker Desktop. Wait until it says Docker is running. On Windows, use Linux containers for this bot.

### Linux

Open a terminal and run:

```sh
curl -fsSL https://get.docker.com | sh
```

This downloads and runs Docker's installation script. It may ask for your computer password. Docker recommends this shortcut for testing and development; for a production server, use the [official installation instructions](https://docs.docker.com/engine/install/ubuntu/) for your Linux distribution.

If a later Docker command says `permission denied`, put `sudo` before `docker`, or follow Docker's [Linux post-installation instructions](https://docs.docker.com/engine/install/linux-postinstall/).

## 5. Download and start the bot

Open **Terminal** on Mac or Linux, or **PowerShell** on Windows. This is a window where you paste commands. Paste one command at a time and press Enter.

### Command 1: make a folder

```sh
mkdir mybot && cd mybot
```

This creates a folder called `mybot` and moves you into it.

On Windows PowerShell, use these two lines instead, so this also works with older PowerShell versions:

```powershell
mkdir mybot
cd mybot
```

### Command 2: download the launch settings

```sh
curl -O https://raw.githubusercontent.com/krxi/discord-bot/main/docker-compose.yml
```

This downloads the file that tells Docker how to run the bot.

On Windows PowerShell, use:

```powershell
curl.exe -O https://raw.githubusercontent.com/krxi/discord-bot/main/docker-compose.yml
```

### Command 3: run the setup wizard

```sh
docker run --rm -it --user "$(id -u):$(id -g)" -v "$PWD:/data" ghcr.io/krxi/discord-bot:latest init
```

This downloads the bot's ready-made package and asks you for its settings.

On Windows PowerShell, use this version instead:

```powershell
docker run --rm -it -v "${PWD}:/data" ghcr.io/krxi/discord-bot:latest init
```

The wizard asks these questions in order:

| Question | What to enter |
| --- | --- |
| Platform | Press Enter for `discord`, or type `slack` and follow the Slack section above. This is asked before the token. |
| Discord bot token | The token from step 1; terminal input is hidden. |
| Provider | Press Enter for `openrouter`, or type `mistral`. |
| API key | Your chosen provider's key from step 2; terminal input is hidden. |
| Model ID | Free OpenRouter example: `google/gemma-4-31b-it:free`. Enter alone uses the provider's **paid** default. |
| Language | Press Enter for `en` (English), or type `tr` (Turkish). |
| Your Discord user ID | Paste the number from step 3, or press Enter to skip. |
| UTC offset hours | Your local time difference from UTC, or Enter for `0`. For example, `3` means UTC+3. This sets the bot's day and night. |

The wizard saves a file called `.env` in `mybot`. It contains your settings and secret keys. It also prints an invite link and a reminder about the two intent switches.

Before command 4, open `.env` and add `GUILD_ID=<your server ID>` and `CHANNELS=<channel ID>` (comma-separated for several channels). With Discord Developer Mode enabled, right-click the server or channel to copy its ID. Remove the `<...>` example text and paste the numeric IDs. Empty values mean **all** accessible servers/channels, including history scanning.

Keep this folder private. Never upload or share `.env`. On Windows restrict folder access in **Properties → Security**; Unix init sets mode `600`. Rotate a leaked Discord token in Developer Portal and a leaked model key in its provider account.

**Keep the invite link handy. Do not share your `.env` file.**

### Command 4: start the bot

```sh
docker compose up -d
```

This starts the bot in the background, so closing the terminal does not stop it.

The first download can take a little while. Leave Docker Desktop running if you use it.

## 6. Invite it to your server

Open the invite link printed by the wizard in your browser. Choose your server and follow Discord's permission and approval prompts.

If you missed the link, run `docker compose logs -f` in the `mybot` folder. When the bot is not in any server yet, it prints the invite link again during startup. Press Ctrl+C to leave the log view; this does not stop the bot.

- [ ] The bot appears in my server's member list.
- [ ] The bot can see and send messages in the channel where I want to talk.

## 7. Verify with a command, then say hi

Run **`/status`** in a channel the bot can access (`/durum` if you selected Turkish). You are done when its status card appears. This command does not depend on its willingness to chat or sleep schedule. Keep `STEALTH` off; reduced UI mode ignores slash interactions.

If no status card appears within 30 seconds, use [the troubleshooting checklist](#the-bot-is-online-but-silent-what-should-i-check). Then try `@My Bot hi!`, choosing the bot from Discord's mention list.

It chooses when to join ordinary conversations, may react instead of replying, and sleeps at night. Name selection is gradual, not an instant setup step: the established stage starts after at least 10 days and 25 finished chats.

## Everyday controls

Run these from the `mybot` folder:

| What you want | Command |
| --- | --- |
| See what is happening | `docker compose logs -f` |
| Stop the bot | `docker compose down` |
| Start it again | `docker compose up -d` |
| Update it | `docker compose pull && docker compose up -d` |

On older Windows PowerShell, run the two update commands on separate lines: first `docker compose pull`, then `docker compose up -d`.

## Troubleshooting

### The bot is online but silent. What should I check?

First, check **Bot → Privileged Gateway Intents** in the Developer Portal. Both **Message Content Intent** and **Server Members Intent** must be on. Save changes, then run `docker compose restart` from `mybot`.

If an intent is off, Discord may reject the connection entirely. Check channel read/write permissions, `.env` scope filters, and that `STEALTH` is off; retry `/status`. If status works but chat does not, check logs for provider errors/exhausted limits and consider the sleep schedule. Ordinary chat is not a deterministic health check.

### It says “DISCORD_TOKEN is missing”. What does that mean?

The bot could not find its password. Check that `.env` exists in `mybot` and run from that folder. An existing-file refusal leaves your settings unchanged: edit `.env` directly, or back it up privately before rerunning `init --force` to replace it. Then run `docker compose up -d`.

For a downloaded binary or a source build, run `init` and start the bot from the same folder. Do not rename `.env` to `.env.txt`.

### Windows says `$(id -u)` does not work. What should I paste?

That part of the Mac/Linux command finds your computer's user number. PowerShell does not use it. Run this instead, from `mybot`:

```powershell
docker run --rm -it -v "${PWD}:/data" ghcr.io/krxi/discord-bot:latest init
```

### How do I see the logs?

Run `docker compose logs -f` from `mybot`. **Logs** are the bot's running activity and error messages. Press Ctrl+C to stop watching. Do not share tokens, API keys, or private conversation text when asking for help.

## Where your memories live

Docker keeps **saved bot data** in the explicitly named volume `discord-bot-data`. The Compose key is still `durum` (“state/data”), but the actual volume name no longer depends on your folder. A volume survives stopping or replacing the container. Multiple installations using this Compose file share that name; choose distinct names if intentionally running separate bots.

Channel history rolls over; larger records are summarized and raw overflow can be archived. `docker compose down` keeps saved data. **Do not use `docker compose down -v`** unless you mean to delete it. Existing project-prefixed volumes need migration below before switching to the new name.

### Back up the memory

From `mybot`, stop the bot and verify the existing volume and database before copying. `docker volume inspect` fails instead of silently creating a misspelled volume. If inspection or the database check fails, stop and investigate; do not continue to tar.

```sh
docker compose down
docker volume inspect discord-bot-data
docker run --rm -v discord-bot-data:/data:ro ubuntu test -s /data/hafiza.redb
docker run --rm -v discord-bot-data:/data:ro -v "$PWD:/backup" ubuntu tar czf /backup/durum-backup.tar.gz -C /data .
docker compose up -d
```

On Windows PowerShell, replace the tar command with:

```powershell
docker run --rm -v discord-bot-data:/data:ro -v "${PWD}:/backup" ubuntu tar czf /backup/durum-backup.tar.gz -C /data .
```

This writes `durum-backup.tar.gz`. Protect it like member data; use a different filename to keep earlier backups. Save `.env` separately and privately. Restore procedures below have not been exercised in a live deployment.

### Restore a backup

Stop the bot, inspect the destination volume, and save a separate copy of its current contents before replacing them. Restore to an **empty** volume; extracting over an existing archive tree can leave stale files.

```sh
docker compose down
docker volume inspect discord-bot-data
docker run --rm -v discord-bot-data:/data -v "$PWD:/backup:ro" ubuntu sh -c 'test -z "$(find /data -mindepth 1 -print -quit)" && tar xzf /backup/durum-backup.tar.gz -C /data'
docker compose up -d
```

For PowerShell, use `"${PWD}:/backup:ro"` instead of `"$PWD:/backup:ro"`. If the empty-volume check fails, nothing is extracted: do not bypass it without first preserving the current data. Use `/status` and `/mind` after startup to inspect the result.

### Existing Compose installations

Changing the volume name does **not** migrate data. Before adopting the new Compose file, stop the old installation and identify its existing volume (often `mybot_durum`) with `docker volume ls`; do not guess. Keep the old volume until the migrated data is confirmed.

For example, after verifying `mybot_durum` is your old nonempty volume:

```sh
docker volume inspect mybot_durum
docker run --rm -v mybot_durum:/data:ro ubuntu test -s /data/hafiza.redb
docker run --rm -v mybot_durum:/data:ro -v "$PWD:/backup" ubuntu tar czf /backup/durum-backup.tar.gz -C /data .
docker volume create discord-bot-data
```

Then use the empty-volume restore procedure above before starting the new Compose service. For PowerShell use `"${PWD}:/backup"` for the backup mount. These migration instructions are not a claim of a tested live migration.

For a binary or source build, memory lives in the local `durum/` folder. Stop the bot before copying that folder to your backup location.

## Privacy

Tell members before inviting it: accessible ordinary messages may be processed, not just mentions. Discord's first scan can include up to 14 days of history, subject to `CHANNELS` and Discord permissions. Slack uses an explicit `SLACK_CHANNELS` allowlist (blank denies all), membership permissions and bounded rate-limited history. Observation continues while asleep. Replies and background analysis send selected chat text and saved notes to the chosen model service (including OpenRouter's serving provider). Attached images, including authorized Slack private-file images, can be sent because image analysis defaults on; `IMAGE_ANALYSIS=off` disables this. Configured Firecrawl receives page URLs and search queries. Enabled platforms share memory, so approve only trusted channels and explain that notes may influence replies in other approved rooms.

Saved data stays on your machine, but processing is **not** local. Channel histories are bounded; derived person/topic notes, summaries and archived raw text can remain after a conversation. **Selective member deletion is not supported.** `/reset` resets channel/chat state, not stored member records or archives. Do not promise members an erasure capability the bot does not have.

For a complete local reset, stop the bot, delete its entire saved-data volume (or `durum/` folder for binary/source installs), and remove every backup and exported log/transcript containing the data. This is destructive and removes all members' data, not one member's. It does not delete copies already processed/retained by model services or Firecrawl; review their retention policies and deletion channels separately. Keep the bot stopped or remove its permissions to avoid recollection.

## Prefer not to use Docker?

These routes use the same token, key, and intent settings from steps 1–3.

### Download a release

No Docker or Rust is needed, but a **terminal is required**. Download the matching archive from [GitHub Releases](https://github.com/krxi/discord-bot/releases): Linux x86_64 or arm64, macOS Apple Silicon, or Windows. Extract it into a private folder, open a terminal there, and run:

```sh
./discord-bot init
./discord-bot
```

On Mac or Linux, if the file is not executable, run `chmod +x discord-bot` first. On Windows PowerShell, use:

```powershell
.\discord-bot.exe init
.\discord-bot.exe
```

Keep this terminal open while the bot runs. Continue with the printed invite link and step 7 above.

### Build from source

This route is for people who already have Git and Rust installed. From a terminal:

```sh
git clone https://github.com/krxi/discord-bot.git
cd discord-bot
cargo run -- init
cargo run --release
```

The first build takes longer. Keep the terminal open, restrict scope in `.env`, invite it with the printed link, and verify `/status` as in step 7.

## Sources

The free model ID was checked against the live catalog on **11 October 2026**. Platform documentation links below explain setup; buttons, available free models, limits, and pricing can change.

- [Discord's official getting-started guide](https://discord.com/developers/docs/quick-start/getting-started) — application creation, **Bot**, **Reset Token**, token safety, and server installation.
- [Member Counter's Developer Portal walkthrough](https://docs.membercounter.app/custom-bot/creating-a-discord-bot-account) — **New Application**, **Create**, **Privileged Gateway Intents**, **Message Content Intent**, **Server Members Intent**, and **Save Changes** labels. Its own bot needs different switches; follow this guide's choices for discord-bot.
- [Discord's official user ID instructions](https://support.discord.com/hc/en-us/articles/206346498-Where-can-I-find-my-User-Server-Message-ID) — **User Settings → Advanced → Developer Mode** and **Copy User ID**.
- [OpenRouter rate limits](https://openrouter.ai/docs/api_reference/limits) — `:free` variants and the published free request limits.
- [OpenRouter live model catalog](https://openrouter.ai/api/v1/models) — catalog verification of the example free model; no running-cost measurement.
- [OpenRouter authentication guide](https://openrouter.ai/docs/api_reference/authentication) — creating keys, naming them, and setting spending limits.
- [OpenRouter key page](https://openrouter.ai/keys) — sign-in and sign-up entry point.
- [Docker Desktop documentation](https://docs.docker.com/desktop/) — Mac/Windows installation links, bundled Docker Compose, and Linux-container support on Windows.
- [Docker Engine installation documentation](https://docs.docker.com/engine/install/ubuntu/#install-using-the-convenience-script) — the `get.docker.com` installer, privilege requirements, and production-use warning.
- [Docker volume documentation](https://docs.docker.com/engine/storage/volumes/#back-up-restore-or-migrate-data-volumes) — persistent storage and using a helper container to make a backup archive.
