# Your first bot

This bot keeps conversation notes and uses them alongside a fixed persona in later replies. You do not need to write code, but setup still requires copying commands into Terminal or PowerShell.

The steps below use Discord, the setup wizard's default. For Matrix, follow
[Matrix plaintext room setup](#matrix-plaintext-room-setup) instead of the Discord
account/intents/invite steps. Both use the same model account and saved brain.

## What you need

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
> Tell members which channels the bot can read. It reads ordinary messages, not only mentions; its first scan reads up to **14 days** of accessible text history, and it observes messages while asleep. Set `CHANNELS` before first launch. **Blank `GUILD_ID` / `CHANNELS` means every accessible server / channel.** Selective member deletion is not supported, and `/reset` does not erase stored records. [Retention details](#privacy).

## What does it cost?

The bot itself is free, open-source software. You bring your own model account. A **model** is the AI that writes the bot's replies.

**You can try a free model.** The live OpenRouter catalog listed `google/gemma-4-31b-it:free` on 11 October 2026. This is a catalog-verified example, not an end-to-end bot compatibility claim. Availability and limits can change. Copy the exact ID into the wizard's model prompt. Enter alone selects the provider's **paid** default, not a free model.

Free-model requests are rate-limited; consult [OpenRouter's current limits](https://openrouter.ai/docs/api_reference/limits). A **request** is one model call, not one Discord message. Background analysis makes requests too and can exhaust the free allowance before ordinary chat does.

We have not published a measured running cost yet. Replies and background analysis both use your model key. Start with a provider spending limit and check the provider's usage dashboard. A rented server may have its own bill.

## Matrix plaintext room setup

Matrix is an alternative chat transport, **not an encrypted assistant**.
End-to-end encrypted rooms and encrypted attachments are unsupported. This bot
uses outbound HTTP(S) sync: a home computer behind NAT can run it without port
forwarding, a tunnel, or a public webhook. Keep the computer on and connected.

### 1. Create a separate bot account

1. In a Matrix client such as Element, create a **dedicated bot account** on
   `matrix.org`, or ask your own homeserver administrator to create one. The bot
   acts as this account; do not give it your personal account's credentials.
2. Save its **full user ID**, such as `@mybot:matrix.org`. A display name or just
   `mybot` is not a user ID. Your homeserver URL is usually `https://matrix.org`
   for that server; on your own server use its Matrix client API base URL.
3. Prefer an **access token**. While signed in as the bot, use the client's
   account/help/developer tools to reveal its access token. Client versions
   differ; if it is not exposed, browser developer tools can show the
   `Authorization: Bearer ...` header on a Matrix client API request. Copy only
   the token, never the `Bearer ` prefix. Treat it as the account's password:
   do not paste it in chat, screenshots, issue reports or source control.
4. Password login is also supported: choose `password` in the wizard if your
   homeserver supports password login. For SSO-only accounts, obtain an access
   token from a signed-in client instead. Revoke a leaked token/session in your
   Matrix client's session settings; change a leaked password too.

Keep your **owner** account separate. Find its full user ID, for example
`@owner:matrix.org`; the wizard needs this to approve invitations and commands.

### 2. Make an explicitly approved, unencrypted room

1. As the owner, create a room and **disable encryption before creation**. Many
   clients enable encryption for new rooms or direct messages by default. Check
   the room's security settings before inviting the bot.
2. An already encrypted room cannot safely be turned into a plaintext room.
   Create a new room if encryption is on. The bot cannot decrypt messages or
   use a key-backup/recovery phrase.
3. Tell members what the bot reads and stores, and that selected conversation
   text and saved notes are sent to your model service. It observes ordinary
   messages in approved rooms, not only mentions. The homeserver can also read
   plaintext content.
4. Invite the bot **from the configured owner's account**. Alternatively,
   explicitly allowlist the room's internal ID in `MATRIX_ROOMS`. Find it in
   the client's room settings/advanced information: it starts with `!`, for
   example `!roomid:matrix.org`. A room alias (`#myroom:matrix.org`) is not an ID.

`MATRIX_ROOMS` accepts native room IDs separated by commas. It is an explicit
allowlist, **not** a list of rooms to search for automatically. A blank value
allows owner-approved invitations only, not every room the account already
joined. Previously joined but unapproved rooms are ignored. Owner invitation
approvals persist across restarts.

**Shared-brain privacy:** approved rooms use the same memories and persona.
Information learned in one approved room can influence replies in another.
Do not approve unrelated private communities expecting isolated memory. A room
invitation, leaving a room, or resetting a chat does not erase stored notes.
See [privacy and retention](#privacy).

### 3. Configure and launch a Matrix-enabled build

Default release builds and the Docker image include the `matrix` Cargo feature.
For source builds the explicit commands below also work. The wizard cannot add
Matrix support to a binary built with `--no-default-features`:
```sh
cargo run --release --features matrix -- init
cargo run --release --features matrix
```

Run both from the repository folder; saved configuration and state are relative
to the launch directory. Get a model API key as in [step 2 below](#2-get-an-openrouter-key).
The wizard asks:

| Question | What to enter |
| --- | --- |
| Chat platform | Type `matrix`. Enter keeps Discord. |
| Homeserver URL | `https://matrix.org`, or your own homeserver base URL. |
| Bot user ID | Full dedicated account ID, e.g. `@mybot:matrix.org`. |
| Login method | Enter for preferred `token`, or type `password`. |
| Token/password | The dedicated bot's credential; terminal input is hidden. |
| Approved room IDs | Optional comma-separated native `!id:server` or modern opaque `!id` values; Enter uses owner invitations only. |
| Provider, API key, model, language | Your model account and preferred language; Enter selects a **paid** default model. |
| Owner user ID | Your full Matrix ID; the wizard writes the percent-encoded `OWNER_KEY`. |
| UTC offset | Your hours from UTC, or Enter for `0`. |

For a Matrix-only installation, leave `DISCORD_TOKEN` blank if editing an
existing `.env`. Protect `.env` and the entire state directory. Matrix session
files contain secrets; never share them as troubleshooting attachments.
Keep the state directory between restarts so the bot restores the same session
and device instead of logging in as a fresh device each time.
[Exact state paths](state-files.md#matrix-session-and-sync-state).

### 4. Verify as the owner

With `STEALTH` off, send **`!status`** in an approved plaintext room (`!durum` for
Turkish UI). `!help` / `!yardim` lists the available commands. Matrix commands
are **owner-only**; a different member's command is not permission to administer
the shared brain.

Matrix has no Discord-style ephemeral reply. Commands deliver their private
results in a separate **plaintext admin room** containing only the bot and owner.
When the bot invites you to that room, **accept the invitation** and run
`!status` there to see the result. Keep it unencrypted and do not invite other
people. This admin room is excluded from conversational memory.

Private results, including mind/settings detail, are denied in public rooms
rather than printed to everyone. If the private room becomes encrypted or has
other members, delivery fails closed: there is no public fallback. Plaintext
admin rooms are not end-to-end confidential; the homeserver can read them too.
Use the approved group for room-targeted actions such as `!news` and `!reset`;
shared-brain commands such as `!status` also work in the admin room.
Do not enable public diagnostics expecting them to be private.

If no reply appears, check that the build has Matrix enabled, the full account
IDs/homeserver are correct, the owner invited the bot or the room is allowlisted,
the room is not encrypted, `STEALTH` is off, and you accepted the bot's private
admin-room invitation. Then inspect local logs without sharing credentials.
Ordinary chat still depends on the sleep schedule and the
bot's willingness to reply; `!status` is the setup check.

For developers using a disposable local homeserver, the ignored integration
test command and `MATRIX_TEST_*` credentials are documented in
[configuration](configuration.md#disposable-matrix-integration-test).


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
| Chat platform | Press Enter for `discord`; Matrix uses the separate setup above. |
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

Tell members before inviting it: accessible ordinary messages and up to 14 days of first-scan history may be processed, subject to `CHANNELS` and Discord permissions. Observation continues while asleep. Replies and background analysis send selected chat text and saved notes to the chosen model service (including OpenRouter's serving provider). Attached images can be sent because image analysis defaults on. Configured Firecrawl receives page URLs and search queries.

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
