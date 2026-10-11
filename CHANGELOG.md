# Changelog

Notable changes are recorded here in [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) style. Git history retains earlier implementation details.

## [Unreleased]

### Added
- Telegram Bot API adapter with NAT-friendly long polling, explicit default-deny room approval, qualified owner administration and private administrative results.
- Discord/Telegram/Slack CLI setup selection, bilingual Telegram onboarding and honest visibility/history/nickname notices.
- Telegram safe media resolution/local uploads, native topic/reply routing, chat-allowed emoji reactions and a durable redb polling cursor; offline coverage and an ignored opt-in `getMe` probe.
- Slack Socket Mode adapter with explicitly approved `SLACK_CHANNELS`, workspace-qualified identities, thread-aware replies, native private-file resolution, bounded history and supported Slack reactions.
- Slack-only wizard setup with hidden `xapp-`/`xoxb-` tokens, qualified `OWNER_KEY`, bilingual Slack UI, exact app manifest and beginner installation/privacy/rate-limit guidance; Discord setup remains the default.
- Concurrent Discord/Telegram/Slack connections sharing the brain, with private Slack text-administration replies and no Slack slash-command registration.
- Offline Slack wizard/manifest coverage and opt-in ignored environment-gated live-test documentation; no live-workspace verification is implied.

### Changed
- Default prompts identify the bot honestly and omit a fictional university biography and fixed political identity.
- Shorter bilingual READMEs explain the memory mechanism, collection scope, external processing, and unmeasured running cost.
- Contributor and reference documentation describe current behavior rather than development sessions.
- Setup hides credentials on a terminal, explains paid defaults, and ends with numbered verification steps.
- CLI setup uses typed secret encoding and atomic config patches; the five-step browser wizard remains Discord-only, with Telegram/Slack configured through protected Settings and a restart.
- Compose uses the stable saved-data volume name `discord-bot-data`.

### Removed
- Tracked generated code graphs and public session/launch drafts; prior versions remain in Git history.
- Signature-restating source comment templates and unsupported setup-time/cost claims.

## [1.1.0] - 2026-10-11

### Added
- `discord-bot init` guided `.env` creation and Discord invite link.
- Docker Compose and multi-architecture container images (Linux amd64/arm64).
- Release binaries for Linux amd64/arm64, macOS Apple Silicon, and Windows x64.
- Configurable `OWNER_ID`, `TZ_OFFSET`, and `NEWS_RSS`, plus a beginner setup guide.
- MIT license, contribution guide, and issue templates.
- Optional `STEALTH` reduced bot UI: no slash interactions or UI embeds, and completed rather than live-edited replies.

### Changed
- English is the default language; Turkish remains available through `BOT_LANG=tr`.

### Upgrade notes
- To preserve the previous Turkish language and UTC+3 schedule, set `BOT_LANG=tr` and `TZ_OFFSET=3` in `.env`.
- Set `OWNER_ID` to your Discord user ID to retain owner-only model changes.

## 1.0.0 - 2026-09-04

This version was marked by commit [`7472fb9`](https://github.com/krxi/discord-bot/commit/7472fb9); there is no published GitHub release or `v1.0.0` tag.

### Added
- Discord conversation replies, silence and Unicode reactions, streamed output, and slash-command management.
- Background profiling, diary, temperament, criticism, summarization, and news analysis.
- Local redb memory with person records keyed by Discord user ID, bounded retrieval, raw overflow archives, and a legacy Markdown importer.
- English and Turkish prompt/UI dispatch, OpenRouter and Mistral model support, and a terminal chat bench.
- Sleep schedules, staged growth and name selection, image commentary, and reaction observation.

[Unreleased]: https://github.com/krxi/discord-bot/compare/v1.1.0...HEAD
[1.1.0]: https://github.com/krxi/discord-bot/releases/tag/v1.1.0
