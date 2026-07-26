# SQLite Chronicle Publication and Migration Safety Plan

- Branch: `codex/sqlite-chronicle-migration`
- Baseline: `main@c3a972d`
- Worktree: `C:\tmp\storyforge-sqlite-chronicle`
- Default backend must remain JSON

## Objective

Extend the unused SQLite foundation from Turn accept + Chronicle A into a complete,
transactional Chronicle B/C publication and migration-safety toolkit, without enabling it
in the production app.

## Required scope

1. Implement a typed B/C publication UoW covering parent upsert, deterministic covers,
   child `covered_by`, `chronicle_revision`, publication marker/job completion and replay.
2. Validate continuous/non-overlapping covers, source lineage, turn spans, parent level and
   payload identity before any write.
3. Make repeated identical publication a no-op and reject same identity/different payload.
4. Add fault injection after parent insert, child update, revision bump and marker cleanup;
   every fault must roll back the complete publication.
5. Add the minimum forward migration required for durable publication/job state. Migrations
   must remain ordered, checksummed, idempotent and safe under concurrent cold start.
6. Add migration readiness tools/APIs for dry-run source manifest validation, SQLite backup
   checkpoint/manifest and a read-only export suitable for rollback inspection.
7. Improve importer diagnostics for duplicate Attempt ownership and partial/invalid source
   graphs while preserving all-or-nothing import.

## TDD and acceptance

- Tests first for A->B, B->C, replay, conflict, bad covers, wrong lineage and each fault point.
- Tests for V1/V2 upgrade to the new schema, concurrent first start and checksum mismatch.
- Tests prove backup/export never mutate the live database and contain no secrets.
- Existing Turn accept UoW tests must remain green.

## Boundaries

- Do not connect SQLite to Tauri or change `storage.backend`.
- Do not add JSON/SQLite dual writes or delete/move user JSON.
- Do not claim Android support; host compilation is owned by another workstream.
- Do not modify M5, GUI, plugin or release scripts.
- Do not edit `docs/HANDOFF.md`.

## Scoped gates

- domain + `storyforge-infra-sqlite` fmt/tests/strict Clippy
- migration/import/export integration tests
- `git diff --check c3a972d..HEAD`

## Delivery

Commit coherent steps, do not push, and create
`docs/workstreams/SQLITE-CHRONICLE-MIGRATION-RESULT.md` recording schema versions, APIs,
rollback evidence, backup/export behavior, default-backend proof and remaining cutover work.
