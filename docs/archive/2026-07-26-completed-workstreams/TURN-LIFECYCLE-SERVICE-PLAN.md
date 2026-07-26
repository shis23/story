# Turn Lifecycle Shared Service Plan

- Branch: `codex/turn-lifecycle-service`
- Baseline: `main@c3a972d`
- Worktree: `C:\tmp\storyforge-turn-lifecycle`
- Mode: implementation + tests; no paid model calls

## Objective

Replace the current split between private Tauri turn lifecycle code and harness-side
`CommitProbe` mirroring with one reusable application service. The service must own the
production sequence from a finished pipeline draft through quality handling, postprocess,
Attempt synchronization, MutationBatch construction, commit/accept, and recovery state.

This is a vertical slice, not a helper-only refactor. Tauri commands and deterministic
harness tests must call the same production implementation.

## Required scope

1. Inventory the private `start_writing`, `regenerate`, QualityGate/autofix,
   Summarizer/PostProcessor, `build_mutation_batch`, Attempt hash/report synchronization,
   `commit_turn_attempt`, cancel cleanup and recovery logic.
2. Introduce a reusable application-layer service/module with explicit dependencies for
   Campaign, Conversation, Turn, compression/job and LLM/postprocess collaborators.
3. Make Tauri command handlers thin adapters over that service without changing public
   command payloads.
4. Make the M5 deterministic harness use the shared lifecycle instead of reproducing its
   state transitions in `CommitProbe` wherever safely possible.
5. Preserve the exact Turn/Attempt state machine, revision CAS, draft hash checks,
   QualityGate force/degraded semantics, old Attempt protection and idempotent replay.
6. Add deterministic end-to-end tests covering start, regenerate, one auto-fix, postprocess,
   accept, failure injection, crash/recovery and duplicate accept.
7. Clearly report any remaining Tauri-only background behavior that cannot yet be shared.

## TDD and acceptance

- Write failing contract tests before moving production logic.
- Prove Tauri adapter and harness service produce equivalent terminal records and mutations.
- Prove a failure after draft/autofix/postprocess cannot return text whose Attempt hash differs.
- Prove cancellation and storage failures clean current-cancel state and leave recoverable Turn state.
- Existing Phase B, Bronze, Turn recovery and M5 deterministic tests must remain green.

## Boundaries

- Do not call a real/paid LLM.
- Do not change SQLite or storage backend selection.
- Do not change UI, Android, production model defaults or `200/4/H/E`.
- Do not edit `docs/HANDOFF.md` or another workstream's PLAN/RESULT.
- Avoid new dependencies unless unavoidable; document every dependency change.

## Scoped gates

- `cargo fmt --all -- --check`
- tests for domain, app-pipeline, app-agent, tauri-app and affected harness targets
- strict Clippy for affected crates with `-D warnings`
- `git diff --check c3a972d..HEAD`

## Delivery

Commit coherent steps, do not push, and create
`docs/workstreams/TURN-LIFECYCLE-SERVICE-RESULT.md` with commits, moved production paths,
red-to-green evidence, remaining mirrors, test results and integration risks.
