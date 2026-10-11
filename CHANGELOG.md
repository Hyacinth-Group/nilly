# Changelog

Notable changes are recorded here in [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) style. Git history retains earlier implementation details.

## [Unreleased]

### Added
- Telegram Bot API adapter with NAT-friendly long polling, explicit default-deny room approval, qualified owner administration and private administrative results.
- Discord/Telegram/Slack/Matrix CLI setup selection, bilingual onboarding and honest platform capability/history/privacy notices.
- Telegram safe media resolution/local uploads, native topic/reply routing, chat-allowed emoji reactions and a durable redb polling cursor; offline coverage and an ignored opt-in `getMe` probe.
- Slack Socket Mode adapter with explicitly approved `SLACK_CHANNELS`, workspace-qualified identities, thread-aware replies, native private-file resolution, bounded history and supported Slack reactions.
- Five-step browser setup now selects Discord, Telegram, Slack or Matrix, tests credentials (`getMe`, Slack `auth.test`/`apps.connections.open`, Matrix `whoami`), and starts first setup without a restart; later platform additions through Settings require restart.
- Concurrent configured platform connections sharing the brain, with private Slack text-administration replies and no Slack slash-command registration.
- Offline Slack wizard/manifest coverage and opt-in ignored environment-gated live-test documentation; no live-workspace verification is implied.
- Default-enabled, removable `matrix` feature using native `matrix-sdk` 0.19.1 for plaintext sync, explicit room approval, owner-only prefix administration and stable persisted session/device state; source MSRV 1.96.
- Feature-gated Matrix CLI setup with hidden access-token/password entry, typed atomic patch/save, qualified owner identity, bilingual guidance, and a beginner plaintext-room/privacy guide; encrypted rooms and attachments have no decryption or plaintext fallback.
- Opt-in ignored `matrix_live` integration tests for session restoration, native transport operations and encryption rejection on disposable homeservers.
- Independent room-level attachment-sending capabilities, separate from optional upload ceilings, using native Discord/Telegram/Slack/Matrix permissions and safe unavailable defaults.
- Browser memory browsing, search and structured/full-document editing with revision conflicts, preserved pre-edit history, and atomic index/cache publication; background writers use the same revision discipline.
- Browser personality inspection with revision-checked temperament editing, all sixteen shared administration actions with destination/side-effect review and bounded jobs, and honest process-local usage reporting.
- A full-height responsive administration shell, one product-name source, and friendly unfinished-setup states.

### Changed
- Default prompts identify the bot honestly and omit a fictional university biography and fixed political identity.
- Shorter bilingual READMEs explain the memory mechanism, collection scope, external processing, and unmeasured running cost.
- Contributor and reference documentation describe current behavior rather than development sessions.
- Setup hides credentials on a terminal, explains paid defaults, and ends with numbered verification steps.
- CLI setup uses typed secret encoding and atomic config patches. Browser step 2 selects Discord, Telegram, Slack or Matrix; Telegram is recommended for beginners, and Telegram/Slack/Matrix room access is default-deny. Credential tests are not proof of live platform health.
- Browser platform selection uses a balanced desktop card grid, full-width readable beginner instructions and secondary help links; credential testing remains the primary action. Native language changes preserve the platform choice, while mobile navigation and checkbox labels offer 44-pixel touch targets.
- Settings offers separate collapsed Discord/Telegram/Slack/Matrix credential tests before the provider test, using supplied or saved effective values without saving or changing wizard drafts. Later additions can be tested before save/restart; Matrix browser tests require an access token and preserve saved runtime passwords. Probes are not live-health checks.
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
