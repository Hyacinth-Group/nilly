# discord-bot

A self-hosted Discord bot with conversation memory and an evolving persona.

[Türkçe](README.tr.md) · [Setup guide](docs/setup-guide.md) · [Releases](https://github.com/krxi/discord-bot/releases) · [Changelog](CHANGELOG.md)

## How it works

discord-bot runs in a Discord server and can join conversations without a slash command. Background model calls write notes about people, topics, and the bot's previous replies; later replies use those notes alongside a fixed personality prompt. A sleep schedule and growth stages control some of its behavior. The bot is written in Rust, stores its memory locally, and uses your OpenRouter or Mistral key for model calls. Chat context leaves your machine, and background work counts toward your API usage.

It is an AI bot, not a human server member. The default persona answers identity questions honestly. There is no published Discord recording or long-term case study yet; a recording recipe is not evidence of personality improvement.

## Data flow and privacy

> **Before inviting it, tell members which channels it can read.** Set `CHANNELS` in browser setup before starting the bot. Blank `GUILD_ID` / `CHANNELS` means every server / channel the bot can access, not just conversations addressed to it.

| Destination | What it receives or keeps |
|---|---|
| Your machine | Person/topic notes, summaries, channel history and panel authentication in `durum/hafiza.redb`; overflow text in `durum/arsiv/` and any backups you make. Bot/model keys in a private `.env` (Docker: `config/.env`). |
| Model service | Selected chat text and saved notes for replies and background analysis. With OpenRouter, the serving model provider also processes the request. Attached images can be included: image analysis defaults on (`IMAGE_ANALYSIS=off` disables it). |
| Firecrawl, if configured | Page URLs and search queries for web reading/search. |

On its first scan it reads up to **14 days** of accessible text-channel history, subject to channel restrictions and Discord permissions. It observes messages while asleep. Local storage can retain summaries and archived material after a conversation ends. **Selective member deletion is not supported; `/reset` is not a data-erasure command.** See [storage and privacy](docs/setup-guide.md#privacy), including backups. Self-hosted does not mean local processing.

## What changes over time

- **The notes behind its replies.** Background analysis updates a group profile, temperament notes, and corrections after conversations. Those notes become context for later replies; model weights are unchanged.
- **What it remembers.** Person and topic records survive restarts in a local redb database. Retrieval is bounded, and old material is summarized. Archived text is kept for human inspection, not searched by the bot.
- **When it joins in.** Ordinary messages go through a willingness check. Mentions and an ongoing conversation can bypass that check while it is awake. It can reply, react, or stay silent.

The baseline persona and growth thresholds are defined in the source. This is a prompt-and-memory experiment; useful personality change needs to be shown with real conversations over time.

## Cost

We have not published a measured running cost yet. Replies and background analysis both use your model key. Start with a provider spending limit and check the provider's usage dashboard.

The setup defaults are **paid** models. An OpenRouter example verified in the live catalog on 11 October 2026 is `google/gemma-4-31b-it:free`; availability and limits can change, and it has not been tested end-to-end with this bot. Background calls can exhaust free limits too. Hosting may have a separate cost.

## Quick start — Docker

You need a Discord bot token, a model API key, and running [Docker](https://docs.docker.com/get-started/get-docker/). This setup requires copying commands into Terminal or PowerShell. The [step-by-step guide](docs/setup-guide.md) explains Discord account setup and includes Windows commands.

In [Developer Portal](https://discord.com/developers/applications) → **Bot → Privileged Gateway Intents**, enable **Message Content Intent** and **Server Members Intent**, not Presence Intent. Keep keys private.

For macOS/Linux:

```sh
mkdir mybot && cd mybot
curl -O https://raw.githubusercontent.com/krxi/discord-bot/main/docker-compose.yml
mkdir -p config photos
```

On Linux, make the configuration directory writable only by the container's UID 10001:

```sh
sudo chown 10001:10001 config
sudo chmod 700 config
```

Docker Desktop handles ownership differently; see the [setup guide](docs/setup-guide.md). No initial `.env` or CLI wizard is needed. Start the panel:

```sh
docker compose up -d
docker compose logs bot
```

Open the one-use `http://localhost:8787/setup#token=…` link printed in the logs; it expires after **15 minutes**. Follow the five steps: set a panel password of at least **12 characters** → verify Discord token → verify provider key → configure language/timezone, owner, access restrictions and news → invite and start. Once saved, the panel password survives interrupted setup; log in to resume. Setup starts the bot without a process restart. Credential tests verify Discord application identity and provider authentication; they **do not make a paid completion** or prove privileged intents, model compatibility or gateway readiness.

Run **`/status`** (`/durum` in Turkish) in Discord to confirm it works, then try mentioning it. Ordinary chat may be ignored, and the bot sleeps. Keep Docker running; use `docker compose logs -f` for errors.

Compose publishes only host loopback port 8787, mounts the writable `./config` **directory** (required for atomic settings saves) and `./photos` read-only, and keeps memory and panel ownership in the stable **`discord-bot-data`** volume. [Back up before upgrading](docs/setup-guide.md#back-up-the-memory); update with `docker compose pull && docker compose up -d`. Existing installations must [migrate `.env` to private `config/.env` before removing `env_file`, and preserve the old memory volume](docs/setup-guide.md#existing-compose-installations)—never discard it to fix startup.

Prefer another route? [Release binary (terminal required)](docs/setup-guide.md#download-a-release) · [Build from source](docs/setup-guide.md#build-from-source). Run the binary with no `.env` to open the same browser setup. `discord-bot init` remains available for headless setup; use `PANEL_ENABLED=off` to disable the panel.

## Panel boundaries and recovery

The embedded panel provides setup/repair, runtime and gateway status, per-conversation mood, validated settings with write-only secrets, live logs, account management and help. Memory/personality editors, action buttons and the dedicated usage page are **disabled** pending shared core APIs. Overview shows process-local call/token counters, not a measured running cost.

Settings saves require a restart (`docker compose restart bot`); a stored `model.md` override still takes precedence. Environment values supplied at process startup override the config file and are read-only in the panel. Live logs are a bounded, redacted recent buffer, **not a durable audit trail**; they can still contain private conversation text.

Use localhost or an SSH tunnel/VPN, not a public port. An HTTPS reverse proxy requires an explicit `PANEL_PUBLIC_URL` and preserved Host/Origin headers; forwarded headers are not trusted authority. See the [setup guide](docs/setup-guide.md).

For password recovery, stop the bot and run `discord-bot panel reset-password` from its usual working directory, using the same `durum/hafiza.redb`. Save the replacement password printed once, then restart; existing owner identity is retained. For Docker:

```sh
docker compose stop bot
docker compose run --rm bot panel reset-password
docker compose up -d
```

## Limits and reference

- Memory retrieval has a character budget; saved material does not imply perfect recall.
- Some persisted keys and remaining weekday/mood logic use Turkish. Travel uses a Turkish calendar and is disabled in English mode.
- `STEALTH` means **reduced bot UI**, not passing as a human: it disables slash interactions/UI embeds and sends completed replies instead of live edits. Leave it off for `/status` verification.
- Prompts are not a security boundary. Use channel permissions and provider spending controls.

[Architecture and agents](docs/architecture.md) · [Memory formats](docs/state-files.md) · [Configuration](docs/configuration.md) · [Commands](docs/commands.md) · [Documentation index](docs/README.md)

## Contributing and license

Fork, make a branch, and open a pull request. Docs, translations, and reproducible bug reports are welcome without Rust experience. See [CONTRIBUTING.md](CONTRIBUTING.md).

MIT — see [LICENSE](LICENSE).
