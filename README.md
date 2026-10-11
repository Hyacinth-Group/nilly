# discord-bot

A self-hosted Discord bot with conversation memory and an evolving persona.

[Türkçe](README.tr.md) · [Setup guide](docs/setup-guide.md) · [Releases](https://github.com/krxi/discord-bot/releases) · [Changelog](CHANGELOG.md)

## How it works

discord-bot runs in a Discord server and can join conversations without a slash command. Background model calls write notes about people, topics, and the bot's previous replies; later replies use those notes alongside a fixed personality prompt. A sleep schedule and growth stages control some of its behavior. The bot is written in Rust, stores its memory locally, and uses your OpenRouter or Mistral key for model calls. Chat context leaves your machine, and background work counts toward your API usage.

It is an AI bot, not a human server member. The default persona answers identity questions honestly. There is no published Discord recording or long-term case study yet; a recording recipe is not evidence of personality improvement.

## Data flow and privacy

> **Before inviting it, tell members which channels it can read.** Set `CHANNELS` before first launch. Blank `GUILD_ID` / `CHANNELS` means every server / channel the bot can access, not just conversations addressed to it.

| Destination | What it receives or keeps |
|---|---|
| Your machine | Person/topic notes, summaries and channel history in `durum/hafiza.redb`; overflow text in `durum/arsiv/` and any backups you make. |
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

The wizard's default models are **paid**. An OpenRouter example verified in the live catalog on 11 October 2026 is `google/gemma-4-31b-it:free`; availability and limits can change, and it has not been tested end-to-end with this bot. Background calls can exhaust free limits too. Hosting may have a separate cost.

## Quick start — Docker

You need a Discord bot token, a model API key, and running [Docker](https://docs.docker.com/get-started/get-docker/). This setup requires copying commands into Terminal or PowerShell. The [step-by-step guide](docs/setup-guide.md) explains Discord account setup and includes Windows commands.

In [Developer Portal](https://discord.com/developers/applications) → **Bot → Privileged Gateway Intents**, enable **Message Content Intent** and **Server Members Intent**, not Presence Intent. Keep keys private.

For macOS/Linux:

```sh
mkdir mybot && cd mybot
curl -O https://raw.githubusercontent.com/krxi/discord-bot/main/docker-compose.yml
docker run --rm -it --user "$(id -u):$(id -g)" -v "$PWD:/data" ghcr.io/krxi/discord-bot:latest init
```

Before starting, open the generated `.env` and restrict `GUILD_ID` and `CHANNELS` to the intended server/channels. Then:

```sh
docker compose up -d
```

Open the wizard's invite link, choose your server, and run **`/status`** (`/durum` in Turkish) in Discord to confirm it works. Then try mentioning it. Ordinary chat may be ignored, and the bot sleeps. Keep Docker running; use `docker compose logs -f` for errors.

Saved data uses the stable Docker volume **`discord-bot-data`**. [Back it up before upgrading](docs/setup-guide.md#back-up-the-memory); update with `docker compose pull && docker compose up -d`. Existing installations with project-prefixed volumes need the [volume migration instructions](docs/setup-guide.md#existing-compose-installations).

Prefer another route? [Release binary (terminal required)](docs/setup-guide.md#download-a-release) · [Build from source](docs/setup-guide.md#build-from-source).

## Limits and reference

- Memory retrieval has a character budget; saved material does not imply perfect recall.
- Some persisted keys and remaining weekday/mood logic use Turkish. Travel uses a Turkish calendar and is disabled in English mode.
- `STEALTH` means **reduced bot UI**, not passing as a human: it disables slash interactions/UI embeds and sends completed replies instead of live edits. Leave it off for `/status` verification.
- Prompts are not a security boundary. Use channel permissions and provider spending controls.

[Architecture and agents](docs/architecture.md) · [Memory formats](docs/state-files.md) · [Configuration](docs/configuration.md) · [Commands](docs/commands.md) · [Documentation index](docs/README.md)

## Contributing and license

Fork, make a branch, and open a pull request. Docs, translations, and reproducible bug reports are welcome without Rust experience. See [CONTRIBUTING.md](CONTRIBUTING.md).

MIT — see [LICENSE](LICENSE).
