# Your first Discord or Telegram bot

This bot keeps conversation notes and uses them alongside a fixed persona in later replies. You do not need to write code. Start the package from Terminal or PowerShell, then connect your accounts and configure the bot in your browser.

Choose **Discord** or **Telegram** for the connection you want. The numbered
Discord walkthrough below retains Discord's intents and invitation steps;
Telegram users follow the short path here, then the same installation/start commands.

## Telegram setup

1. Open the official [@BotFather](https://t.me/botfather) in Telegram and send
   `/newbot`. Choose a name and a username ending in `bot`; save its token
   privately. The token is a password, not your personal Telegram login.
2. Send `/setprivacy` to BotFather, choose the bot and select **Disable** so it
   can observe ordinary group conversation. **Remove and re-add it to existing
   groups** after changing privacy. Alternatively add it as a group
   administrator, which receives ordinary messages regardless of privacy.
3. Add the bot to a group you administer and give it permission to **send
   messages**. Tell members that selected messages/notes may leave the host for
   model processing. Administrator status is also required to receive human
   reaction updates; sending reactions is limited to the chat's allowed emoji.
4. Obtain a model key (step 2 below), install Docker or a release binary, and
   start the package as described in step 5. The panel opens on port **8787**
   before credentials are required. Open the printed setup link and complete
   **Claim and protect**, then visit **Settings** instead of the Discord wizard.
   In **Platform**, save the BotFather token and approved Telegram rooms; also
   configure the provider/model credentials and language. Telegram Settings do
   not run a `getMe` credential test. For terminal-only setup, use
   [Headless CLI setup](#headless-cli-setup), select `telegram` at the first
   prompt and paste the BotFather token when requested. No Discord account or
   token is needed for Telegram alone.
5. Set `OWNER_KEY` in **Settings → Privacy** as
   `telegram:user:<your numeric user ID>`, for example `telegram:user:42`; this
   is not `@username` or the group's ID. You can skip it, in which case Telegram
   administration is denied. Set approved `TELEGRAM_ROOMS` in
   **Settings → Platform** (or answer the CLI prompt) as comma-separated native
   **negative** group IDs or canonical keys, such as
   `-100123,telegram:room:-456`. **Blank denies every room**; merely adding the
   bot does not approve it.
6. Save Settings and **restart the process** to activate the configuration
   (`docker compose restart bot` for Docker, or stop and restart the binary).
   Telegram alone starts without a Discord token; both tokens can be configured
   to run both platforms together. CLI users launch with the same commands
   below. Open a private chat with the bot and press **Start** before requesting
   administrative results, which are delivered privately. In an approved group
   send `/status` (`/durum` for Turkish). Check the privacy/history/nickname
   notices in that result and any limited-visibility warning on Overview.

To find IDs without forwarding conversations to a third-party ID bot, inspect a
Bot API `getUpdates` response privately before starting this runner (`from.id`
for the sender, `chat.id` for the group). Treat the response as private member
data, never publish it, and do not run a second poller while the bot is running.
Keep the minus sign on group IDs. Canonical private-chat RoomKeys can also be
explicitly approved; approval does not automatically grant administrative rights.

Telegram uses outbound HTTPS long polling, so **NAT is fine** and no public
webhook port is needed. An existing webhook or second process using the same
token conflicts with polling. There is **no old group-history fetch**: it learns
future updates and locally retained conversation. Telegram's pending update
queue lasts at most 24 hours, not fourteen days. Personality name selection
cannot change a per-group nickname. Forum topic IDs are retained for reply routing,
but approval is at the **chat** level: approving a group approves its topics;
`TELEGRAM_ROOMS` cannot grant or deny individual topics. Topic creation, closure
and other topic administration are not offered.

Replies are plain text split at 4096 Unicode scalars. Reactions use only allowed
emoji, not paid/custom emoji. Images follow `IMAGE_ANALYSIS=off` (no download);
enabled images use a validated `getFile` download without exposing token-bearing
URLs to the model. Local outgoing files are uploaded directly. For limits,
tokenless offline authorization smoke and the explicitly ignored live `getMe`
probe, see [Telegram configuration](configuration.md#telegram).

Official references: [creating a bot](https://core.telegram.org/bots/features#creating-a-new-bot),
[privacy mode](https://core.telegram.org/bots/features#privacy-mode),
[Bot API polling](https://core.telegram.org/bots/api#getupdates).

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
Tell members which rooms the bot can read. It reads ordinary messages, not only mentions, and observes messages while asleep. Discord's first scan reads up to **14 days** of accessible text history; set `CHANNELS` before first launch. **Blank Discord `GUILD_ID` / `CHANNELS` means every accessible server / channel.** Telegram has no group-history scan and **blank `TELEGRAM_ROOMS` denies all rooms**. Selective member deletion is not supported, and `/reset` does not erase stored records. [Retention details](#privacy).

## What does it cost?

The bot itself is free, open-source software. You bring your own model account. A **model** is the AI that writes the bot's replies.

**You can try a free model.** The live OpenRouter catalog listed `google/gemma-4-31b-it:free` on 11 October 2026. This is a catalog-verified example, not an end-to-end bot compatibility claim. Availability and limits can change. Copy the exact ID into setup's **Model** field. Leaving it blank selects the provider's **paid** default, not a free model.

Free-model requests are rate-limited; consult [OpenRouter's current limits](https://openrouter.ai/docs/api_reference/limits). A **request** is one model call, not one Discord message. Background analysis makes requests too and can exhaust the free allowance before ordinary chat does.

We have not published a measured running cost yet. Replies and background analysis both use your model key. Start with a provider spending limit and check the provider's usage dashboard. A rented server may have its own bill.

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
4. Copy the key and keep it private. You will paste it into browser setup.
5. If you want a paid model, add credits to your OpenRouter account. If you want to start free, choose a current `:free` model instead.

Already using Mistral? You can use your Mistral API key instead. Choose `mistral` in browser setup. The free limits above are for OpenRouter, not Mistral.

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

**Upgrading an existing installation?** Complete [the configuration and volume migration](#existing-compose-installations) before downloading the new launch settings or starting the service. Changing Compose's volume name alone would start with different storage.

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

### Command 3: create the editable configuration folder

```sh
mkdir -p config photos
```

On Linux, give the container's account access to the private configuration folder:

```sh
sudo chown 10001:10001 config
sudo chmod 700 config
```

On Windows PowerShell: `mkdir config` then `mkdir photos`. Docker Desktop manages mount
permissions; Linux ownership commands are not required there.

### Command 4: start, then open browser setup

```sh
docker compose up -d
docker compose logs bot
```

The panel starts before checking the bot's credentials, so a new installation does not need an existing `.env`. Open the printed `http://localhost:8787/setup#token=...` link within **15 minutes**. Treat it as a password: do not share it or the log containing it. The claim token is consumed when claimed, before you save a password. If it expires, or you lose that setup session before saving a password, run `docker compose restart bot` and use the new link. Once the password is saved, use it to log in; restarts no longer print a claim link.

### The five browser steps for Discord

1. **Claim and protect.** Use the one-use link and set an administrator password of at least **12 characters**. Save it in a password manager. The panel administrator password is separate from your platform token and optional bot owner identity.
2. **Connect Discord.** Paste the token from step 1 and test it. Setup checks Discord's API for the actual application identity; the invite is generated from that verified ID, not decoded from the token. A successful test is not yet a gateway connection.
3. **Choose AI.** Select `openrouter` or `mistral`, paste its key, and enter a model ID. To start free on OpenRouter, explicitly choose a current `:free` model; leaving **Model** blank uses a paid default. The key test makes authenticated **GET** requests only, not a paid completion. It does not prove that the model can produce a reply or that you have sufficient credit.
4. **Make it fit.** Choose `en` (English) or `tr` (Turkish), your UTC offset (for example `3` or `5.5`), optional Discord owner ID, and news settings. **Set access before starting:** fill `GUILD_ID` with your server ID and `CHANNELS` with channel IDs, comma-separated for several channels. With Developer Mode enabled, right-click a server or channel to copy its ID. Blank values mean **all accessible servers/channels**, including the first history scan.
5. **Invite and start.** Open **Add to server**, choose your server, and approve Discord's prompts. The invite requests bot and slash-command scopes with permission mask `67226688`, not Administrator access. Confirm **Message Content Intent** and **Server Members Intent** are enabled in the Developer Portal, then finish setup to start the bot.

Credential tests do not save your keys to disk. Finish saves the private `config/.env` and starts the Discord runtime **without restarting the process**. Once its password has been saved, the administrator account survives an interrupted setup; log in to resume, rather than trying to reclaim it. Unfinished credential drafts are session-local, expire 15 minutes after creation or the latest successful credential test, and do not survive logout or process restart.

Keep `config/.env` private: it contains plaintext tokens and keys. Browser saves replace the file atomically and preserve unrelated settings/comments; they do not encrypt it. Never upload it or share it in a screenshot. On Windows restrict folder access in **Properties → Security**. Rotate a leaked Discord token in Developer Portal, a leaked Telegram token with BotFather, and a leaked model key in its provider account.

Keep Docker Desktop running. Panel health is separate from platform connection
health: a successful Discord token test is not proof that privileged intents,
channel permissions or the gateway connection work. Telegram's Settings path
requires a restart and does not offer the Discord wizard's credential test.

For a release binary, run it from its private data folder and open the printed browser link; it writes `.env` there. Keep the terminal open. An existing valid configuration starts its configured platform connections automatically alongside the panel. [Headless CLI setup](#headless-cli-setup) remains available if you do not want browser administration.

### Migrating an existing Docker `.env`

Stop the existing service with its **old** Compose file first: `docker compose stop bot`. Preserve its old volume as described under [Existing Compose installations](#existing-compose-installations). Create `config` and `photos`, then copy your existing configuration before removing any `env_file` injection:

```sh
mkdir -p config photos
cp .env config/.env
sudo chown 10001:10001 config config/.env
sudo chmod 700 config
sudo chmod 600 config/.env
```

On Linux, those ownership/mode commands let container UID `10001` read and update the private directory. On Docker Desktop, omit the Unix ownership commands; on Windows use `Copy-Item .env config/.env` and restrict folder access instead.

Only after the copy, download/update Compose. It must mount **the directory** `./config:/data/config`, set `PANEL_CONFIG_PATH=/data/config/.env`, and no longer use `env_file: .env`; atomic saves require a writable directory, not a single-file mount. Complete any old-volume migration before starting it. The new Compose file uses the fixed volume name **`discord-bot-data`**; an old project-prefixed volume is not automatically reused. Keep the old `.env` private as a migration backup.

If you deliberately retain `env_file` or externally injected environment settings,
the panel shows those fields as externally managed and read-only. Environment values win
over the file; a browser save cannot override Compose injection.

### Panel safety and everyday settings

Open `http://localhost:8787` after setup and log in. **Overview** distinguishes panel availability from platform connections and shows mood per active conversation, not a fabricated global mood. Its model-call/token counters cover only this process, not durable usage history or measured cost. **Settings** validates supported fields without revealing stored keys. Token/key fields are write-only: a blank secret field keeps its saved value, and the explicit clear control removes it. **Settings → Platform** provides Telegram's token and approved rooms alongside Discord's token; blank Telegram approval denies all rooms. Save the model credentials as well when configuring Telegram alone, and restart to apply settings. **Logs** is a bounded recent, redacted in-memory buffer; entries are lost on restart and it is not a permanent audit trail. Redaction is not a guarantee that logs contain no member data.

Memory/personality editors, action buttons, and the dedicated usage page are intentionally disabled pending core APIs. Use existing supported Discord commands for bot operations; do not assume the panel supplies member erasure, durable usage history, or provider cost accounting.

Saving settings shows a restart-required banner. Apply them with
`docker compose restart bot`; a standalone binary must be stopped and started in its
original data folder. The saved `model.md` override has precedence over the configured default; changing **Model** in Settings does not erase that separate override.

### Accessing a remote server

Do not publish port 8787 to the internet. Compose publishes `127.0.0.1:8787:8787`, even though the container binds `0.0.0.0`. On your own computer, keep this SSH tunnel open:

```sh
ssh -N -L 8787:127.0.0.1:8787 your-user@your-server
```

Open `http://localhost:8787` on **your computer**. For first setup, open the server's printed claim link through that tunnel; both ports remain 8787 so the expected origin matches. Do not share the link or tunnel access.

An HTTPS reverse proxy is an advanced alternative: explicitly set `PANEL_PUBLIC_URL` to its HTTPS origin, preserve the matching Host and browser Origin, and restart the service. Forwarded headers are never trusted as authority. Merely forwarding a public HTTP port is not a safe substitute.

### Recovering the administrator account

Forgot the password? Stop the running bot **before** recovery, so two processes do not open the same database:

```sh
docker compose stop bot
docker compose run --rm bot panel reset-password
docker compose up -d
```

Save the random replacement password printed **once**, then log in with it. Recovery retains the existing administrator account and conversation memory while invalidating old sessions. For a binary, stop it, run `./discord-bot panel reset-password` (Windows: `.\discord-bot.exe panel reset-password`) in the same data folder, then start it again.

The administrator password is protected with Argon2id in a reserved table in the **same `durum/hafiza.redb` database** as memory, not a separate panel database. A database error fails closed: it does not open anonymous setup or silently replace memory with an empty database. Keys remain plaintext in the private config file because the bot needs them; the password hash does not protect those keys. Treat state-volume and configuration backups as sensitive. A state backup also includes the administrator account; never delete memory as a password-recovery shortcut.

## 6. Invite it to your server

Use **Add to server** in the browser wizard before clicking Start. If you need the invite again, log in to the panel, return to **Setup**, and test the token again. Gateway/member progress appears on **Overview** after starting; a token test alone does not prove that the intents and channel permissions work.

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

If an intent is off, Discord may reject the connection entirely. Check channel read/write permissions, `config/.env` scope filters (or the binary's `.env`), and that `STEALTH` is off; retry `/status`. If status works but chat does not, check logs for provider errors/exhausted limits and consider the sleep schedule. Ordinary chat is not a deterministic health check.

### Telegram is silent. What should I check?

Check `TELEGRAM_ROOMS` first: blank denies all rooms, and discovery is not
approval. Verify the group's negative ID, the bot's send permission and ordinary
message visibility (`/setprivacy` → Disable plus remove/re-add, or administrator
status). A successful `getMe` response from a separate API probe alone does not
prove a particular group's visibility or permissions; unknown visibility must
not be treated as ordinary message access. Start the bot's private chat before
expecting administrative results, and use your source-qualified `OWNER_KEY`,
not `OWNER_ID`. Check for an existing webhook or another process polling the
same token. If commands work but
chat does not, check provider errors/limits and the sleep schedule; normal chat
is not a deterministic health check.

### Configuration is missing or invalid. What does that mean?

The panel remains available for missing or invalid bot credentials, so log in to finish setup or repair Settings. Configure `DISCORD_TOKEN`, `TELEGRAM_TOKEN`, or both; Telegram alone does not need a Discord token. After saving changes to an existing configuration, restart the bot. Panel availability does not mean either platform connection is healthy.

For Compose, check `config/.env` and the writable directory mount's ownership. Process environment values (including retained `env_file` settings) take precedence and are read-only in the panel; a file save cannot override them. A panel bind/config-file/database failure can prevent the panel itself from opening—check the console rather than assuming every failure is recoverable in the browser.

With `PANEL_ENABLED=off`, startup still requires valid bot/model credentials. For a binary, run `init` in its usual working folder to create `.env`; Compose loads `config/.env` through `PANEL_CONFIG_PATH`. Do not rename `.env` to `.env.txt`. `init` refuses an existing file unless you explicitly use `init --force`; back up your settings privately before replacing them.

### How do I see the logs?

Run `docker compose logs -f` from `mybot`. **Logs** are the bot's running activity and error messages. Press Ctrl+C to stop watching. The authenticated panel's **Logs** page shows a bounded recent buffer, not historical container logs. Do not share claim links, passwords, tokens, API keys, or private conversation text when asking for help.

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

This writes `durum-backup.tar.gz`, including memory and the panel administrator account. Protect it like member data; use a different filename to keep earlier backups. Save `config/.env` separately and privately (or `.env` for a binary). Restore procedures below have not been exercised in a live deployment.

### Restore a backup

Stop the bot, inspect the destination volume, and save a separate copy of its current contents before replacing them. Restore to an **empty** volume; extracting over an existing archive tree can leave stale files.

```sh
docker compose down
docker volume inspect discord-bot-data
docker run --rm -v discord-bot-data:/data -v "$PWD:/backup:ro" ubuntu sh -c 'test -z "$(find /data -mindepth 1 -print -quit)" && tar xzf /backup/durum-backup.tar.gz -C /data && chown -R 10001:10001 /data'
docker compose up -d
```

For PowerShell, use `"${PWD}:/backup:ro"` instead of `"$PWD:/backup:ro"`. The restore assigns the data to the container's UID/GID `10001`, including when the old installation ran as a different user. If the empty-volume check or extraction fails, stop; do not start the new service or bypass the guard. Preserve the current data before choosing a different empty destination. Use `/status` and `/mind` after startup to inspect the result.

### Existing Compose installations

Changing the volume name does **not** migrate data. Before adopting the new Compose file:

1. Stop the service using the **old** Compose file: `docker compose stop bot`.
2. Identify the volume actually mounted at `/data/durum`: find the old container ID with `docker compose ps -a`, then run `docker inspect --format '{{range .Mounts}}{{println .Destination .Name}}{{end}}' CONTAINER_ID` using that ID and read the `/data/durum` line. It is often `mybot_durum`; do not guess from the folder name. If no container or volume name is found, locate the original storage before continuing. Avoid posting full `docker compose config` or `docker inspect` output: it can expose injected credentials. The examples below use `mybot_durum`—replace it with the name you actually found.
3. Copy the existing `.env` into the private configuration directory using [Migrating an existing Docker `.env`](#migrating-an-existing-docker-env). Only then remove `env_file` injection when updating Compose.
4. If the old volume already is `discord-bot-data`, keep using it: back it up, and while the bot is stopped run `docker run --rm -v discord-bot-data:/data ubuntu chown -R 10001:10001 /data` to make it accessible to the new container account. No copy/restore is needed; update Compose's config mounts and start with the same volume. Otherwise inspect and back up the **old** nonempty volume before creating the destination:

```sh
docker volume inspect mybot_durum
docker run --rm -v mybot_durum:/data:ro ubuntu test -s /data/hafiza.redb
docker run --rm -v mybot_durum:/data:ro -v "$PWD:/backup" ubuntu tar czf /backup/durum-backup.tar.gz -C /data .
docker volume create discord-bot-data
```

5. For a project-prefixed old volume, update Compose to the new directory mounts and fixed volume name. Restore the archive into `discord-bot-data` using the [empty-volume restore procedure](#restore-a-backup), then start the new service. If that name already contains data, stop: it may belong to another bot. Do not overwrite it or bypass the empty-volume check.
6. Inspect `/status`, `/mind`, and the panel after startup. **Keep the old volume and backup** until the migrated data is confirmed; no step here deletes the original volume. Never use `docker compose down -v` or `docker volume prune` as part of the upgrade.

For PowerShell use `"${PWD}:/backup"` for the backup mount. These instructions are explicit migration steps, not a claim of a tested live migration.

For a binary or source build, memory lives in the local `durum/` folder. Stop the bot before copying that folder to your backup location.

## Privacy

Tell members before inviting it: Discord's accessible ordinary messages and up to 14 days of first-scan history may be processed, subject to `CHANNELS` and Discord permissions. Telegram processes new updates from explicitly approved `TELEGRAM_ROOMS` and retained local text, not old group history; blank approval denies all rooms. Observation continues while asleep. Replies and background analysis send selected chat text and saved notes to the chosen model service (including OpenRouter's serving provider). Attached images can be sent because image analysis defaults on. Configured Firecrawl receives page URLs and search queries.

Saved data stays on your machine, but processing is **not** local. Channel histories are bounded; derived person/topic notes, summaries and archived raw text can remain after a conversation. **Selective member deletion is not supported.** `/reset` resets channel/chat state, not stored member records or archives. Do not promise members an erasure capability the bot does not have.

For a complete local reset, stop the bot, delete its entire saved-data volume (or `durum/` folder for binary/source installs), and remove every backup and exported log/transcript containing the data. This is destructive and removes all members' data **and the panel administrator account**, not one member's. It does not delete copies already processed/retained by model services or Firecrawl; review their retention policies and deletion channels separately. Keep the bot stopped or remove its permissions to avoid recollection.

## Prefer not to use Docker?

These routes use the same platform token and model key settings described above; Discord also needs the intent settings from steps 1–3.

### Download a release

No Docker or Rust is needed, but a **terminal is required**. Download the matching archive from [GitHub Releases](https://github.com/krxi/discord-bot/releases): Linux x86_64 or arm64, macOS Apple Silicon, or Windows. Extract it into a private folder, open a terminal there, and run:

```sh
./discord-bot
```

On Mac or Linux, if the file is not executable, run `chmod +x discord-bot` first. On Windows PowerShell, use:

```powershell
.\discord-bot.exe
```

Keep this terminal open while the bot runs. Open the printed one-use browser link; complete the five Discord steps above or the [Telegram Settings path](#telegram-setup), then verify `/status` for that platform.

### Build from source

This route is for people who already have Git and Rust installed. From a terminal:

```sh
git clone https://github.com/krxi/discord-bot.git
cd discord-bot
cargo run --release
```

The first build takes longer. Keep the terminal open, use the printed browser link, and restrict access before starting either platform. Follow the five Discord steps or the [Telegram Settings path](#telegram-setup), then verify `/status`.

### Headless CLI setup

Browser setup is the default, but `discord-bot init` (or `cargo run -- init`) remains available for headless installations. It first asks for `discord` (Enter) or `telegram`, then the platform token, provider, API key, model, language, owner, and UTC offset. Telegram asks for a qualified `OWNER_KEY` and approved rooms instead of a Discord owner ID. **Token and key input are hidden when run interactively in a terminal**; piped input is not a hidden terminal prompt. Do not place secrets in shell command arguments or share `.env`.

| Question | What to enter |
| --- | --- |
| Platform | Press Enter for `discord`, or type `telegram`. |
| Bot token | Discord's token from step 1, or Telegram's BotFather token. |
| Provider | Press Enter for `openrouter`, or type `mistral`. |
| API key | Your chosen provider's key from step 2. |
| Model ID | Choose a current `:free` OpenRouter model explicitly if wanted; Enter uses the provider's **paid** default. |
| Language | Press Enter for `en` (English), or type `tr` (Turkish). |
| Owner | Discord: your numeric user ID, or skip. Telegram: `telegram:user:<numeric ID>`, or skip administration. |
| Approved rooms (Telegram only) | Negative group IDs or canonical Telegram RoomKeys, comma-separated; blank denies all rooms. |
| UTC offset hours | Your local time difference from UTC, or Enter for `0`; for example `3` means UTC+3. |

The CLI saves `.env` through the same typed configuration validation and atomic save path as the panel, and refuses an existing file unless `--force` is explicit. Back up any existing file privately before replacing it. Before launching, add `PANEL_ENABLED=off`, set Discord `GUILD_ID`/`CHANNELS` to limit Discord access, or explicitly approve `TELEGRAM_ROOMS`, and keep the file private (mode `600` on Unix). Blank Discord scope means every accessible server/channel; blank Telegram rooms deny all. A blank model uses the provider's paid default. Discord prints an invite link and intent reminders; unlike browser setup, CLI init does not verify the token's application identity with Discord's API. Telegram instead prints BotFather privacy, approved-room and private-command reminders; it does not perform live authentication.

For Docker, first create the private `config`/`photos` directories and complete any existing-volume migration above. Run `docker compose run --rm -it --workdir /data/config bot init` to write `config/.env` as container UID `10001`, then add `PANEL_ENABLED=off` and your access scope to that file before `docker compose up -d`. Do not reintroduce `env_file` if you want the file to remain browser-editable later. To re-enable the panel, remove or change `PANEL_ENABLED=off` and restart.

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
