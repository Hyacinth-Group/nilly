# Set up your bot in Docker and a browser

You do not need to write code. Docker runs the ready-made package; your browser connects the accounts and controls the bot. It can use **Discord, Telegram, Slack or Matrix**, and keeps conversation notes for later replies.

This guide starts with a **new installation**. Already running an older version? Follow [the upgrade instructions](#upgrading-from-v1-to-v2) before replacing your Compose file or starting the new version.

## Start with Docker

### What you need

- A computer or server with internet access that can stay on while the bot runs.
- **Docker with Docker Compose**. On Windows or Mac, install and open [Docker Desktop](https://docs.docker.com/desktop/); wait until Docker is running. Use Linux containers on Windows. On Linux, follow [Docker Engine's installation instructions](https://docs.docker.com/engine/install/) for your distribution, including the Compose plugin.
- A web browser and Terminal (Mac/Linux) or PowerShell (Windows). You will paste a few commands there; no programming is needed.
- Permission to add a bot to your chosen chat platform, and a model-provider account. You can create the tokens and model key while following browser setup.

**Before connecting a room:** tell its members that selected conversation text, saved notes and optionally images go to your chosen AI service. The bot reads ordinary conversation, not just mentions, and retains notes locally. All enabled platforms share those notes. [Privacy and retention](#privacy) explains the limits, including the lack of selective member deletion.

### Download the launch settings

Open a terminal and paste:

```sh
mkdir mybot && cd mybot && curl -O https://raw.githubusercontent.com/krxi/discord-bot/main/docker-compose.yml
```

This creates your private working folder and downloads the file that tells Docker how to run the package. Stay in this folder for the remaining Docker commands.

In Windows PowerShell, use these separate lines (also works in older PowerShell):

```powershell
mkdir mybot
cd mybot
curl.exe -O https://raw.githubusercontent.com/krxi/discord-bot/main/docker-compose.yml
mkdir config
mkdir photos
```

On Mac or Linux, create the folders:

```sh
mkdir -p config photos
```

On **Linux**, also allow the container's account to save private settings:

```sh
sudo chown 10001:10001 config
sudo chmod 700 config
```

Docker Desktop manages mount permissions; these Linux ownership commands are not required on Windows or Mac. On Windows, keep the folder private using **Properties → Security**. If Docker on Linux reports `permission denied`, use `sudo` before `docker`, or follow [Docker's Linux post-installation instructions](https://docs.docker.com/engine/install/linux-postinstall/).

### Start and open the setup link

```sh
docker compose up -d
docker compose logs bot
```

The first start downloads the image. The browser panel starts **before chat credentials are required**; you do not need to create an `.env` file first.

In the logs, find a link like:

```text
http://localhost:8787/setup#token=...
```

Open the **actual printed link**, not this example, in your browser on the same computer. Port **8787** is the local browser panel; it is not a public webhook. Compose exposes it only to your computer. For a rented/remote server, first use [an SSH tunnel](#accessing-a-remote-server).

**The setup token lasts 15 minutes and can be used once.** Treat the full link and the log containing it like a password. Claiming consumes the token before the administrator password is saved. If the link expires, or you lose that session before saving a password, run `docker compose restart bot` and use the newly printed link. After you save a password, restarts no longer print a claim link: sign in at `http://localhost:8787` instead.

## Browser setup

The browser walks you through five steps:

1. **Claim and protect.** Set a private administrator password of at least **12 characters** and save it in a password manager. This password is only for the panel; it is not a chat-platform token, model API key or bot-owner identity.
2. **Choose and connect a platform.** Choose [Discord](#discord-setup), [Telegram](#telegram-setup), [Slack](#slack-socket-mode) or [Matrix](#matrix-plaintext-room-setup). Follow that platform's section below to get its credentials, then use the wizard's test button.
3. **Choose AI.** Select OpenRouter or Mistral, paste your private model key and choose a model. [Model keys and cost](#model-keys-and-cost) explains paid defaults and free-model limits.
4. **Make it fit.** Choose English (`en`) or Turkish (`tr`), your hours from UTC (for example `3`, `-5` or `5.5`), optional bot owner, and the rooms the bot may read. Set restrictions **before** starting the chat connection. The owner key examples are in each platform section; this identity controls bot commands, not panel sign-in.
5. **Review and start.** Finish the platform's invitation/access steps, review the privacy notice and start. First setup saves the configuration and starts the selected connection **without a process restart**. Then [verify the connection](#verify-the-connection).

**A green credential test is not a live-health test.** It checks authentication/identity, not a persistent connection, room permissions, ordinary-message visibility or successful replies. Model-provider tests use authenticated GET requests, not a paid completion; they do not prove model compatibility or sufficient credit. Tests do not save the keys to disk or finish setup.

After saving your administrator password, ownership survives interrupted setup: log in to resume. Unsaved credential drafts expire 15 minutes after creation or the latest successful credential test; logout and process restart discard them.

Docker saves tokens and model keys in **`config/.env`**. They are plaintext because the bot needs to use them. Never upload this file, put it in Git or include it in a screenshot. Browser saves use private file permissions on Unix, but are not encryption. On Windows, restrict access to the folder. Rotate leaked tokens/keys with their issuer.

### Model keys and cost

An **API key** is the password that allows the bot to use your model account. The **model** is the AI that writes replies and analyzes conversation for notes.

For OpenRouter:

1. Sign in at [openrouter.ai/keys](https://openrouter.ai/keys).
2. Create a key, name it and set a spending limit.
3. Copy it privately into setup. Add credits if choosing a paid model.

Already use Mistral? Choose `mistral` and use your Mistral key instead. OpenRouter's free limits do not apply to Mistral.

**A free model must be chosen explicitly.** The existing catalog check on 11 October 2026 listed `google/gemma-4-31b-it:free`; it is a catalog-verified example, not an end-to-end bot compatibility claim. Availability and limits can change: consult [the live catalog](https://openrouter.ai/api/v1/models) and copy the exact model ID. Leaving **Model** blank selects the provider's **paid default**, not a free model.

Free requests have [rate limits](https://openrouter.ai/docs/api_reference/limits). One request is one model call, not one chat message; background analysis also makes requests and can use the allowance before ordinary chat does.

The software is free and open source. **Running cost has not been measured or published.** Replies and background analysis both use your key; a rented server may have its own bill. Start with provider spending controls and watch the provider's billing dashboard, not just the panel's token counters.

## Connect your chat platform

Choose one platform for first setup. It does not require credentials for the others. To add another later, use protected **Settings**, save and restart; do not rerun `init --force`. Connections share one brain, not separate memories.

### Discord setup

You need a Discord account and a server where you can install the bot and set channel permissions.

1. Sign in to the [Discord Developer Portal](https://discord.com/developers/applications), select **New Application**, give it a name and create it.
2. Open **Bot**. Under **Token**, use **Reset Token**, confirm the security prompts and save the token privately.
3. Still on **Bot**, find **Privileged Gateway Intents**. Turn on **Message Content Intent** and **Server Members Intent**. Leave **Presence Intent** off. Save changes if prompted.
4. In browser setup, choose Discord, paste the token and test it. Use **Add to server** to invite it to your server before starting.
5. Give it permission to view the intended channels, read message history and send messages. Choose approved channels/server in setup; use Discord permissions to deny access elsewhere as well.

**Blank Discord `GUILD_ID` / `CHANNELS` means every accessible server / channel.** Its first scan can read up to **14 days** of accessible text history. Restrict access before starting; this differs from the default-deny behavior on the other platforms.

To find IDs, turn on **User Settings → Advanced → Developer Mode** in Discord. Right-click a server, channel or your own name and use the appropriate **Copy … ID** option. IDs are numbers, not names. For optional owner commands, enter **`discord:user:<your user ID>`** as `OWNER_KEY`, for example `discord:user:42`. Legacy `OWNER_ID` still accepts a Discord numeric user ID; the browser wizard uses `OWNER_KEY`.

After starting, check **Overview** for the real connection, then send **`/status`** in an accessible channel (`/durum` for Turkish). Keep `STEALTH` off for slash-command verification. A working status card confirms command handling, not a successful model reply. Then mention the bot and say hello; it chooses when to talk and may be asleep.

### Telegram setup

Telegram is the recommended beginner route. You need a Telegram account and a group you administer.

1. Open the official [@BotFather](https://t.me/botfather), send `/newbot` and choose a name and username ending in `bot`. Save its token privately; it is not your personal Telegram login.
2. Send `/setprivacy` to BotFather, choose the bot and select **Disable** so it can receive ordinary group conversation. **Remove and re-add it to existing groups** after changing privacy. Alternatively, make it a group administrator, which receives ordinary messages regardless of privacy.
3. Add it to your approved group and allow it to **send messages**. Administrator status is also needed to receive human reaction updates.
4. Choose Telegram in browser setup and enter the token. Its `getMe` test reports the authenticated bot identity and warns about privacy that may limit ordinary-message visibility; it does not prove a group's permissions.
5. Explicitly approve the group's ID in **`TELEGRAM_ROOMS`**. **Blank denies every room.** Keep the minus sign on native group IDs, for example `-100123`. Canonical keys such as `telegram:room:-100123` are also accepted. Private chats can be approved explicitly using their canonical keys.
6. For administration, enter **`OWNER_KEY=telegram:user:<your numeric user ID>`**, for example `telegram:user:42`. This is not your `@username`, group ID or bot ID. Skipping it denies Telegram administration.

To find IDs without forwarding conversations to a third-party ID bot, inspect a Bot API `getUpdates` response privately **before** running this bot: `from.id` is the sender; `chat.id` is the group. The response contains private member data. Do not publish it or run a second poller while the bot is active.

Before checking status, open the bot's **private chat** and press **Start**. Then, as the configured owner, send **`/status`** in an approved group (`/durum` for Turkish); administrative results are delivered privately. Approval alone does not grant owner rights.

Telegram uses outbound HTTPS polling; no public webhook port is needed. Another poller or existing webhook using the same token conflicts with it. It cannot fetch old group history: it learns future updates and retained local text; Telegram's pending queue lasts at most 24 hours. Approval is at the **chat** level, including all forum topics, not per topic. See [Telegram configuration](configuration.md#telegram) for files, reactions and other limits.

### Slack Socket Mode

You need a Slack account and permission to install an app in your workspace (or administrator approval). **Socket Mode** makes an outbound WebSocket connection; there is no public webhook, signing secret or incoming port to configure.

#### Create and install the app

1. Open [Slack app management](https://api.slack.com/apps) and select **Create New App → From a manifest**.
2. Choose your workspace, select **YAML** and paste the full contents of the supplied [`slack-app-manifest.yml`](slack-app-manifest.yml). You can change its display name; do not broaden permissions silently.
3. Check **Socket Mode** is enabled. Under **OAuth & Permissions → Bot Token Scopes**, confirm:
   - `app_mentions:read`
   - `channels:history`, `channels:read`
   - `groups:history`, `groups:read`
   - `chat:write`
   - `files:read`, `files:write`
   - `reactions:read`, `reactions:write`
   - `users:read`
4. Under **Event Subscriptions**, confirm the bot events: `app_mention`, `message.channels`, `message.groups`, `member_joined_channel`, `reaction_added`.
5. Under **OAuth & Permissions**, select **Install to Workspace** and approve the scopes. Follow your workspace's administrator approval process if required. Reinstall after changing scopes or event subscriptions.

`channels:*` covers public channels; `groups:*` covers private channels the bot has joined. The manifest does not request direct-message reading, user-token scopes, `chat:write.public` or slash commands, and contains no credentials.

#### Get both tokens and approve channels

1. Copy the installed **Bot User OAuth Token** beginning **`xoxb-`** from **OAuth & Permissions**. This is `SLACK_BOT_TOKEN`.
2. Under **Basic Information → App-Level Tokens**, choose **Generate Token and Scopes**. Add only **`connections:write`**, generate it and copy the **`xapp-`** token. This is `SLACK_APP_TOKEN`. Do not add this scope to bot scopes; the manifest cannot generate this secret.
3. In each agreed public or private channel, use Slack's **`/invite @your-bot-name`**. Installation alone does not join channels; a private channel needs a member's invitation.
4. Copy the channel ID from its details or browser URL. Approve native IDs such as **`C01234567,G01234567`** in **`SLACK_CHANNELS`**, not names like `#general`. **Missing/blank approves no channels.**
5. For the command owner, use **More/… → Copy member ID** on your own profile (`U…`). The workspace ID is the `T…` part of the browser URL `https://app.slack.com/client/T…/C…`. Enter **`OWNER_KEY=slack:user:T01234567:U01234567`** using your real IDs. Skipping this denies Slack administration, including status, but not ordinary chat.

Choose Slack in browser setup, enter **both** tokens from the same app/workspace, approved channel IDs and the correct history tier. The wizard checks `auth.test` and `apps.connections.open` (`connections:write`), including the returned WebSocket URL; it does **not** open a persistent Socket Mode connection or prove channel access. A partial token pair is rejected even if another platform is configured.

For an internal customer-built or Marketplace-approved app, leave **`SLACK_HISTORY_TIER=internal`**. For a commercially distributed non-Marketplace app, choose **`restricted`** before starting. Self-hosting alone does not establish the app's Slack distribution classification. History is bounded and rate-limited, not a complete workspace/thread export. [Configuration details](configuration.md#slack-and-simultaneous-platforms) explain both tiers.

After starting, as the configured owner in an invited, approved channel, send **`!status`** or mention the bot followed by **`status`**. **Do not use `/status`: this bot registers no Slack slash commands.** Results are recipient-only ephemeral messages, with no public or direct-message fallback. An ephemeral result is transient, not durable private storage.

Private Slack channels still feed the shared brain and model service. Attached private files can be downloaded within Slack's permissions; enabled image analysis may send images to the model. Slack has no typing indicator, per-channel bot nickname changes or Discord-style interactive menus. See [capability details](architecture.md#slack-socket-mode-boundary).

### Matrix plaintext room setup

Matrix is **not an encrypted assistant**. Encrypted rooms and attachments are unsupported, with no decryption or plaintext fallback. It uses outbound HTTP(S) sync, so no public incoming port is needed.

#### Create a dedicated account and plaintext room

1. Create a **separate bot account** in a Matrix client such as Element on `matrix.org`, or ask your homeserver administrator. Do not give the bot your personal account credentials.
2. Save its full user ID, such as **`@mybot:matrix.org`**, and homeserver API base URL, usually `https://matrix.org`. A display name is not a user ID.
3. Get an **access token** while signed in as the bot. Client account/help/developer tools may expose it; otherwise browser developer tools can show the `Authorization: Bearer ...` header of a Matrix API request. Copy only the token, not `Bearer `. Treat it like a password. For SSO-only accounts, obtain a token from the signed-in client. Settings and CLI also support password login where the homeserver permits it, but browser credential tests require a token.
4. From your separate **owner** account, create a room with **encryption disabled before creation**. Many clients enable encryption by default. An encrypted room cannot be safely downgraded: create a new plaintext room instead. The homeserver can read plaintext, including private rooms.
5. Tell members about storage and model processing. Explicitly approve the internal room ID in **`MATRIX_ROOMS`**, for example `!roomid:matrix.org` (modern opaque `!…` IDs also work). Find it in room settings/advanced information; a `#room:server` alias is not an ID. Alternatively, invite the bot from the configured owner's account after setup.

**Blank `MATRIX_ROOMS` approves no already-joined rooms.** Only explicit IDs or invitations sent by the configured owner grant access. Invitation approvals persist across restarts. Membership alone is not approval or memory isolation.

Choose Matrix in browser setup. Enter the homeserver, full bot user ID, access token and optional room IDs. Its `whoami` test checks that the token's user matches `MATRIX_USER`; it does not prove room access or successful conversation.

For **`OWNER_KEY`**, enter the qualified, encoded identity of your owner account: **`matrix:user:%40owner%3Amatrix.org`** represents `@owner:matrix.org`. Encode `@` as `%40` and `:` as `%3A`; for other reserved characters use uppercase percent-encoding. **The browser requires the canonical key, not the raw Matrix ID.** The headless CLI instead asks for the full owner ID and encodes it for you. Without an owner, invitations cannot approve new rooms and Matrix administration is denied.

Docker and default release/source builds include Matrix. Source builds need Rust 1.96 or newer; a build made with `--no-default-features` omits Matrix and explicitly rejects configured Matrix settings.

#### Verify as the owner

Keep `STEALTH` off. As the owner, send **`!status`** in an approved plaintext room (`!durum` for Turkish); `!help` / `!yardim` lists commands.

The bot invites you to a separate **plaintext admin room** for private results. **Accept the invitation**, keep it unencrypted with only you and the bot, and run `!status` there. This room is excluded from conversational memory. Extra members or encryption make private delivery fail closed; results are never printed publicly as a fallback. The homeserver can still read these plaintext results.

Use the approved group for room-targeted actions such as `!news` and `!reset`; shared-brain actions such as `!status` also work in the admin room. Keep the entire state volume across restarts: Matrix session files contain secrets and preserve the account's device/sync progress. [Matrix configuration](configuration.md#matrix-plaintext-only) and [state paths](state-files.md#matrix-session-and-sync-state) cover advanced options.

## Verify the connection

After **Review and start**, open **Overview** and inspect the actual platform connection and room readiness. The panel being reachable only proves the panel is running.

| Platform | First command | Where the result appears |
| --- | --- | --- |
| Discord | `/status` (`/durum` in Turkish), with `STEALTH` off | A status card visible to the caller |
| Telegram | Owner sends `/status` in an approved group after pressing **Start** in the bot's private chat | The bot's private chat with the owner |
| Slack | Owner sends `!status` or `@bot status` in an invited, approved channel | A private ephemeral result in that channel; no `/status` |
| Matrix | Owner sends `!status` in an approved plaintext room, accepts the admin-room invitation, then sends it there | The separate owner/bot plaintext admin room |

A command result confirms that command route, not a model completion or every room's permissions. Then try ordinary conversation or a mention. It chooses when to join, may react instead of replying, and sleeps at night. Normal chat is not a deterministic health check. Name selection is gradual: the established stage starts after at least 10 days and 25 finished chats.

## Everyday controls

Open **`http://localhost:8787`** and sign in with your panel password.

- **Overview:** actual connections, rooms and active conversations; mood is per conversation, not one invented global mood.
- **Memory:** browse/search people, topics and monthly events; edit supported records without touching the database file. A conflicting save keeps your draft rather than overwriting someone else's edit. Future learning can update manual edits again.
- **Personality:** read the fixed core and evolving records; edit temperament. The periodic coach can later replace it; this is not model training or a permanent pin.
- **Actions:** review the destination, arguments and privacy/cost warnings before a command. Queued work is not proof of delivery; closing the browser does not cancel a job, and interruption cannot undo a message already accepted or a write already committed. Results are retained for 30 minutes, not replayed after restart.
- **Settings:** update credentials/access restrictions. Blank secret inputs keep saved values; use the explicit clear control to remove one. Add other platforms here, save and restart. Credential-test cards test only the selected platform without saving.
- **Usage:** only calls/tokens actually reported in the current process. Missing values are **Not reported**, not zero. It is not persistent history, measured cost or complete billing; use your provider's dashboard.
- **Logs:** a bounded recent redacted buffer, lost on restart, not a permanent audit trail. Redaction does not guarantee there is no member data.

Saved Settings need a restart; first setup is the exception. Run these from `mybot`:

| What you want | Command |
| --- | --- |
| Watch activity (Ctrl+C stops watching, not the bot) | `docker compose logs -f bot` |
| Apply saved Settings | `docker compose restart bot` |
| Stop without deleting saved data | `docker compose down` |
| Start again | `docker compose up -d` |
| Update the image and restart | `docker compose pull && docker compose up -d` |

On older PowerShell, run the two update commands separately. Back up before upgrading. Keep Docker Desktop running. For a binary, stop and restart from its original data folder.

The saved `model.md` selection takes precedence over the default **Model** in Settings; changing that default does not erase the separate saved override. For technical details, see [configuration](configuration.md), [panel behavior](architecture.md#embedded-panel-and-startup) and [platform capabilities](architecture.md#platform-boundary).

## Troubleshooting

### The setup link expired or setup was interrupted

If no administrator password was saved, restart with `docker compose restart bot` and use the new one-use link. If a password was already saved, log in and resume setup. Credential drafts may need to be entered/tested again. Do not delete the data volume to recover setup.

### The bot is online but silent. What should I check?

Start with **Overview**, then the platform's command in [verification](#verify-the-connection). For Discord, confirm both **Message Content Intent** and **Server Members Intent** in the Developer Portal, channel read/write permissions, approved IDs and `STEALTH=off`; restart after changes. Disabled intents can reject the connection entirely.

For the other platforms:

- **Telegram:** check the group's negative ID in `TELEGRAM_ROOMS`, send permission and privacy mode (disable then remove/re-add, or make it administrator). Confirm your numeric `OWNER_KEY`, private-chat **Start**, and that no webhook/second poller shares its token.
- **Slack:** check both tokens belong to the same installed app/workspace, Socket Mode is on, required scopes/events are installed, the bot is invited, the native channel ID is approved and your owner key includes workspace/member IDs. Use `!status`, not `/status`.
- **Matrix:** check the full IDs/homeserver, token user, room approval, encryption **off**, owner key and accepted plaintext admin-room invitation. Keep `STEALTH` off and only the owner/bot in the admin room.

Credential authentication alone does not prove a room is readable. If commands work but chat does not, check provider errors, exhausted limits and the sleep schedule. Unknown visibility is not proof of full access.

### Configuration is missing or invalid

The panel normally remains available when chat credentials are missing/invalid; finish setup or repair **Settings**, save and restart. A partial Slack token pair is rejected; a build without Matrix explicitly rejects its settings. Other platforms alone do not need a Discord token.

For Docker, check `config/.env` and the writable **directory** mount/permissions. External environment values, including old `env_file` injection, override the file and appear read-only in Settings. A file save cannot override them. [Migrate old configuration](#migrating-an-existing-docker-env) rather than injecting browser-editable fields.

A bind, configuration-file or database failure can prevent the panel itself opening: inspect local logs. With `PANEL_ENABLED=off`, startup requires valid chat/model settings. For binaries, keep `.env` in the usual launch folder, not `.env.txt`; `init` refuses an existing file unless `--force` is explicit. Back up before overwriting.

When asking for help, share only relevant redacted errors: no claim links, passwords, tokens, model keys, session files or private conversation text.

### Accessing a remote server

Do not expose port **8787** to the internet. Compose publishes `127.0.0.1:8787:8787` even though the container listens internally on `0.0.0.0`.

On your own computer, keep this tunnel open, replacing the user/server:

```sh
ssh -N -L 8787:127.0.0.1:8787 your-user@your-server
```

Open `http://localhost:8787` on **your computer**. For first setup, use the server's printed claim link through the tunnel; keeping both ports at 8787 preserves the expected origin. Do not share the link or tunnel access.

An HTTPS reverse proxy is an advanced alternative: explicitly set `PANEL_PUBLIC_URL` to its HTTPS origin, preserve matching Host/browser Origin and restart. Forwarded headers are not trusted as authority. A publicly forwarded HTTP port is not a safe substitute. See [network configuration](configuration.md).

### Recovering the administrator account

Forgot the panel password? Stop the running bot **first** so two processes do not open the same database:

```sh
docker compose stop bot
docker compose run --rm bot panel reset-password
docker compose up -d
```

Save the random replacement password printed **once**, then log in. Recovery preserves the administrator account and memory, and invalidates old sessions. For a binary, stop it, run `./discord-bot panel reset-password` (Windows: `.\discord-bot.exe panel reset-password`) in the same data folder, then restart.

The password's Argon2id hash lives in the **same `durum/hafiza.redb` database** as memory. Database errors fail closed, not into anonymous setup or empty replacement memory. This hash does not encrypt the plaintext keys in your configuration. State backups include the administrator account: **never delete memory as password recovery**.

## Where your memories live

Docker saves memory, archives, platform sessions and panel ownership in the named volume **`discord-bot-data`**, mounted at `/data/durum`. The Compose key `durum` means state/data; the actual name no longer depends on your folder. Stopping/replacing a container or `docker compose down` keeps this volume. **Do not use `docker compose down -v`** unless you intend to delete it.

Multiple installations using this Compose file share that name; use deliberately distinct names for separate bots. An old project-prefixed volume is **not** automatically reused. [Existing Compose installations](#existing-compose-installations) explains migration.

For binaries/source builds, saved state is in the launch folder's **`durum/`** directory, and configuration is `.env`. Stop the bot before copying the entire directory. Matrix session files contain secrets and must also be preserved; backing up only the memory database omits those sessions. [Exact state paths](state-files.md) are the advanced reference.

### Back up the memory

From `mybot`, **run each line separately and wait for it to succeed before the next; do not paste the whole block at once**. Stop the bot and check that the named volume and nonempty database exist before copying. If `docker volume inspect` fails, stop: later `docker run -v` commands could otherwise create an empty volume with the wrong name. If the database check or backup fails, stop and locate your data; do not continue or restart.

```sh
docker compose down
docker volume inspect discord-bot-data
docker run --rm -v discord-bot-data:/data:ro ubuntu test -s /data/hafiza.redb
docker run --rm -v discord-bot-data:/data:ro -v "$PWD:/backup" ubuntu tar czf /backup/durum-backup.tar.gz -C /data .
docker compose up -d
```

On PowerShell, replace the tar command with:

```powershell
docker run --rm -v discord-bot-data:/data:ro -v "${PWD}:/backup" ubuntu tar czf /backup/durum-backup.tar.gz -C /data .
```

`durum-backup.tar.gz` includes memory, archives, platform session files and the panel account. Protect it like credentials and member data. Use a different filename to keep earlier backups; privately back up **`config/.env` separately**. For binaries, privately save `.env` as well as the complete stopped `durum/` folder.

These are backup/restore instructions, not a claim that a live deployment restore was exercised.

### Restore a backup

Stop the bot and separately back up the destination's current contents. **Run each line separately; continue only after success.** Restore to an **empty** volume: extracting over existing files can leave stale archives or session state. The command below deliberately refuses a nonempty destination; choose/create a different empty volume and update Compose if needed, rather than deleting data to bypass it.

```sh
docker compose down
docker volume inspect discord-bot-data
docker run --rm -v discord-bot-data:/data -v "$PWD:/backup:ro" ubuntu sh -c 'test -z "$(find /data -mindepth 1 -print -quit)" && tar xzf /backup/durum-backup.tar.gz -C /data && chown -R 10001:10001 /data'
docker compose up -d
```

For PowerShell, use `"${PWD}:/backup:ro"` instead of `"$PWD:/backup:ro"`. If the empty-volume guard, extraction or ownership change fails, **stop; do not run the final start command**. The ownership change gives the container UID/GID `10001` access to restored data, including backups from older users.

Restore the matching private configuration separately. After startup, use the platform's [status check](#verify-the-connection) and the panel's **Memory** page to inspect the result. A restored database restores the administrator account/password at backup time; use [password recovery](#recovering-the-administrator-account) if needed.

## Upgrading from v1 to v2

There are **two separate migrations**: preserving Docker's volume/configuration is manual; converting the memory database's identities happens automatically.

1. Stop the old version and [back up the full state and private configuration](#back-up-the-memory). Keep the old executable/image version available for rollback.
2. For Docker, complete [existing Compose migration](#existing-compose-installations) before using the new launch settings. For binaries, use the same private launch folder and preserve its `durum/`.
3. Start v2 once. It automatically converts old numeric Discord person/room identities into platform-qualified identities. Before conversion it creates a consistent database snapshot and manifest beside `hafiza.redb`, in a directory named `hafiza.redb.migration-v2-…`, containing **`original.redb`** and **`manifest.json`**.
4. The conversion and schema change are **atomic**: they commit together or leave the active database unchanged. Body text and timestamps are preserved; unknown/malformed records remain and are reported, not invented as people or silently deleted. Conflicts or backup/database errors stop the cutover. Completed migrations are not repeated.
5. Inspect **Overview**, **Memory** and the platform's status command. Retain the old volume, operator backup, migration snapshot and manifest until the data is confirmed.

The automatic database snapshot is not a replacement for a full stopped-state backup: archives and platform session files live outside the database. See [identity migration and rollback](state-files.md#identity-schema-migration-and-rollback) for the schema contract.

**Rollback:** stop v2, restore the saved **v1** snapshot/backup and run the **old version**. Never point an old binary at the converted v2 database. For Docker, preserve the original old volume and Compose configuration as a rollback route; [restore instructions](#restore-a-backup) cover a full operator backup. Rollback loses changes made after that backup. Migration does not erase member data or archives.

For older 1.0 settings, [configuration](configuration.md#upgrading-from-10) also explains preserving Turkish UI, UTC+3 and owner model control.

### Migrating an existing Docker `.env`

With the old service stopped and its storage preserved, copy the old settings **before** replacing Compose or removing `env_file`:

```sh
mkdir -p config photos
cp .env config/.env
```

On Linux, make both the folder and file private and writable by the container account:

```sh
sudo chown 10001:10001 config config/.env
sudo chmod 700 config
sudo chmod 600 config/.env
```

On Docker Desktop, omit Unix ownership commands. On Windows, use `mkdir config`, `mkdir photos`, then `Copy-Item .env config/.env`, and restrict folder access.

The new Compose file must mount the **directory** `./config:/data/config`, set **`PANEL_CONFIG_PATH=/data/config/.env`**, and remove **`env_file: .env`** for browser-editable fields. Atomic browser saves need a writable directory, not a single-file mount. Keep the original `.env` privately as a migration backup.

If you deliberately keep external environment injection, it wins over file values and those fields appear read-only in Settings. Saving in the browser cannot override them.

### Existing Compose installations

Changing a volume name does **not** move memory. Before adopting the new Compose file:

1. Stop the service using the **old** file: `docker compose stop bot`.
2. Find the old container ID with `docker compose ps -a`. Run `docker inspect --format '{{range .Mounts}}{{println .Destination .Name}}{{end}}' CONTAINER_ID` with that ID and read the `/data/durum` line. Do not guess its volume from the folder name. If there is no container/volume name, locate the original storage before continuing. Do not post full `docker inspect` or `docker compose config` output: it may expose credentials.
3. [Copy `.env` to `config/.env`](#migrating-an-existing-docker-env) before removing `env_file` injection.
4. If the old volume is already **`discord-bot-data`**, keep it: back it up, then while stopped run `docker run --rm -v discord-bot-data:/data ubuntu chown -R 10001:10001 /data`. Update the config mounts and start using the same volume; no volume copy is needed.
5. Otherwise, inspect/back up the **actual old volume** before creating the destination. **Run each line separately, continuing only after success.** The example uses `mybot_durum`; replace that with the name you found:

```sh
docker volume inspect mybot_durum
docker run --rm -v mybot_durum:/data:ro ubuntu test -s /data/hafiza.redb
docker run --rm -v mybot_durum:/data:ro -v "$PWD:/backup" ubuntu tar czf /backup/durum-backup.tar.gz -C /data .
docker volume create discord-bot-data
```

If inspection or the database check fails, stop and locate the data. For PowerShell use `"${PWD}:/backup"` for the backup mount.

6. Update/download Compose with the new directory mounts and fixed volume name. Use [the empty-volume restore procedure](#restore-a-backup) to restore into `discord-bot-data`, then start. If that name already contains data, **stop**: it may belong to another bot. Do not overwrite it or bypass the guard.
7. Check the panel's **Memory** page and platform status. **Keep the old volume and backup.** These steps do not delete the original. Never use `docker compose down -v` or `docker volume prune` as part of the upgrade.

This is a migration procedure, not a claim that a live migration was tested.

## Privacy

Before approving rooms, explain these boundaries to members:

- **Read and stored locally:** ordinary messages, bounded channel histories, summaries and person/topic notes. Overflow raw text may remain in `durum/arsiv/`, plus backups. Observation continues while asleep. [Each platform section](#connect-your-chat-platform) describes access/history limits.
- **Sent to the model service:** selected chat text and saved notes for replies **and background analysis**. OpenRouter also routes them to the serving model provider. Self-hosting does not make this processing local.
- **Images:** analysis defaults on. Attached images, including authorized Slack private files and plaintext Matrix media, may be sent to the model. Set **`IMAGE_ANALYSIS=off`** before starting to disable attached-image processing.
- **Optional web service:** configured Firecrawl receives page URLs and search queries.
- **Shared brain:** all enabled platforms/approved rooms share notes and persona. Source-qualified IDs prevent identity collisions, not information transfer between communities. A private channel or separate platform is not isolated memory.
- **No selective erasure:** member-specific deletion is not supported. `/reset` resets chat/channel state, **not** stored member/topic records or archives. Leaving a room or removing permissions does not erase earlier notes. Backups can retain older data.

A **complete local reset** requires stopping the bot, deleting its entire saved-data volume (or `durum/` for binaries/source), and removing every backup/exported log/transcript containing that data. This deletes **all members' data and the panel account**, not one person's. It cannot delete copies retained by model services or Firecrawl; check their policies and deletion channels separately. Keep the bot stopped or remove permissions to avoid collecting data again.

Matrix plaintext, including its admin room, is also readable to the homeserver. Protect the full state, configuration and backups like credentials. See [retention details](state-files.md#retention-and-deletion).

## Prefer not to use Docker?

All routes use the same browser setup and platform credentials. Run from a private folder and keep the launch folder/state unchanged between starts.

### Download a release

No Docker or Rust is needed, but a **terminal is required**. Download the matching archive from [GitHub Releases](https://github.com/krxi/discord-bot/releases): Linux x86_64 or arm64, macOS Apple Silicon, or Windows. Extract it into a private folder, open a terminal there and run:

```sh
./discord-bot
```

On Mac/Linux, use `chmod +x discord-bot` first if needed. On Windows PowerShell:

```powershell
.\discord-bot.exe
```

Keep the terminal open. Open the printed one-use link, follow [browser setup](#browser-setup), then [verify your platform](#verify-the-connection). Configuration is `.env` in this launch folder; state is `durum/`. An existing valid configuration starts its configured connections alongside the panel.

### Build from source

For people with Git and **Rust 1.96 or newer** installed:

```sh
git clone https://github.com/krxi/discord-bot.git
cd discord-bot
cargo run --release
```

Keep the terminal open, follow the printed browser link and [browser setup](#browser-setup). Matrix is enabled by default; `--no-default-features` removes it without disabling Discord, Telegram or Slack. Configuration/state are relative to the launch folder. See [development](development.md) for developer checks and [capabilities](architecture.md#platform-boundary) for adapter details.

### Headless CLI setup

Use this if you cannot or do not want to use browser administration. With a release binary run `discord-bot init` (Windows: `.\discord-bot.exe init`); from source run `cargo run --release -- init`.

The CLI asks for a platform (Enter defaults to Discord), its credentials, provider/key/model, language, owner and UTC offset. Telegram, Slack and Matrix also ask for explicit room approval. Slack requires both tokens; Matrix asks for a token or password and a **raw full owner ID**, which the CLI encodes into `OWNER_KEY`. Discord asks for the legacy numeric owner ID. Interactive secrets are hidden; **piped input is not hidden**.

CLI init saves `.env` through typed validation/atomic save but performs **no live platform authentication**. It refuses an existing file unless `--force` is explicit; back up privately before overwriting. To add another platform, edit the protected configuration or use Settings rather than rerunning `init --force`.

Before launching:

- Add **`PANEL_ENABLED=off`** if you want no browser panel.
- Set Discord `GUILD_ID`/`CHANNELS` and platform permissions before first scan; blanks allow all accessible Discord rooms.
- Approve Telegram/Slack rooms explicitly (blank denies all); Matrix uses explicit room IDs or owner-approved invitations, not every joined room.
- Set a qualified `OWNER_KEY` for non-Discord administration. Use the examples in your platform section; legacy `OWNER_ID` is Discord-only.
- Set the correct Slack history tier, and choose a model explicitly if you want a free one; a blank model uses a paid default.
- Keep `.env` private (Unix mode `600`; restrict Windows access), and keep secrets out of command arguments/history.

For Docker, first prepare the private `config`/`photos` folders as in [Docker setup](#start-with-docker), and complete any existing-volume migration. Then:

```sh
docker compose run --rm -it --workdir /data/config bot init
```

This writes **`config/.env`** as container UID `10001`. Add `PANEL_ENABLED=off` and your access scope there **before** `docker compose up -d`. Do not reintroduce `env_file`. To enable the panel later, remove/change `PANEL_ENABLED=off` and restart. Headless startup requires valid chat/model settings. Verify using [your platform's command](#verify-the-connection).

For advanced settings and optional integration-test recipes, use [configuration](configuration.md), not onboarding credential tests as proof of live health.

## Sources

Platform website labels, free models, limits and prices can change. These links support the instructions; they are not evidence of a live bot deployment or measured cost.

- [Discord getting started](https://discord.com/developers/docs/quick-start/getting-started) and [copying IDs](https://support.discord.com/hc/en-us/articles/206346498-Where-can-I-find-my-User-Server-Message-ID).
- [Telegram bot creation](https://core.telegram.org/bots/features#creating-a-new-bot), [privacy mode](https://core.telegram.org/bots/features#privacy-mode) and [Bot API polling](https://core.telegram.org/bots/api#getupdates).
- [Slack app management](https://api.slack.com/apps); supplied [local app manifest](slack-app-manifest.yml).
- [OpenRouter keys](https://openrouter.ai/keys), [authentication](https://openrouter.ai/docs/api_reference/authentication), [model catalog](https://openrouter.ai/api/v1/models) and [rate limits](https://openrouter.ai/docs/api_reference/limits). The free-model example above is the existing 11 October 2026 catalog check, not a bot compatibility test.
- [Docker Desktop](https://docs.docker.com/desktop/), [Docker Engine installation](https://docs.docker.com/engine/install/) and [volume backup/restore](https://docs.docker.com/engine/storage/volumes/#back-up-restore-or-migrate-data-volumes).
