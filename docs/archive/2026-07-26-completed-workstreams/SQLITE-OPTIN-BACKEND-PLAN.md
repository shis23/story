# SQLite Opt-In Production Backend Plan

> Branch: `codex/sqlite-optin-backend`
> Worktree: `C:\tmp\storyforge-sqlite-optin`
> Base: `b46ddc8`
> Status: implementation plan; JSON remains the production default.

## Objective

Connect the existing SQLite migration, repository, Turn accept UoW, Chronicle
publication, backup, export, and importer primitives to the application as an
explicit opt-in backend. The implementation must provide a reversible cutover
without JSON/SQLite dual-write and without changing the default backend.

## Core Invariants

- Default startup remains JSON when no selector is present.
- Backend selection is fixed for one process lifetime.
- Never dual-write JSON and SQLite.
- A cutover must validate, backup, import into a temporary DB, verify, and only
  then atomically publish the SQLite database and marker.
- A failed cutover leaves JSON authoritative and usable.
- A successful SQLite run never silently falls back to stale JSON.
- Rollback/export is explicit, versioned, validated, and secret-safe.
- Existing Turn/Attempt/Campaign/conversation scope and revision invariants must
  remain identical across backends.

## Scope

### Backend Selection

- Add a typed `StorageBackend::{Json,Sqlite}` selector.
- Support an explicit config/environment/test override using existing config
  patterns; JSON is the serde/default value.
- Reject unknown values and runtime switching.
- Record selected backend and schema version in diagnostics without paths or
  secrets.

### Application Wiring

- Introduce the smallest application-facing storage boundary that can serve
  Campaign, Conversation, Turn/Attempt, summary, and accept/recovery operations.
- Wire SQLite through `tauri-app` without duplicating business validation.
- Route SQLite Accept through `SqliteProductionRepository` and Chronicle B/C
  publication through `SqliteChronicleRepository`.
- Preserve the shared `TurnLifecycleService` contract and terminal statuses.
- Wire startup migrations and recovery for the selected backend.

### Cutover

Implement a fail-closed one-time JSON -> SQLite cutover flow:

1. acquire a single-process migration lock;
2. dry-run JSON source validation and graph diagnostics;
3. write a source manifest and backup/checkpoint metadata;
4. import into a new temporary SQLite file;
5. verify counts, ids, revisions, Attempt ownership, summary graph, payload
   hashes, and schema version;
6. fsync/close and atomically publish the DB;
7. write a versioned backend marker last;
8. reopen through the production storage path and run a read-only audit.

Inject failures after every step and prove JSON remains authoritative until the
marker is safely published.

### Rollback and Export

- Provide SQLite -> portable JSON export matching the current JSON store input
  layout closely enough for an explicit rollback/import operation.
- Include an export manifest, schema/backend versions, hashes, and unsupported
  fields list.
- Fail closed on corrupt payloads or graph drift.
- Redact credential-shaped values, hostile keys, absolute paths, and operational
  errors using the hardened readiness helpers.
- Never automatically delete the SQLite DB or original JSON backup.

### Platform and Locking

- Add Windows reopen/file-lock/rename tests.
- Add concurrent-start and concurrent-cutover tests.
- Compile `storyforge-infra-sqlite` and the app for Android aarch64 when the
  target/toolchain is available; otherwise record a truthful environment skip.
- Keep bundled SQLite and final binary-size impact observable.

## Tests

Use TDD and fault injection for:

- default JSON and explicit SQLite selection;
- unknown selector rejection;
- no dual-write behavior;
- clean cutover and idempotent restart;
- interruption at each cutover phase;
- corrupted JSON, duplicate Attempt owner, graph drift, and stale marker;
- SQLite schema upgrade before app use;
- Accept, force-Degraded, regenerate, recovery, and B/C publication;
- reverse export and re-import equivalence;
- Windows lock and concurrent startup behavior;
- diagnostics/manifest secret redaction.

## Validation

Minimum gates:

```powershell
cargo fmt --all -- --check
cargo test -p storyforge-infra-sqlite
cargo test -p storyforge --lib turn_lifecycle
cargo test -p storyforge --lib campaign_bundle
cargo clippy -p storyforge-infra-sqlite -p storyforge --all-targets -- -D warnings
```

Run broader workspace tests if the app wiring changes shared interfaces. Do not
run real LLM or GUI suites.

## Deliverables

- typed selector and production storage wiring;
- fail-closed cutover coordinator and recovery marker;
- reverse export/rollback tooling;
- fault-injection and platform locking tests;
- diagnostics and operator commands/docs;
- `docs/workstreams/SQLITE-OPTIN-BACKEND-RESULT.md`;
- logical commits and a clean worktree.

## Acceptance

The slice is complete when JSON remains the default, an explicit SQLite startup
can migrate and run Campaign/Conversation/Turn/Chronicle flows, interrupted
cutovers recover without dual truth, reverse export is independently readable,
and all declared gates pass.

Do not switch the default to SQLite in this branch.

## Prohibited Changes

- no JSON/SQLite dual-write;
- no deletion of original user data;
- no default backend switch;
- no edits to `docs/HANDOFF.md` or `docs/RELEASE-CHECKLIST.md`;
- no M5/LLM, plugin, import-corpus, GUI, or release-CI work;
- no push, rebase, force push, or modification of `main`.
