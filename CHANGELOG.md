# Changelog

Notable changes are recorded here in [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) style. Git history retains earlier implementation details.

## [2.0.0] - Unreleased

### Added
- A shared multi-platform core: Discord, Telegram, Slack and Matrix connections can run together with source-qualified user, room and thread identities.
- Telegram long polling with explicit approved rooms, privacy-mode guidance, allowed emoji reactions and private owner administration.
- Slack Socket Mode with two-token setup, explicit channel approval, thread-aware replies, native file handling and private administrative results.
- Matrix plaintext-room support, enabled in default builds, with explicit room approval, persisted sessions and owner administration. Encrypted rooms and attachments are unsupported.
- An embedded local web admin panel on port 8787: five-step browser setup, connection overview, searchable/editable memory, personality, actions, usage, logs and settings. No separate web app is required.
- Revision-checked memory and temperament editing with preserved previous versions and shared coordination with background writers.
- Automatic, atomic migration of legacy Discord memory to the platform-qualified identity schema, preceded by a consistent database backup and migration manifest.
- Real admin panel screenshots, with fictional sample memory clearly labeled.

### Changed
- The default persona answers AI/bot identity questions honestly; its fictional university biography and fixed political identity are removed. `STEALTH` means reduced bot UI, not human impersonation.
- Browser setup can start the first configured platform without restarting. Later Settings changes need a restart; credential checks are not proof of live room access.
- Compose mounts writable `./config` at `/data/config`, uses `PANEL_CONFIG_PATH=/data/config/.env`, and publishes the panel only on `127.0.0.1:8787:8787`. Saved data keeps the explicit `discord-bot-data` volume.
- README and setup docs lead with Docker and browser configuration, with platform-specific requirements, collection boundaries and unmeasured-cost disclosure.
- Usage shows observed process-local calls/tokens; missing monetary charges stay “Not reported,” not estimated. Provider dashboards remain the billing source.

### Removed
- Tracked generated code graphs, public session/launch drafts, signature-restating comment templates, and unsupported setup-time/cost claims. Earlier material remains in Git history.

### Upgrading from 1.x
- Stop the old bot and make a private backup of the entire saved-data directory/volume before launching v2. First startup automatically creates a migration snapshot/manifest and commits identity changes atomically; unresolved legacy material is retained, not silently deleted.
- Keep the migration backup and old binary for rollback. Stop v2 and restore its pre-migration snapshot before using the old binary; do not run v1 against the migrated database. See [memory migration and rollback](docs/state-files.md#identity-schema-migration-and-rollback).
- Replace the old Compose file. Copy the existing `.env` to `config/.env` and mount the **directory**, not a single file; remove old `env_file` injection for browser-editable settings. Port 8787 is local-only. Do not discard an old project-prefixed volume: [migrate it explicitly](docs/setup-guide.md#existing-compose-installations).
- Set optional `OWNER_KEY` to a canonical platform-qualified user identity for Telegram/Slack/Matrix administration. It takes precedence over legacy Discord-only `OWNER_ID`; invalid nonblank values fail validation. See [configuration](docs/configuration.md).
- All connected platforms share memory/personality. Only approve trusted rooms; use separate installations for unrelated groups.

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

[2.0.0]: https://github.com/krxi/discord-bot/compare/v1.1.0...HEAD
[1.1.0]: https://github.com/krxi/discord-bot/releases/tag/v1.1.0
