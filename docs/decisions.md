# Design decisions

These are the maintained rationale and tradeoffs behind the current design. Earlier chronology, superseded behavior and historical verification claims remain in Git history. This page is not a test report.

## Local transactional memory, compatible record keys

redb provides pure-Rust transactional storage in one file. Records remain text under path-like keys so moving from Markdown files did not require reshaping the person/topic format or retrieval interface. Modified timestamps live in a separate database table. The legacy importer preserves source records and timestamps.

Person records use Discord IDs rather than display names so name changes do not split a person's memory. Topic records use slugs. Some keys and serialized fields retain Turkish names for compatibility, independently of `BOT_LANG`. Renaming those contracts needs a migration.

Raw overflow remains readable files under `durum/arsiv/`, not database values. It is for operator inspection and is not searched by the bot. This does not provide selective deletion or perfect recall. [Formats and retention](state-files.md).

## Bounded retrieval rather than sending every record

A small index accompanies replies; person/topic/event details are retrieved within a character budget. Summarization reduces oversized records, with raw overflow archived. The index and fixed context are additional to the retrieval budget. Bounds are enforced in code instead of relying only on the model to stay concise. Tradeoff: relevant material can be missed or summarized imperfectly even when saved.

## Separate analysis and conversation

`analyze` is personality-free for extraction, scoring and maintenance; persona prose would add noise to those tasks. `generate` / `generate_stream` combine the fixed persona, saved notes and task for conversation. Image commentary is intentionally a conversational path. Notes change later reply context, not model weights, and source-defined growth thresholds remain fixed.

## Short lock scopes and per-channel reply ownership

Shared state uses a mutex, but network/model work happens outside it: snapshot inputs, release the guard, await, then apply results. A per-channel RAII guard releases busy ownership on exit/unwinding. Arrivals during generation can enter the next turn without discarding an in-progress stream. This avoids overlapping replies; it is not an API-spending guarantee.

## Embedded localized text and shared-scope files

Prompt Markdown is embedded at build time to keep prose separate from Rust; editing prompts requires rebuilding. Discord UI strings live in language JSON files. Both follow process-fixed `BOT_LANG`, defaulting to English. Persisted protocol keys are not translated alongside prose.

`include!` groups split bot/command source by topic while preserving parent private-item and macro access. The tradeoff is shared Rust scope across file boundaries, not independent modules. Serenity remains vendored for planned in-tree changes. [Source navigation](modules.md).

## Explicit request/protocol boundaries

OpenAI-compatible requests use reqwest JSON rather than a provider SDK, keeping request bodies and image blocks visible. Provider-specific options are selected at the endpoint boundary; a custom router need not support OpenRouter options.

Reply parsing supports bounded message lines, silence and Unicode reactions. Prompts can request behavior, but parsing, size limits and mention restrictions must be enforced by code. Prompt-injection rules are not a security boundary. Scope restrictions, Discord permissions and provider spending limits remain operator responsibilities.

## Honest identity and reduced UI

The default character is an AI bot without a real offline biography or fixed political affiliation. It answers sincere AI/bot questions directly. `STEALTH` reduces bot UI (slash interactions, embeds, thinking and live edits); it is not a human-impersonation feature and does not remove Discord's bot identity. This copy change does not change the mode's code behavior.
