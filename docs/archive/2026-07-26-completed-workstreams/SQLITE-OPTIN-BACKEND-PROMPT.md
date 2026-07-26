# ZCode Prompt: SQLite Opt-In Production Backend

Work only in:

```text
C:\tmp\storyforge-sqlite-optin
branch codex/sqlite-optin-backend
```

Before running Cargo commands, set:

```powershell
$env:CARGO_TARGET_DIR = 'C:\tmp\storyforge-parallel-target'
```

The parallel workstreams share this directory to control disk usage. Wait for
Cargo locks, do not run `cargo clean`, and prefer affected-crate tests before a
single final broader gate.

Read completely before editing:

1. `docs/workstreams/SQLITE-OPTIN-BACKEND-PLAN.md`
2. `docs/workstreams/SQLITE-PRODUCTION-UOW-DAY-RESULT.md`
3. `docs/workstreams/SQLITE-CHRONICLE-MIGRATION-RESULT.md`
4. `docs/workstreams/SQLITE-MIGRATION-FOUNDATION-RESULT.md`
5. `docs/workstreams/TURN-LIFECYCLE-SERVICE-RESULT.md`
6. the current `campaign_store`, `turn_store`, lifecycle, and SQLite repository
   implementations.

Implement the full opt-in vertical slice with TDD. Start by writing failing
tests for default JSON selection, explicit SQLite selection, unknown values,
no dual-write, cutover phase failures, idempotent restart, reverse export, and
Windows lock behavior.

Use the existing SQLite repositories and readiness/importer helpers. Do not
create a second independent validation/state machine. Preserve
`TurnLifecycleService` semantics and route SQLite Accept/B/C publication through
the existing transactional APIs.

Implement a typed backend selector, startup wiring, fail-closed JSON -> SQLite
cutover into a temporary database, manifest/hash verification, atomic publish,
versioned marker, startup recovery, and SQLite -> portable JSON rollback export.
Backend selection is fixed for the process lifetime. JSON remains the default.

Never dual-write. Never silently fall back from a selected SQLite database to
stale JSON. Never delete original JSON or the SQLite DB automatically. Inject
failures after each cutover phase and prove authority remains unambiguous.

Test Windows reopen/rename/file-lock and concurrent startup. Attempt Android
aarch64 compile only when the existing toolchain is available; record a truthful
skip otherwise. Do not install large SDKs or use a device.

Do not edit `docs/HANDOFF.md` or `docs/RELEASE-CHECKLIST.md`. Do not touch real
LLM, GUI, plugin, import-corpus, or release-CI work. Do not switch the default to
SQLite. Do not push.

Make logical commits, keep the worktree clean, and finish with
`docs/workstreams/SQLITE-OPTIN-BACKEND-RESULT.md` documenting architecture,
cutover/rollback state machine, fault points, exact gates, unsupported cases,
platform evidence, and merge advice.

When finished, report branch, HEAD, commits, files, test counts, cutover proof,
rollback proof, remaining risks, and whether the opt-in path is ready to merge.
