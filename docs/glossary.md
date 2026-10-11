# Glossary (runtime terms that stay in Turkish)

Most code identifiers are English. This glossary explains retained storage/protocol vocabulary.
Some persisted keys retain Turkish names for compatibility; they do not select the bot's language.
Prompts and Discord UI follow process-fixed `BOT_LANG` (`en` default).

| Turkish term | Meaning / where it appears |
|---|---|
| durum | state — both the old name of the shared `State` struct and the name of the `durum/` folder |
| hafiza | memory — both the old name of `src/memory.rs` and the concept of "memory" |
| sohbet | chat — an open conversation in a channel |
| gecmis | history — a chat's messages (`Chat.history`) |
| sayac | counter — the number of messages the bot has sent in that chat |
| hackli | "hacked" — the number of rounds remaining in the hack prank |
| mesgul | busy — a reply is currently being generated in the channel |
| yasakli | banned — (no longer used, the channel ban was removed) |
| profil | group profile — `profil.md`, produced by the profiler agent |
| huy | temperament — `huy.md`, produced by the coach agent, the bot's evolving personality |
| duzeltmeler | corrections — the critic agent's `duzeltmeler.md` |
| kendim | "myself" — the diarist agent's `kendim.md`, the bot's own current state |
| gundem | agenda — opinions the wanderer agent writes after browsing the internet (`gundem.md`) |
| planlar / uyuyor | sleep plans / is asleep |
| son_yol_mesaji / duyurulan_seyahat | last on-the-road message day / announced trip (old field names, now `last_road_message`/`announced_trip`) |
| gezgin | wanderer — the agent that browses the internet (its code name is now `wander`/`wanderer_cycle`) |
| haberci | news agent — its code name is now `news_agent` |
| profilci | profiler — still appears as a category label in the `!durum` breakdown shown on Discord |
| gunlukcu | diarist — the agent that writes the chat into memory; stays as a category label |
| hoca | coach — the agent that shapes the personality; stays as a category label |
| elestirmen | critic — stays as a category label |
| ozetleyici | summarizer/compactor agent |
| resimci | image commenter agent; stays as a category label |
| kisi / kisiler | person / people — `kisiler/<id>.md` |
| konu / konular | topic(s) — `konular/<slug>.md` |
| olay / olaylar | event(s) — `olaylar/YYYY-AA.md` |
| arsiv / arsivle | archive |
| kanaat / puan / not | opinion / score / note — person file fields |
| bilgiler / etiket | facts / tags — person file fields |
| tarih / ay / saat | date / month / time |
| uyku / uyanik_mi / uykusuz | sleep / awake? / sleepless |
| gergin | tense — a personality-linked insomnia trigger |
| seyahat / yolda / gidiyorum | travel / on the road / I'm leaving |
| etkinlik | event (calendar) |
| yer / sebep | place / reason |
| favori | favorite — the always-favored user |
| ayar | setting — an env variable or the `/ayarlar` panel |
| talimat | instruction — a call's task text |
| kaynak | source |
| gelisim / evre / hak edilen | growth / stage / earned stage |
| dogum | birth — the moment it first ran (unix timestamp) |

## Turkish field names in the model's JSON output (deliberately not translated)
These field names are shared by the English and Turkish prompts and serde structs.
Translating the contract without updating both producers and consumers would break parsing.
- Willingness (`WILLINGNESS`): `puan` (0-10 score), `sebep` (single-sentence justification)
- Target selection (`TARGET_PICK`): `hedef` (person's name)
- Mood (`MOOD`): `durum` (e.g. "confusion"), `yogunluk` (1-10)
- Waking (`WAKING`): `ilgi` (0-10), `konu`
- Diarist (`DIARIST`): `olay`, `kisiler[].isim/puan_degisimi/not/bilgiler/etiketler`,
  `konular[].ad/not`, `kendim`

## The Turkish surface shown on Discord
With `BOT_LANG=tr`, slash names include `/durum`, `/yardim`, `/zihin`, `/ayarlar` and `/dusunme`.
English uses `/status`, `/help`, `/mind`, `/settings` and `/thinking`; see [commands](commands.md).
Some internal metric categories retain Turkish names (`sohbet`, `isteklilik`, `profilci`,
`gunlukcu`, `hoca`, `elestirmen`, `ozetleyici_*`, `haber_sec`, `hedef_sec`, `ruh_hali`,
`uyanis`, `uyandim`, `isim_sec`, `hack_giris`, `sorun`, `laf`, `gozlem`).
