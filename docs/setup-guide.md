# Your first bot: Discord, Telegram, Slack or Matrix

This bot keeps conversation notes and uses them alongside a fixed persona in later replies. You do not need to write code. Start the package from Terminal or PowerShell, then connect your accounts and configure the bot in your browser.

Choose **Discord**, **Telegram**, **Slack** or **Matrix** for the connection you
want. The numbered Discord walkthrough below retains Discord's intents and
invitation steps; other users follow their platform sections, then the same
installation/start commands. The five-step browser wizard remains Discord-only;
other platforms use the CLI or protected Settings followed by a process restart.

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

For Slack, use [Slack Socket Mode](#slack-socket-mode); for Matrix, use
[Matrix plaintext room setup](#matrix-plaintext-room-setup). Neither requires a
Discord account or token. Enabled platform connections share one brain, so
approve only rooms whose members understand that their information may influence
replies elsewhere.

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
Tell members which rooms the bot can read. It reads ordinary messages, not only mentions, and observes messages while asleep. Discord's first scan reads up to **14 days** of accessible text history; set `CHANNELS` before first launch. **Blank Discord `GUILD_ID` / `CHANNELS` means every accessible server / channel.** Telegram has no group-history scan and **blank `TELEGRAM_ROOMS` denies all rooms**. Slack requires explicit `SLACK_CHANNELS`; **blank denies all Slack rooms**, and history is bounded/rate-limited. Matrix requires explicit room IDs or owner-approved invitations, ignores unapproved joined rooms, and never decrypts encrypted content. Selective member deletion is not supported, and `/reset` does not erase stored records. [Retention details](#privacy).

## What does it cost?

The bot itself is free, open-source software. You bring your own model account. A **model** is the AI that writes the bot's replies.

**You can try a free model.** The live OpenRouter catalog listed `google/gemma-4-31b-it:free` on 11 October 2026. This is a catalog-verified example, not an end-to-end bot compatibility claim. Availability and limits can change. Copy the exact ID into setup's **Model** field. Leaving it blank selects the provider's **paid** default, not a free model.

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

### 4. Configure and start

Install Docker using [step 4 below](#4-install-docker), then follow the download/start commands in [step 5](#5-download-and-start-the-bot). The panel opens on port **8787** before platform credentials are required. Complete **Claim and protect**, then open protected **Settings**, not **Connect Discord**. In **Platform**, save both Slack tokens, native approved channel IDs and the history tier; configure provider/model credentials and language, and set the qualified owner in **Privacy**. Save and **restart the process** (`docker compose restart bot` for Docker). Slack Settings do not perform a live Slack credential test or start a connection without restarting.

For terminal-only setup, follow [Headless CLI setup](#headless-cli-setup), including its Docker command that writes `config/.env`. With a downloaded release binary, run:

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

To run several platforms simultaneously, add `DISCORD_TOKEN` and/or `TELEGRAM_TOKEN` with their corresponding room restrictions to the same configuration; retain both Slack tokens and `SLACK_CHANNELS`. The wizard configures one platform at a time. Use protected Settings or edit the private file rather than rerunning `init --force` to add a platform, since init overwrites the answers it collects. Each platform keeps its own room permissions and approval settings. A partial Slack app/bot token pair is rejected even if Discord or Telegram credentials are complete.

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

The native `matrix-sdk` **0.19.1** integration is an optional dependency enabled
by the default `matrix` Cargo feature. Source builds require **Rust 1.96**.
Default release builds and the Docker image include Matrix; a binary built with
`--no-default-features` does not offer Matrix in CLI init and rejects configured
Matrix settings explicitly rather than silently ignoring them.

Install and start using [step 5 below](#5-download-and-start-the-bot). The panel
opens on port **8787** before platform credential validation. Complete **Claim
and protect**, then open protected **Settings**, not the Discord-only browser
wizard. In **Platform**, save the homeserver URL, full bot user ID, access token
or password, and optional room IDs. Configure provider/model credentials and
language, and set `OWNER_KEY` in **Privacy**, for example
`matrix:user:%40owner%3Amatrix.org`. Save and **restart the process**
(`docker compose restart bot` for Docker). Matrix Settings do not run a live
homeserver credential test or activate a connection without restarting.

For terminal-only setup, follow [Headless CLI setup](#headless-cli-setup);
its Docker command writes `config/.env`. With a release binary run
`discord-bot init`. For source builds:

```sh
cargo run --release --features matrix -- init
cargo run --release --features matrix
```

Run source commands from the repository folder; saved configuration and state
are relative to the launch directory. Get a model API key as in
[step 2 below](#2-get-an-openrouter-key). The CLI wizard asks:

| Question | What to enter |
| --- | --- |
| Chat platform | Type `matrix`. Enter keeps Discord. |
| Homeserver URL | HTTP(S) base URL, e.g. `https://matrix.org`; no embedded credentials, query or fragment. |
| Bot user ID | Full dedicated account ID, e.g. `@mybot:matrix.org`. |
| Login method | Enter for preferred `token`, or type `password`. |
| Token/password | The dedicated bot's credential; interactive terminal input uses hidden `rpassword` entry, while piped input is not hidden. |
| Approved room IDs | Optional comma-separated native `!id:server` or modern opaque `!id` values; Enter uses owner invitations only. |
| Provider, API key, model, language | Your model account and preferred language; Enter selects a **paid** default model. |
| Owner user ID | Your full Matrix ID; the CLI validates it and writes the percent-encoded qualified `OWNER_KEY`. |
| UTC offset | Your hours from UTC, or Enter for `0`. |

The CLI uses typed config encoding and an atomic patch/save, not raw secret
interpolation. It does not contact the homeserver to verify credentials.
Matrix-only output does not require or write `DISCORD_TOKEN`; when editing an
existing `.env`, leave that token blank if Discord is not wanted. Use protected
Settings to add other platforms rather than rerunning init.

Protect `.env` (Compose: **`config/.env`**) and the entire state directory.
Matrix session files contain secrets; never share them as troubleshooting
attachments. Keep the **`discord-bot-data`** volume between Docker restarts so
the bot restores the same session and device instead of logging in as a fresh
device each time. There is no crypto store, decryption, or plaintext fallback
for encrypted rooms or attachments.
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

Keep `config/.env` private: it contains plaintext tokens and keys. Browser saves replace the file atomically and preserve unrelated settings/comments; they do not encrypt it. Never upload it or share it in a screenshot. On Windows restrict folder access in **Properties → Security**. Rotate a leaked Discord token in Developer Portal, a leaked Telegram token with BotFather, leaked Slack tokens in Slack app management, a leaked Matrix token/session in the client (and change leaked passwords), and a leaked model key in its provider account.

Keep Docker Desktop running. Panel health is separate from platform connection
health: a successful Discord token test is not proof that privileged intents,
channel permissions or the gateway connection work. Telegram, Slack and Matrix's
protected Settings paths require a restart and do not offer the Discord wizard's
credential test.

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

Open `http://localhost:8787` after setup and log in. **Overview** distinguishes panel availability from platform connections and shows mood per active conversation, not a fabricated global mood. Its model-call/token counters cover only this process, not durable usage history or measured cost. **Settings** validates supported fields without revealing stored keys. Token/key/password fields are write-only: a blank secret field keeps its saved value, and the explicit clear control removes it. **Settings → Platform** provides Telegram's token and approved rooms, Slack's app/bot tokens, native channel allowlist and history tier, Matrix's homeserver/user/credentials and optional room IDs, and Discord's token. Blank Telegram/Slack approval denies all rooms; Slack requires both tokens. Matrix blank rooms permits only owner-approved invitations, not all joined rooms. Save model credentials as well for any non-Discord-only installation, and restart to apply settings. **Logs** is a bounded recent, redacted in-memory buffer; entries are lost on restart and it is not a permanent audit trail. Redaction is not a guarantee that logs contain no member data.

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

The panel remains available for missing or invalid bot credentials, so log in to finish setup or repair protected Settings. Configure `DISCORD_TOKEN`, `TELEGRAM_TOKEN`, a complete `SLACK_APP_TOKEN` + `SLACK_BOT_TOKEN` pair, Matrix's homeserver/user plus access token or password, or several platforms. Non-Discord platforms alone need no Discord token. A partial Slack pair is rejected even with another platform configured; a build without the Matrix feature rejects configured Matrix settings explicitly. After saving changes to an existing configuration, restart the bot. Panel availability does not mean any platform connection is healthy.

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

Tell members before inviting it: Discord's accessible ordinary messages and up to 14 days of first-scan history may be processed, subject to `CHANNELS` and Discord permissions. Telegram processes new updates from explicitly approved `TELEGRAM_ROOMS` and retained local text, not old group history; blank approval denies all rooms. Slack uses an explicit `SLACK_CHANNELS` allowlist (blank denies all), membership permissions and bounded rate-limited history. Matrix processes plaintext rooms approved by explicit IDs or owner invitations; unapproved joined rooms and encrypted content are not readable routes. Observation continues while asleep. Replies and background analysis send selected chat text and saved notes to the chosen model service (including OpenRouter's serving provider). Attached images, including authorized Slack private-file images and plaintext Matrix media, can be sent because image analysis defaults on; `IMAGE_ANALYSIS=off` disables this. Configured Firecrawl receives page URLs and search queries. Enabled platforms share memory, so approve only rooms whose members trust that cross-platform information boundary.

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

Keep this terminal open while the bot runs. Open the printed one-use browser link; complete the five Discord steps above or the protected Settings path for [Telegram](#telegram-setup), [Slack](#4-configure-and-start) or [Matrix](#3-configure-and-launch-a-matrix-enabled-build). Verify `/status` for Discord/Telegram or owner-only `!status` in an approved Slack/Matrix room, following each platform's private-result instructions.

### Build from source

This route is for people who already have Git and **Rust 1.96 or newer** installed. Matrix is enabled by default; `--no-default-features` removes Matrix without disabling Discord, Telegram or Slack. From a terminal:

```sh
git clone https://github.com/krxi/discord-bot.git
cd discord-bot
cargo run --release
```

The first build takes longer. Keep the terminal open, use the printed browser link, and restrict access before starting any platform. Follow the five Discord steps or the protected Settings path for [Telegram](#telegram-setup), [Slack](#4-configure-and-start) or [Matrix](#3-configure-and-launch-a-matrix-enabled-build). Verify `/status` for Discord/Telegram or owner-only `!status` in an approved Slack/Matrix room, following each platform's private-result instructions.

### Headless CLI setup

Browser setup is the default, but `discord-bot init` (or `cargo run -- init`) remains available for headless installations. It first asks for `discord` (Enter), `telegram`, `slack` or feature-enabled `matrix`, then platform credentials, provider, API key, model, language, owner, and UTC offset. Telegram asks for a qualified `OWNER_KEY` and approved rooms instead of a Discord owner ID. Slack asks for both `xapp-` and `xoxb-` tokens, native channel IDs and a workspace-qualified `OWNER_KEY`. Matrix asks for an HTTP(S) homeserver, full bot user ID, token or password, optional native room IDs and a full owner ID, which it encodes into a qualified key. **Token, password and key input use hidden `rpassword` prompts when run interactively in a terminal**; piped input is not hidden. Do not place secrets in shell command arguments or share `.env`.

| Question | What to enter |
| --- | --- |
| Platform | Press Enter for `discord`, or type `telegram`, `slack` or feature-enabled `matrix`. |
| Platform credentials | Discord's token, Telegram's BotFather token, Slack's `xapp-` then `xoxb-` tokens, or Matrix's homeserver/full user ID plus access token or password. |
| Approved channels/rooms (Slack/Matrix, before provider) | Slack: native `C…` / `G…` IDs; blank denies all. Matrix: native `!…` room IDs; blank allows owner-approved invitations only. |
| Provider | Press Enter for `openrouter`, or type `mistral`. |
| API key | Your chosen provider's key from step 2. |
| Model ID | Choose a current `:free` OpenRouter model explicitly if wanted; Enter uses the provider's **paid** default. |
| Language | Press Enter for `en` (English), or type `tr` (Turkish). |
| Owner | Discord: optional numeric user ID. Telegram: optional `telegram:user:<numeric ID>`. Slack: optional `slack:user:<workspace ID>:<member ID>`. Skipping denies that platform's administration. Matrix: full `@owner:server` ID; CLI writes its qualified key. |
| Approved rooms (Telegram only) | Negative group IDs or canonical Telegram RoomKeys, comma-separated; blank denies all rooms. |
| UTC offset hours | Your local time difference from UTC, or Enter for `0`; for example `3` means UTC+3. |

The CLI saves `.env` through the same typed configuration validation and atomic patch/save path as the panel, and refuses an existing file unless `--force` is explicit. Back up any existing file privately before overwriting collected settings. Before launching, add `PANEL_ENABLED=off`, set Discord `GUILD_ID`/`CHANNELS` to limit Discord access, explicitly approve Telegram/Slack rooms or Matrix room IDs/owner invitations, and keep the file private (mode `600` on Unix). Blank Discord scope means every accessible server/channel; blank Telegram/Slack rooms deny all; blank Matrix rooms uses owner invitations only. Set `SLACK_HISTORY_TIER=restricted` for commercial non-Marketplace Slack apps; the default is `internal`. A blank model uses the provider's paid default. Discord prints an invite link and intent reminders; unlike browser setup, CLI init does not verify the token's application identity with Discord's API. Telegram prints BotFather privacy, approved-room and private-command reminders; Slack prints Socket Mode, native-channel approval and owner-only status reminders; Matrix prints plaintext-room approval and private admin-room guidance. CLI init performs no live platform authentication.

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
