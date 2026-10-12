# Roadmap

Released changes are in [CHANGELOG](../CHANGELOG.md). These items remain open; they are not promises of shipped behavior.

## Bounded contribution

- [#7: English weekday and mood-keyword localization](https://github.com/krxi/nibby/issues/7). English mode still has Turkish weekday/mood logic.

## Design decisions before implementation

- [#5: Custom emoji reactions](https://github.com/krxi/nibby/issues/5): agree on accepted protocol syntax and invalid-emoji behavior; current output supports Unicode reactions only.
- [#6: English-mode travel](https://github.com/krxi/nibby/issues/6): choose a locale/event set and travel windows, not an undefined “international” calendar. Travel remains disabled in English.
- [#8: Interest-triggered participation](https://github.com/krxi/nibby/issues/8): define the interest source, matching boundary and sleep/mention behavior before bypassing willingness.

## Evidence and operational limits

- Publish a consented real Discord example and a longitudinal note/reply comparison. Do not substitute fabricated conversations or a terminal recording for evidence of Discord behavior.
- Publish measured provider usage, separating startup history scans from ordinary replies and background work.
- Selective member-data deletion is not implemented. Records, derived notes, archives and operator backups need a complete design before an erasure promise can be made.
- The pending memory queue is in process memory and is lost on a crash.
- Seed/history windows count lines; multi-line replies consume more of those windows. Measure before tuning.
