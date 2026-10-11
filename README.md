# discord-bot

An AI character you add to your Discord, Telegram, Slack or Matrix group chat that remembers people, has moods and a sleep schedule, joins in on its own, and runs on your computer.

- Set it up and control it in your browser.
- See and correct what it remembers about people.
- Connect several chat apps to the same memory and personality.

[Türkçe](README.tr.md) · [Step-by-step setup](docs/setup-guide.md) · [Changelog](CHANGELOG.md)

## What the controls look like

![Browser setup with Discord, Telegram, Slack and Matrix choices](docs/images/setup-platforms.png)

*Actual local admin panel: choosing a chat platform during setup. No live chat or credentials are shown.*

<details>
<summary>See the overview, memory editor and personality page</summary>

![Admin overview before the bot is connected](docs/images/overview.png)

*Actual admin panel: the Overview before a chat connection is configured. This is not a running-chat demo.*

![Person memory editor with fictional Alex record](docs/images/memory-person.png)

*Actual admin panel: editing a person record. “Alex” and the saved notes are fictional demo data, not a member's conversation.*

![Personality page with editable temperament and read-only context](docs/images/personality.png)

*Actual admin panel: temperament editing and read-only personality context, with fictional demo notes. These screenshots show the interface, not measured personality improvement.*

</details>

## Start in 3 steps

Install and open [Docker](https://docs.docker.com/get-started/get-docker/) first. You need permission to add a bot to your group and an AI service account/key. You will copy three commands into Terminal on Mac/Linux; [Windows PowerShell commands and platform instructions](docs/setup-guide.md#start-with-docker) are in the guide.

**1. Create a folder and download the launch file.**

```sh
mkdir mybot && cd mybot && curl -O https://raw.githubusercontent.com/krxi/discord-bot/main/docker-compose.yml
```

On Linux, [prepare the writable settings folder](docs/setup-guide.md#download-the-launch-settings) before the next step. Docker Desktop users do not need the Linux ownership commands.

**2. Start it.**

```sh
docker compose up -d
```

**3. Open the setup link.**

```sh
docker compose logs bot
```

Find the `http://localhost:8787/setup#token=...` link and open it in your browser on the same computer. It is private, usable once, and expires after 15 minutes. The browser wizard asks you to set an admin password, choose a chat app, enter its bot credentials and your AI key, choose which rooms it may read, then start the bot. Platform accounts still need their own setup; the wizard gives instructions. [Expired link or remote server?](docs/setup-guide.md#browser-setup)

After setup, return to **http://localhost:8787** to view connections, edit memory/temperament, run actions, check usage/logs, or change settings. The panel is available only on this computer by default; do not publish port 8787 to the internet. Keep your computer and Docker running. [Verify your chat connection](docs/setup-guide.md#verify-the-connection) before relying on ordinary replies.

Terminal alternative: `discord-bot init` still provides [CLI setup](docs/setup-guide.md#headless-cli-setup).

Without Docker: download a [release binary](docs/setup-guide.md#download-a-release), start it from a terminal, and open its setup link; Rust is not needed.

**Already running v1?** [Read the upgrade instructions first](docs/setup-guide.md#upgrading-from-v1-to-v2). Memory migrates automatically with a backup, but Compose settings now live in `config/.env`.

## Choose your chat app

All four use the same AI and memory. What the chat app permits is different:

| App | What works | What to know before adding it |
|---|---|---|
| **Discord** | Chat, replies, reactions, images, slash commands and private control cards; up to 14 days of accessible history on first scan. | Enable **Message Content Intent** and **Server Members Intent**, not Presence Intent. Restrict channels before starting; blank Discord filters mean all accessible channels. |
| **Telegram** | Group chat, replies/forum topics, allowed emoji reactions, images and private owner controls. | Disable BotFather **privacy mode** and re-add the bot, or make it a group administrator, to read ordinary messages. Approve rooms explicitly. It cannot fetch old group history. |
| **Slack** | Channel/thread chat, reactions, images/files, bounded history and private owner controls. | Uses **Socket Mode**, an app token and a bot token. Invite it to approved channels. No Slack slash commands; use owner-only `!status` or `@bot status`. |
| **Matrix** | Approved plaintext-room chat, replies, reactions, images, bounded history and owner controls. | **Plaintext only:** encrypted rooms and attachments are unsupported. Replies are supported, Matrix threads are not. Set an owner and approve rooms; no Discord-style buttons/menus. |

Permissions can limit these features. A successful credential test is not proof that the bot can read or reply in a room. See [platform setup](docs/setup-guide.md) and [capability details](docs/architecture.md#platform-boundary).

## What “remembering” means

After conversations, background AI calls write notes about people, topics and the bot's replies. Later replies use selected notes alongside a fixed character prompt. You can inspect and correct notes in **Memory**, and edit its temperament in **Personality**; later background work can change those edits again.

This changes context, not model weights. Recall is limited, moods belong to individual conversations, and ordinary messages may get a reply, a reaction or silence. The character is an AI bot and answers sincere identity questions honestly. [How it works](docs/architecture.md).

## Cost

We have not published a measured running cost yet. Replies and background analysis both use your model key. Start with a provider spending limit and check the provider's usage dashboard.

The default models are **paid**. Free options have changing availability and limits; background calls consume those limits too. The panel reports observed usage, not a complete bill or spending cap. [Choosing a model](docs/setup-guide.md#model-keys-and-cost).

## Privacy and data flow

Tell members what the bot can read **before** adding it. Choose only trusted rooms: **all connected apps share one memory**, so something learned in one room may affect a reply elsewhere. Use separate installations for unrelated groups.

| Where | What goes there |
|---|---|
| Your computer | Saved notes, summaries, recent room history, retained edit history and archives; plus any backups you make. |
| Your model service | Selected messages and saved notes for replies/analysis. Images may be sent because image analysis defaults on; turn it off in Settings if needed. OpenRouter also sends requests to the serving model provider. |
| Firecrawl, if enabled | Page URLs and search queries for web reading/search. |

The bot observes approved messages even while asleep. **Selective member deletion is not supported**, and resetting a chat does not erase person records, previous versions, archives or backups. Running it yourself does not make AI processing local. [Collection, retention and removal details](docs/setup-guide.md#privacy).

## Contributing and license

Docs, translations and reproducible bug reports are welcome; you do not need to know Rust. For code changes, fork the project, make a branch and open a pull request. [Contribution guide](CONTRIBUTING.md) · [Technical docs](docs/README.md).

MIT — see [LICENSE](LICENSE).
