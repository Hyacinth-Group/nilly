# Slash commands

Management uses slash commands, not `!` text commands. Names follow `BOT_LANG`. Responses are embed cards visible only to the caller; actions such as news can also post to the channel. Network/model commands defer before working to meet Discord's interaction deadline.

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
| `/model [id]` | show current model; only `OWNER_ID` can change it, saved across restarts |
| `/debug [state]` / `/debug [durum]` | willingness, target, mood, silence/reaction decision traces |

`STEALTH=on` is reduced bot UI: slash interactions are ignored. Keep it off for this management interface. See [configuration](configuration.md).
