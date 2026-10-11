# Agent guidance

Human contribution workflow: [CONTRIBUTING.md](CONTRIBUTING.md).

## Build and checks

```sh
cargo build --locked
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

## Stable invariants

- Never hold a state or cache lock across `.await`; snapshot inputs, release the guard, then await.
- Discord UI text belongs in `langs/<lang>.json`; update English and Turkish together for shared behavior.
- Model prompts belong in `prompts/<lang>/`, embedded through `src/prompts.rs`, not Rust string literals.
- Personality-aware conversation uses `Bot::generate` / `Bot::generate_stream`; background evaluation uses `Bot::analyze`.
- Persisted record keys and protocol fields are compatibility contracts. Do not rename them without a migration.
- Never commit credentials, private server transcripts, saved records, or generated local tooling output.
