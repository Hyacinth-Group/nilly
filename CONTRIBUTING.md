# Contributing

Thanks for helping. You can contribute without Rust: improve docs, translate prompts/UI, or report a reproducible bug. Start with [issue #7](https://github.com/krxi/nibby/issues/7) for a bounded code task, or browse [open issues](https://github.com/krxi/nibby/issues). Comment before taking a task so others know it is being worked on. Issues labeled `needs design` need agreement on behavior before implementation.

## Fork, branch, pull request

1. Fork the repository and clone your fork (or use a topic branch if you have write access).
2. Create a focused branch, for example `git switch -c fix-sleep-localization`.
3. Make one behavioral change and add/update focused tests. Read [architecture](docs/architecture.md) and [source navigation](docs/modules.md) as needed.
4. Run the non-modifying checks below. Describe changed behavior, checks performed, and any limits in the PR.
5. Push to **your fork/topic branch**, then open a pull request against `main`. Use a short English commit subject and keep unrelated cleanup out of the PR.

With stable Rust installed:

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Documentation-only changes need link/content review; say if Rust checks were not run. Never claim a live Discord/provider test you did not perform. A test server and model account are optional for most changes; live model calls may cost money.

## Shared behavior and safety

- Update English and Turkish `prompts/<lang>/` and `langs/<lang>.json` together when changing shared prompt/UI behavior. `BOT_LANG` defaults to English.
- Never hold state/cache locks across `.await`; snapshot inputs and release guards first.
- Conversation uses `generate` / `generate_stream`; background evaluation uses `analyze`.
- Persisted keys and model-output fields are compatibility contracts; renames require migration, not cosmetic edits.
- Explain non-obvious reasons or invariants in comments; do not add signature or caller-list templates.
- Never include tokens, server transcripts, photos, saved person records, `.env`, or runtime data in a PR or bug report. Redact logs. Rotate any leaked credential with its issuer.
- The bot must answer sincere AI/bot identity questions honestly; reduced UI is not human impersonation.

For bug reports, include version/commit, OS, language/provider/model (no keys), a minimal reproduction, expected versus actual behavior, and redacted errors. Do not publish other members' conversations without consent.

Implementation recipes: [development guide](docs/development.md). Optional tool guidance: [AGENTS.md](AGENTS.md).
