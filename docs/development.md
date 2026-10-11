# Development guide

Start with [CONTRIBUTING](../CONTRIBUTING.md) for fork/branch/PR workflow and safe checks. [Architecture](architecture.md) and [source navigation](modules.md) describe current subsystem boundaries. This is implementation guidance, not a session log.

## Focused changes

### Background analysis

Snapshot the required `State` fields under a short lock, release it, call `self.analyze(...)`, then apply results under another short lock. Give model output an explicit token/size bound and validate numeric ranges. Store durable text through `memory::write`: it writes a redb record, not a physical Markdown file. Add focused tests for parsing and maintenance behavior.

### Personality-aware output

Use `generate` / `generate_stream` for conversation. Preserve the line protocol, silence/reaction semantics, outgoing mention restrictions, and one-reply-per-channel ownership. Do not substitute personality generation for plain background extraction.

### Prompts and UI

Add/update shared behavior in **both** `prompts/en/` and `prompts/tr/`, and wire prompt dispatch in `src/prompts.rs`. Add Discord UI text to **both** language JSON files and access it with `strings::t`. File names, placeholders, and serialized protocol keys can retain Turkish spellings for compatibility; translate prose, not the contract. Keep sincere identity disclosure direct.

### Commands and cycles

Commands live in the single `command::definitions()` registration/execution table. Slow network/model handlers defer first and edit their response. New background cycles need startup/reconnect guarding, shutdown handling, and appropriate sleep/travel gates. Do not add a second command parser or duplicate configuration convention.

### Persistence

Person records use Discord IDs and topic records use slugs. Document new redb keys in [state files](state-files.md), their bounds in [constants](constants.md), and migrations where required. Archives remain readable files outside redb; they are not part of bot retrieval. Test persistence with synthetic records, never real member data.

## Common pitfalls

- `std::sync::MutexGuard` or Serenity cache guards must not survive an await. Copy IDs or clone the needed snapshot before network calls.
- `include!` groups share their parent module scope. Relative include paths and macro textual order matter; a file is not automatically a Rust module.
- Discord `ready` and `guild_create` fire again on reconnect. Preserve cycle-start/scanned-guild guards.
- Use `content_safe` rather than raw mention text. Preserve disabled outgoing mentions.
- A model may return empty or malformed output or require reasoning tokens. Keep existing bounds and error handling instead of assuming a provider-specific response shape.
- `CHANNELS` and Discord permissions define observation scope. Do not accidentally bypass them in history scans, interactions, or new handlers.

## Checking changes

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Use focused pure tests for parsing/calendar/content-building changes. The terminal bench (`cargo run -- chat`) requires a model key and may cost money; it checks the reply protocol, not Discord integration.

For an explicitly consented live test, use a private test server, restrict `GUILD_ID`/`CHANNELS` before launch, set a provider limit, and enable the required intents. Start from the configured folder, verify `/status` first, then try chat. Inspect records through the `/mind` card; durable records are keys in `durum/hafiza.redb`, **not** loose `INDEX.md` or `kisiler/` files. Overflow archives are the exception. Report exactly what was exercised, redact credentials and member data, and do not commit runtime state.

Generated local tooling artifacts are optional and ignored. No graph refresh, generated graph commit, public session chronology, or maintainer push is required for a contribution.
