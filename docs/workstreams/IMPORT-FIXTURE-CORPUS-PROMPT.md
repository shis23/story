# ZCode Prompt: Import/Export Real Fixture Corpus

Work only in:

```text
C:\tmp\storyforge-import-corpus
branch codex/import-fixture-corpus
```

Before Cargo commands, set `$env:CARGO_TARGET_DIR` to
`C:\tmp\storyforge-parallel-target`. Do not run `cargo clean`; wait for shared
Cargo locks and use affected-crate gates until final verification.

Read:

1. `docs/workstreams/IMPORT-FIXTURE-CORPUS-PLAN.md`
2. `docs/workstreams/IMPORT-EXPORT-HARDENING-RESULT.md`
3. `docs/RELEASE-CHECKLIST.md` only for evidence semantics; do not edit it.
4. current domain world-info/character formats, infra-import compatibility code,
   fixtures, and report scripts.

Use TDD and complete the entire PLAN. Run the existing local `test-card.png`
ignored fixture when available, but never commit raw card content or derived
private text. Evidence may contain only sanitized counts, hashes, feature flags,
fixture ids, seeds, and classified losses.

Build a larger generated/sanitized ST V2/V3 JSON/PNG and Campaign bundle corpus,
including large world books, aliases, extensions, regex/Reasoning placements,
MVU/TavernHelper payloads, multiple definitions, malformed inputs, size limits,
broken references, and A/B/C summary graphs.

Compare source -> first import -> export -> re-import. Classify every difference
as preserved, intentional normalization, unsupported, lossy bug, or N/A. Add
stable JSON/Markdown reporting, multiple fixed property seeds, reproducible
failure output, and strict completeness checks.

Keep all failure paths atomic and verify no partial stores. Do not extend Tauri
bundle commands, SQLite, or runtime Turn/Attempt portability in this branch.

Do not edit `docs/HANDOFF.md` or `docs/RELEASE-CHECKLIST.md`. Do not use GUI or
real LLMs and do not push.

Make logical commits, keep the worktree clean, run PLAN gates, and finish with
`docs/workstreams/IMPORT-FIXTURE-CORPUS-RESULT.md`. Report branch, HEAD, commits,
fixture/case counts, real-fixture status, losses found/fixed, gates, privacy
checks, residual boundaries, and merge advice.
