# Contributing

Thanks for helping make the bot better! You will need a stable Rust toolchain. From the repository root, run:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
```

Keep Discord UI text in `langs/*.json` and prompts in `prompts/<lang>/`. Never hold a lock across `.await`, and keep identifiers in English.

See [AGENTS.md](AGENTS.md) for project rules and [docs/development.md](docs/development.md) for development recipes. New contributors are especially welcome to pick up issues labeled **good first issue**.
