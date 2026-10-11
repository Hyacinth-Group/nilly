# Your first Discord bot

This bot grows its own personality. It remembers people, forms opinions, has moods, and eventually chooses its own name. You do not need to write any code.

## What you need

- [ ] A Discord account.
- [ ] A Discord server where you are an administrator.
- [ ] A computer or a small server with internet access.
- [ ] About 10 minutes, plus any installation or download time.

Keep the computer on for the bot to stay online.

## What does it cost?

The bot itself is free, open-source software. You bring your own model account. A **model** is the AI that writes the bot's replies.

**You can try a free model.** OpenRouter currently lists models whose IDs end in `:free`, such as `liquid/lfm-2.5-2.6b:free`. Availability can change. Copy a current model ID from [OpenRouter's model list](https://openrouter.ai/models) and enter it during setup. Pressing Enter for the default model does **not** select a free model.

According to [OpenRouter's limits documentation](https://openrouter.ai/docs/api_reference/limits), free variants have these limits:

| All-time credits purchased | Requests per minute | Requests per day |
| --- | --- | --- |
| Less than 10 credits | 20 | 50 |
| At least 10 credits | 20 | 1,000 |

A **request** is one call to the AI service, not one Discord message. The bot's background memory work makes requests too. Free limits can run out quickly in a busy server. Buying credits raises the daily allowance; the free variant itself still has no model charge.

**Paid models charge for usage.** Light use with an inexpensive model can cost cents, but there is no fixed daily price. For a concrete example, OpenRouter currently lists `openai/gpt-4o-mini` at $0.15 per million input tokens and $0.60 per million output tokens. **Tokens** are small pieces of text. At those prices, 100 requests containing 1,000 input tokens and 1,000 output tokens each would cost $0.075, or 7.5 US cents, for model usage. This is an illustration, not a prediction of this bot's bill. Long conversations, background work, and more expensive models cost more.

Set a spending limit on your OpenRouter key. Check usage in your provider account. A rented server may have its own bill.

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
| Discord bot token | The token from step 1. |
| Provider | Press Enter for `openrouter`, or type `mistral`. |
| API key | Your chosen provider's key from step 2. |
| Model ID | Paste a model ID, including `:free` if you want a free OpenRouter model. Enter alone uses the provider's default. |
| Language | Press Enter for `en` (English), or type `tr` (Turkish). |
| Your Discord user ID | Paste the number from step 3, or press Enter to skip. |
| UTC offset hours | Your local time difference from UTC, or Enter for `0`. For example, `3` means UTC+3. This sets the bot's day and night. |

The wizard saves a file called `.env` in `mybot`. It contains your settings and secret keys. It also prints an invite link and a reminder about the two intent switches.

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

## 7. Say hi

In a channel the bot can access, mention it and say hello: `@My Bot hi!` Choose the bot from Discord's mention list.

Give it a moment to respond. It can get to know the server's recent conversation history when it first joins. Over time it remembers people, develops opinions, and picks its own name and nickname.

It is not a machine that answers every message. It chooses when to join ordinary conversations, and sometimes reacts instead of writing a reply. It also sleeps at night. Mentioning it is a good way to start talking.

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

If an intent is off, Discord may also reject the connection entirely. If the switches are already on, mention the bot in a channel it can read and write in. Check the logs for provider errors or exhausted free limits. Remember that it does not answer every ordinary message.

### It says “DISCORD_TOKEN is missing”. What does that mean?

The bot could not find its password. Open your `mybot` folder and check that the wizard created `.env` there. Run the setup command again from that folder and enter your token. If asked about replacing an existing `.env`, only do so if you want to replace its settings. Then run `docker compose up -d` again.

For a downloaded binary or a source build, run `init` and start the bot from the same folder. Do not rename `.env` to `.env.txt`.

### Windows says `$(id -u)` does not work. What should I paste?

That part of the Mac/Linux command finds your computer's user number. PowerShell does not use it. Run this instead, from `mybot`:

```powershell
docker run --rm -it -v "${PWD}:/data" ghcr.io/krxi/discord-bot:latest init
```

### How do I see the logs?

Run `docker compose logs -f` from `mybot`. **Logs** are the bot's running activity and error messages. Press Ctrl+C to stop watching. Do not share tokens, API keys, or private conversation text when asking for help.

## Where your memories live

Docker keeps memory in a **volume** called `durum`. A volume is storage that survives stopping or replacing the bot. With the folder name used above, Docker normally calls it `mybot_durum`. You can list volume names with `docker volume ls`.

The bot never deletes memories automatically. Older material is summarized and archived, not discarded. Stopping it with `docker compose down` keeps that storage. **Do not use `docker compose down -v`** unless you mean to delete its saved memory.

### Back up the memory

From `mybot`, stop the bot, copy the volume into a backup file, and start it again:

```sh
docker compose down
docker run --rm -v mybot_durum:/data:ro -v "$PWD:/backup" ubuntu tar czf /backup/durum-backup.tar.gz -C /data .
docker compose up -d
```

On Windows PowerShell, replace the middle command with:

```powershell
docker run --rm -v mybot_durum:/data:ro -v "${PWD}:/backup" ubuntu tar czf /backup/durum-backup.tar.gz -C /data .
```

If you chose a different folder or project name, replace `mybot_durum` with the existing volume name shown by `docker volume ls`. Do not guess: a wrong name can create an empty volume instead of backing up your memory.

This makes `durum-backup.tar.gz` in your folder. The first backup may download the Ubuntu helper package. Copy the backup somewhere safe. Give each backup a different filename if you want to keep older copies. Also save `.env` securely so you can recover your settings; it contains secrets.

For a binary or source build, memory lives in the local `durum/` folder. Stop the bot before copying that folder to your backup location.

## Privacy

Tell your server members what the bot does before inviting it. Messages it can read, and relevant conversation history and memory, are sent to your chosen model provider for replies and background analysis. With OpenRouter, that includes the model provider serving the request. Attached images may also be sent when image analysis is enabled.

The bot's saved memory stays on your machine, in its memory storage. This does **not** mean all processing stays on your machine. Check your provider's privacy terms before using it with sensitive conversations.

## Prefer not to use Docker?

These routes use the same token, key, and intent settings from steps 1–3.

### Download a release

Download the matching archive from [GitHub Releases](https://github.com/krxi/discord-bot/releases): Linux x86_64 or arm64, macOS Apple Silicon, or Windows. Extract it into a folder, open a terminal in that folder, and run:

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

The first build takes longer. Keep the terminal open, invite the bot with the printed link, and say hi.

## Sources

External setup details and prices were checked on **11 October 2026**. Buttons, available free models, and limits can change.

- [Discord's official getting-started guide](https://discord.com/developers/docs/quick-start/getting-started) — application creation, **Bot**, **Reset Token**, token safety, and server installation.
- [Member Counter's Developer Portal walkthrough](https://docs.membercounter.app/custom-bot/creating-a-discord-bot-account) — **New Application**, **Create**, **Privileged Gateway Intents**, **Message Content Intent**, **Server Members Intent**, and **Save Changes** labels. Its own bot needs different switches; follow this guide's choices for discord-bot.
- [Discord's official user ID instructions](https://support.discord.com/hc/en-us/articles/206346498-Where-can-I-find-my-User-Server-Message-ID) — **User Settings → Advanced → Developer Mode** and **Copy User ID**.
- [OpenRouter rate limits](https://openrouter.ai/docs/api_reference/limits) — `:free` variants and the published free request limits.
- [OpenRouter live model catalog](https://openrouter.ai/api/v1/models) — currently available `:free` models and `openai/gpt-4o-mini` input/output prices used in the cost example.
- [OpenRouter authentication guide](https://openrouter.ai/docs/api_reference/authentication) — creating keys, naming them, and setting spending limits.
- [OpenRouter key page](https://openrouter.ai/keys) — sign-in and sign-up entry point.
- [Docker Desktop documentation](https://docs.docker.com/desktop/) — Mac/Windows installation links, bundled Docker Compose, and Linux-container support on Windows.
- [Docker Engine installation documentation](https://docs.docker.com/engine/install/ubuntu/#install-using-the-convenience-script) — the `get.docker.com` installer, privilege requirements, and production-use warning.
- [Docker volume documentation](https://docs.docker.com/engine/storage/volumes/#back-up-restore-or-migrate-data-volumes) — persistent storage and using a helper container to make a backup archive.
