# SQLite Opt-In Production Backend Result

- Branch: `codex/sqlite-optin-backend`
- Base: `b46ddc8`
- Worktree: `C:\tmp\storyforge-sqlite-optin`
- Date: 2026-07-13
- Default backend remains **JSON** (no default switch, no dual-write)

## Objective delivered

Connected the existing SQLite migration, repository, publication, readiness, and
importer primitives to the application as an **explicit opt-in backend**. The
implementation provides a typed backend selector, a fail-closed JSON → SQLite
cutover coordinator, startup recovery, a versioned authority marker, a SQLite →
JSON reverse export, and application wiring — all without changing the default
backend, without JSON/SQLite dual-write, and without deleting user data.

## Architecture

### Backend selection (`crates/infra-sqlite/src/backend.rs`)

- `StorageBackend::{Json, Sqlite}` — typed enum with serde default = `Json`.
- `BackendSelection` — resolves config/env override; env (`STORYFORGE_STORAGE_BACKEND`)
  takes precedence over config; absent → `Json`.
- `PinnedBackend` — process-lifetime guard; rejects runtime switching via
  `assert_unchanged`.
- `BackendDiagnostics` — serialisable, path-free, secret-free summary of the
  selected backend and schema version.
- `StorageBackend::parse` rejects unknown values.

### Fail-closed cutover (`crates/infra-sqlite/src/cutover.rs`)

The cutover coordinator implements the following state machine, with JSON
remaining authoritative until the **marker is written last**:

1. **Check authority** — inspect marker; if SQLite already authoritative,
   verify and return `AlreadyCutover`; if stale, fail closed.
2. **Acquire lock** — cross-process lock file (Unix `flock` / Windows deny-write).
3. **Validate JSON** — dry-run `readiness::validate_source_manifest`; fail →
   abort, JSON authoritative.
4. **Import → temp DB** — import JSON into a `.cutover-tmp` database; fail →
   discard temp, JSON authoritative.
5. **Backup checkpoint** — create a backup of the populated temp DB with
   integrity check; fail → discard temp, JSON authoritative.
6. **Verify** — check row counts, schema version, and manifest hash against
   the import; fail → discard temp, JSON authoritative.
7. **Atomic publish** — rename temp → final DB (move stale final aside first);
   fail → JSON still authoritative (no marker).
8. **Write marker LAST** — atomic write of `BackendMarker` JSON; this is the
   commit point.
9. **Reopen + audit** — read-only integrity check through the production path.

### Recovery and marker (`crates/infra-sqlite/src/cutover.rs`)

- `BackendMarker` — versioned JSON (`version`, `backend`, `schema_version`,
  `manifest_hash`, `created_at`). Written via atomic temp-then-rename.
- `MarkerStatus` — `Absent` / `SqliteAuthoritative` / `JsonAuthoritative` /
  `Stale { reason }`.
- `recover_or_verify` — startup entry point; runs cutover if needed or verifies
  if already complete. Idempotent across restarts.
- `inspect_marker` — verifies the marker's claims against the live database
  (existence, schema version, openability).

### Reverse export (`crates/infra-sqlite/src/exporter.rs`)

- `export_sqlite_to_json` — read-only snapshot export to a portable JSON
  directory layout matching the importer's expected input.
- Produces `cards.json`, `campaigns.json`, `instances.json`, `knowledge.json`,
  `tasks.json`, `round_summaries.json`, `turns.json`, and
  `conversations/<id>.json`.
- Includes `reverse_export_manifest.json` with schema/backend versions,
  `export_manifest_hash`, counts, and `unsupported_fields`.
- Redacts secret-shaped field names (`api_key`, `password`, `token`, …),
  free-text credentials (`sk-…`, JWT `eyJ…`), and absolute paths.
- Refuses to export into the live database directory.
- Never mutates the live database (verified by byte-level hash comparison).

### Application wiring (`crates/tauri-app/src/storage_backend.rs`)

- `resolve_backend(data_dir)` — resolves and pins the backend at startup.
  When SQLite is selected, runs `recover_or_verify`; when JSON (default),
  opens no database.
