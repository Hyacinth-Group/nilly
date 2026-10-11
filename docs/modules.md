# Source navigation

Use this map to locate a subsystem; signatures and detailed semantics belong in source/rustdoc, not a parallel caller inventory. [Architecture](architecture.md) explains data flow and the shared-scope `include!` layout.

| Subsystem | Entry points | Ownership / invariant |
|---|---|---|
| Process and configuration | `src/main.rs`, `src/bot/setup.rs`, `src/init.rs` | starts runtime, loads configuration, builds `.env`; never expose secrets in support output |
| Discord input | `src/bot/handler/handler_event.rs`, `handler_buttons.rs` | scope filters, history/mentions, willingness, interactions; ignore bot/webhook/DM chat |
| Conversation | `src/bot/chat/chat_reply.rs`, `chat_lookup.rs` | personality-aware turns and research; one in-flight reply per channel |
| Shared state | `src/bot/types/types_bot.rs`, `types_chat_state.rs`, `types_message.rs`, `types_settings.rs` | short mutex scopes; no lock across await; snapshot model inputs |
| Model requests | `src/bot/provider/provider_ask.rs`, `provider_ask_raw.rs`, `provider_generate.rs`, `provider_reasoning.rs` | raw requests and analysis/generation split; preserve explicit token bounds |
| Output and streaming | `src/bot/provider/provider_send_line.rs`, `provider_send_stream.rs`, `provider_stream.rs`, `provider_stream_view.rs`, `provider_system.rs` | bounded protocol, mention restrictions, streamed edits and final reaction sends |
| Text protocol | `src/bot/text/text_1.rs`, `text_2.rs`, `text_3.rs` | pure parsing/helpers; Unicode reactions, `-` silence, line-per-message burst limit |
| Background work | `src/bot/cycle/`, `src/agents.rs` | snapshots → analysis → short state update; finished chats queued for memory processing |
| Web/news | `src/agenda.rs`, `src/bot/cycle/cycle_news.rs` | configured RSS, page downloads/Firecrawl; external services receive queries/URLs |
| Memory | `src/memory.rs`, `src/migrate.rs` | redb text records; person IDs, topic slugs, bounded retrieval; real overflow files are not retrieved |
| Growth and schedules | `src/growth.rs`, `src/sleep.rs`, `src/travel.rs` | source-defined thresholds, UTC offset; travel only in Turkish; weekday/mood logic not fully localized |
| Slash commands and UI | `src/command.rs`, `src/command/registration_table.rs`, `src/modal.rs` | one registration/execution table; defer slow interactions; Discord UI comes from language files |
| Prompt dispatch | `src/prompts.rs`, `prompts/en/`, `prompts/tr/` | `prompts::current()` chooses embedded language-specific Markdown, not one language's constants |
| UI dispatch | `src/strings.rs`, `src/lang.rs`, `langs/en.json`, `langs/tr.json` | `strings::t(key)` follows process-fixed `BOT_LANG` (`en` default) |
| Terminal bench | `src/chat_cli.rs` | model-key-only reply protocol testing, not a Discord or persistence demo |

Some persisted keys retain Turkish names for compatibility; they do not select the bot's language. Do not cosmetically rename database keys or protocol fields without a migration. See [state formats](state-files.md), [prompts](prompts.md), and [glossary](glossary.md).

For behavior details, use [flows](flows.md); for numerical limits, [constants](constants.md). Contributor workflow lives in [CONTRIBUTING](../CONTRIBUTING.md), implementation recipes in [development](development.md).
