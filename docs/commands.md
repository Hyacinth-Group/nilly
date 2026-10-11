# Administration

The browser panel's **Actions** page provides shared controls with destination and side-effect review.
Sign in with the local administrator password. Chat commands depend on the platform and permissions:

| Platform | First check | Where the result appears |
|---|---|---|
| Discord | `/status` (`/durum` in Turkish) | Private embed card |
| Telegram | Owner sends `/status` (`/durum`) in an approved group | Bot's private chat; first open it and press Start |
| Slack | Owner sends `!status` or `@bot status` | Private ephemeral message; no Slack slash commands |
| Matrix | Owner sends `!status` (`!durum`) in an approved plaintext room | Private plaintext admin room; accept its invitation |

Non-Discord administration requires a configured qualified `OWNER_KEY`; room approval alone is not
administrative permission. See [setup](setup-guide.md#verify-the-connection).

## Discord slash commands

Discord management uses slash commands, not `!` text commands. Names follow `BOT_LANG`.
Responses are embed cards visible only to the caller; actions such as news can also post
to the channel. Slow commands defer before working to meet Discord's interaction deadline.

| English / Turkish | Behavior |
|---|---|
| `/status` / `/durum` | growth, counters, model, sleep, thinking, travel, token metrics, version; recommended first-run check |
| `/help` / `/yardim` | command list |
| `/mind [test]` / `/zihin [test]` | People/Topics/Events card, person picker and detail modals; `test` diagnoses the last 30 lines |
| `/settings` / `/ayarlar` | thinking, debug, sleep panel |
| `/reset [all]` / `/sifirla [hepsi]` | reset channel state/open chat, optionally all channels; **does not erase stored person records or archives** |
| `/news` / `/haber` | post news now |
| `/problem` / `/sorun` | post a software gripe |
| `/wander` / `/gez` | run agenda browsing |
| `/prank` / `/saka`, `/hack` | image prank / fictional hacked-bot bit |
| `/agents` / `/ajanlar` | run profiler and coach |
| `/wake` / `/uyan`, `/sleep [hours]` / `/uyu [saat]` | wake early / test sleep (default 8h) |
| `/thinking [mode]` / `/dusunme [kip]` | show, hide, silent, or off |
| `/model [id]` | show current model; only the configured qualified owner (`OWNER_KEY`, or legacy Discord `OWNER_ID`) can change it, saved across restarts |
| `/debug [state]` / `/debug [durum]` | willingness, target, mood, silence/reaction decision traces |

`STEALTH=on` means reduced bot UI and suppresses chat command UI; it is not human impersonation.
Keep it off for chat administration checks. See [configuration](configuration.md).