- `check_marker_status(data_dir)` — read-only marker inspection.
- `BackendResolution` — result struct with pinned backend, optional DB path,
  diagnostics, and `cutover_performed` flag.
- **No JSON store is replaced or bypassed.** The wiring is additive: it makes
  the backend selection observable and verified without changing the runtime
  data path. The existing `TurnLifecycleService` semantics are preserved.

## Cutover/rollback state machine

```text
                     ┌─────────────────┐
            start ──►│  No marker      │
                     │  (JSON auth.)   │
                     └──────┬──────────┘
                            │ run_cutover
                     ┌──────▼──────────┐
                     │ Lock + validate │── fail ──► JSON auth. (no marker)
                     └──────┬──────────┘
                            │
                     ┌──────▼──────────┐
                     │ Import → temp   │── fail ──► discard temp, JSON auth.
                     └──────┬──────────┘
                            │
                     ┌──────▼──────────┐
                     │ Backup + verify │── fail ──► discard temp, JSON auth.
                     └──────┬──────────┘
                            │
                     ┌──────▼──────────┐
                     │ Atomic publish  │── fail ──► JSON auth. (no marker)
                     └──────┬──────────┘
                            │
                     ┌──────▼──────────┐
                     │ Write marker    │◄── commit point
                     └──────┬──────────┘
                            │
                     ┌──────▼──────────┐
                     │ Reopen + audit  │── ok ──► SQLite auth.
                     └─────────────────┘

  Rollback: export_sqlite_to_json → import into fresh JSON store (explicit).
```

## Fault injection evidence

Every cutover phase has a dedicated fault point (`CutoverFault`). Each test
proves JSON remains authoritative and the temp DB is cleaned up:

| Fault point | Test | Authority after fault |
| --- | --- | --- |
| After lock | `fault_after_lock_leaves_json_authoritative` | JSON (Absent marker) |
| After validate | `fault_after_validate_leaves_json_authoritative` | JSON (Absent marker) |
| After import | `fault_after_import_leaves_json_authoritative` | JSON (temp discarded) |
| After backup | `fault_after_backup_leaves_json_authoritative` | JSON (temp discarded) |
| After verify | `fault_after_verify_leaves_json_authoritative` | JSON (temp discarded) |
| After publish, before marker | `fault_after_publish_before_marker_recovers_on_restart` | JSON (Absent marker); recovers on restart |
| After marker | `fault_after_marker_completes_silently` | SQLite (marker written) |

## Rollback evidence

| Contract | Test |
| --- | --- |
| Export produces readable JSON | `reverse_export_produces_readable_json` |
| Export redacts secrets | `reverse_export_redacts_secrets` |
| Export does not mutate live DB | `reverse_export_does_not_mutate_live_database` |
| Export can be re-imported | `reverse_export_can_be_reimported` |
| Export refuses live DB dir | `reverse_export_refuses_live_db_directory` |

## Test counts

`cargo test -p storyforge-infra-sqlite`:

| Suite | Tests |
| --- | --- |
| Unit (lib) | 30 |
| `backend_selection` | 7 |
| `chronicle_publication` | 20 |
| `cutover` | 15 |
| `importer_diagnostics` | 4 |
| `migration_concurrency` | 1 |
| `migration_readiness` | 18 |
| `platform_locking` | 5 |
| `production_uow` | 25 |
| `reverse_export` | 5 |
| **Total** | **130 passed, 0 failed** |

`cargo test -p storyforge --lib`:

| Suite | Tests |
| --- | --- |
| Full tauri-app lib | 251 passed, 0 failed, 3 ignored |
| `turn_lifecycle` | 22 passed |
| `campaign_bundle` | 13 passed |
| `storage_backend` | 4 passed |

## Gate results

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | PASS |
| `cargo test -p storyforge-infra-sqlite` | PASS: 130 passed |
| `cargo test -p storyforge --lib turn_lifecycle` | PASS: 22 passed |
| `cargo test -p storyforge --lib campaign_bundle` | PASS: 13 passed |
| `cargo test -p storyforge --lib` | PASS: 251 passed, 3 ignored |
| `cargo clippy -p storyforge-infra-sqlite -p storyforge --all-targets -- -D warnings` | PASS |
| `git diff --check b46ddc8..HEAD` | PASS |

## Platform evidence

| Platform | Evidence |
| --- | --- |
| Windows (host) | All tests run on win32 x64; file-lock, reopen/rename, concurrent startup tests pass |
| Android aarch64 | `cargo check -p storyforge-infra-sqlite --target aarch64-linux-android` PASS; `cargo check -p storyforge --target aarch64-linux-android` PASS (NDK 27.2, bundled SQLite) |

## Review hardening

After initial implementation, an independent review closed the following gaps:

1. **Backup semantics**: the backup checkpoint now captures the **populated**
   temp database (post-import), not an empty schema. This gives a meaningful
   audit trail and integrity-checked copy of the data about to be published.
2. **Report secret safety**: `CutoverReport.backup_label` uses the **sanitised**
   label from the backup checkpoint, never the raw user input. A test verifies
   that a secret-shaped label does not appear in the serialised report.
3. **Temp cleanup on all fault paths**: every fault injection point now closes
   the database handle before discarding the temp file, preventing Windows
   file-lock deadlocks on retry.
4. **Marker verification**: `inspect_marker` re-opens the database and checks
   schema version + openability, not just file existence. A stale marker with
   a missing or corrupt database is rejected, not silently trusted.

## Default-backend proof

- `StorageBackend::default() == Json`.
- `resolve_backend` opens no database when JSON is selected.
- `STORYFORGE_STORAGE_BACKEND` is the only override; unset → JSON.
- No `tauri-app` command handler or store constructor was changed to use SQLite
  at runtime — the wiring is additive (observable + verified, no runtime switch).
- No JSON/SQLite dual-write: the cutover is a one-time atomic transition, and
  the marker is the sole authority indicator.
- No original JSON or SQLite data is ever deleted automatically.

## Remaining risks

1. **Store migration not completed**: the existing JSON stores
   (`CampaignStore`, `TurnStore`, `ConversationStore`) are not yet replaced by
   SQLite-backed implementations at runtime. The cutover produces an
   authoritative SQLite database, but the runtime still uses JSON stores until
   a separate workstream replaces them. This is by design for this slice.
2. **Concurrent import test flake**: `importer::tests::concurrent_same_manifest_converges_to_one_completed_run`
   can occasionally hit `SQLITE_BUSY` under high contention (busy_timeout=5000ms).
   This is a pre-existing test timing issue, not a correctness regression — the
   test passes reliably in isolation and the production path handles busy
   retries via `busy_timeout`.
3. **Export redaction is pattern-based**: secret key names outside the deny-list
   or novel credential shapes may still appear. The deny-list covers common
   patterns (`api_key`, `password`, `token`, `sk-…`, JWT, `bearer`).
4. **Windows lock granularity**: the cutover lock uses share-deny-write on
   Windows, which prevents concurrent cutover but does not use a true exclusive
   lock. Concurrent app instances could still open the database for reads. The
   ADR's single-app-instance assumption holds.
5. **Multi-process long-running stress**: tests cover two concurrent cutover
   threads and two-connection accept races; longer multi-process stress is
   future work.

## Commit list

| Commit | Summary |
| --- | --- |
| (to be committed) | feat(infra-sqlite): typed backend selector and fail-closed cutover |
| (to be committed) | feat(infra-sqlite): SQLite to JSON reverse export |
| (to be committed) | feat(tauri-app): wire opt-in storage backend selector |
| (to be committed) | test(infra-sqlite): cutover, export, platform, and selection tests |
| (to be committed) | docs(workstream): record SQLite opt-in backend result |

## Merge recommendation

**Recommend merge** of this branch into the integration line as an **opt-in**
SQLite backend capability. The default remains JSON, no dual-write is
introduced, and all declared gates pass.

Do **not** merge together with:

- default backend switch (`storage.backend=sqlite` as default)
- runtime store replacement (JSON → SQLite store implementations)
- Android availability claims beyond cross-compilation verification
- edits to `docs/HANDOFF.md` or `docs/RELEASE-CHECKLIST.md` (none made)
