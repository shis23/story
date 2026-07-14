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
JSON reverse export, and a **real production storage boundary** for Accept /
recovery / active-turn barrier — all without changing the default backend,
without JSON/SQLite dual-write, and without deleting user data.

Architecture-review hardening (same branch) closed the previous blockers:

1. `resolve_backend()` is now called from `run()` before recovery;
2. SQLite Accept / recovery / barrier no longer consult JSON stores;
3. already-cutover restart audits SQLite only (deleted JSON still starts);
4. cutover rechecks authority after lock (TOCTOU closed);
5. verification recomputes DB content hash (not importer self-report alone);
6. reverse export stages then atomically publishes (no stale conversation residue);
7. conversation filenames reject path escape / absolute shapes.

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

### Application wiring

- `storage_backend::resolve_backend(data_dir)` — called from `run()` before
  `AppState` recovery. When SQLite is selected, runs `recover_or_verify` and
  activates the process-owned DB handle.
- `sqlite_runtime` — process-owned `Mutex<Database>` boundary:
  - `accept_by_variant` → `SqliteProductionRepository::accept_turn`
  - `recover_turns_on_startup` → `fail_incomplete_turns` (atomic accept model)
  - `get_active_turn` for turn barrier
  - `save_turn` / `save_conversation` / `save_campaign` / list helpers
- `commit_turn_attempt` and `recover_turns_on_startup` branch on
  `sqlite_runtime::is_sqlite_active()` and **never fall back to JSON** when
  SQLite is authoritative.
- In SQLite mode, `AppState` builds `ConversationStore` with the runtime
  `ConversationPersistence` adapter. `prepare_start_conversation`,
  `start_writing`, `regenerate`, variant edit/delete/switch, abandon, and
  attempt mutations therefore read and write the same SQLite authority.
  Persist failures are returned to the caller; a JSON shadow conversation is
  not created.
- `prepare_start_conversation` reads the selected campaign/conversation from
  SQLite; `start_writing` obtains the SQLite campaign revision and persists the
  new turn there; `regenerate` discovers and mutates its active attempt there.
  Post-accept invalidates the in-process conversation cache so it cannot retain
  a stale Draft after SQLite records the Final variant.
- The SQLite context snapshot reads campaign, instances, knowledge, tasks,
  summaries, card payload, and epoch state through the runtime boundary. The
  JSON-only chronicle compressor and MVU fallback fragments are explicitly
  skipped in SQLite mode rather than silently reading or writing JSON.
- The campaign picker (`list_campaigns`, `get_campaign`, selection, and active
  campaign lookup) reads SQLite after cutover. Selection is process-local in
  SQLite mode until a SQLite-native preference record is added; it does not
  read or write the legacy JSON authority pointer.
- `CampaignStore` becomes a no-I/O disabled sentinel in SQLite mode. Any
  unported JSON campaign read returns no legacy data and every mutation returns
  an error, which prevents a forgotten command handler from creating a shadow
  JSON write. Campaign creation and fork remain explicitly unavailable until
  they have SQLite-native transactions.
- `regenerate` verifies selected Campaign, requested conversation, and active
  Turn scope *before* it invokes the pipeline mutation. SQLite Accept also
  replays an already terminal request through its persisted mutation ledger
  rather than trying to re-prepare a non-`AwaitingAcceptance` Attempt.
- `sqlite_optin_lifecycle` performs a JSON-to-SQLite cutover, deletes the JSON
  campaign/conversation/turn source, then exercises user message, draft,
  regenerate, blocked normal Accept, scope rejection, force Accept, ledger
  replay, picker lookup, and restart recovery against a freshly re-opened
  SQLite database.
- JSON remains the default path when the selector is unset.

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
| `cutover` | 16 |
| `importer_diagnostics` | 4 |
| `migration_concurrency` | 1 |
| `migration_readiness` | 18 |
| `platform_locking` | 5 |
| `production_uow` | 25 |
| `reverse_export` | 6 |
| **Total** | **132 passed, 0 failed** |

`cargo test -p storyforge --lib`:

| Suite | Tests |
| --- | --- |
| Full tauri-app lib | 255 passed, 0 failed, 3 ignored |
| `turn_lifecycle` | 22 passed |
| `campaign_bundle` | 13 passed |
| `storage_backend` | 4 passed |

## Gate results

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | PASS |
| `cargo test -p storyforge-infra-sqlite` | PASS: 132 passed |
| `cargo test -p storyforge-app-conversation` | PASS: 19 passed |
| `cargo test -p storyforge --test sqlite_optin_lifecycle` | PASS: 1 passed |
| `cargo test -p storyforge --lib turn_lifecycle` | PASS: 22 passed |
| `cargo test -p storyforge --lib campaign_bundle` | PASS: 13 passed |
| `cargo test -p storyforge --lib` | PASS: 255 passed, 3 ignored |
| `cargo clippy -p storyforge-infra-sqlite -p storyforge-app-conversation -p storyforge --all-targets -- -D warnings` | PASS |
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
5. **Writing authority closure**: an external conversation load failure is
   retryable and never becomes a permanently loaded empty cache; a failed
   conversation mutation is rolled back/invalidated so a later save cannot
   flush rejected draft text. The selected Campaign/picker now routes to
   SQLite, JSON compress-job recovery is skipped, and cross-Campaign regenerate
   is rejected before pipeline mutation.
6. **Replay closure**: a terminal SQLite Accept reuses its durable
   `MutationBatch` and enters the repository ledger replay path, rather than
   failing the pre-write `AwaitingAcceptance` check.
7. **Active Campaign isolation**: startup and context fallback share one
   resolver that ignores `active_campaign.json` whenever SQLite is active. A
   restart regression test writes a stale JSON pointer and proves SQLite uses
   only an explicit in-process selection, while JSON mode retains its legacy
   behavior.

## Default-backend proof

- `StorageBackend::default() == Json`.
- `resolve_backend` opens no database when JSON is selected.
- `STORYFORGE_STORAGE_BACKEND` is the only override; unset → JSON.
- JSON-mode command handlers and stores retain their original behavior. SQLite
  routing is activated only after an explicit, verified opt-in cutover and is
  process-pinned for the lifetime of the app.
- Any remaining legacy `CampaignStore` access is inert in SQLite mode: it
  exposes no JSON-derived records and refuses writes, so there is no secondary
  authority or fallback after cutover.
- No JSON/SQLite dual-write: the cutover is a one-time atomic transition, and
  the marker is the sole authority indicator.
- No original JSON or SQLite data is ever deleted automatically.

## Remaining risks

1. **SQLite coverage is intentionally scoped to the writing lifecycle**:
   campaign creation/fork and some non-writing UI CRUD paths have not been
   migrated. They now fail closed through the disabled legacy store (creation
   and fork also have explicit command guards); JSON-only MVU fallback and
   chronicle compression are skipped rather than falling back to a JSON shadow
   store. Those operations need dedicated SQLite-native implementations before
   they can be enabled.
2. **Concurrent import test flake**: `importer::tests::concurrent_same_manifest_converges_to_one_completed_run`
   can occasionally hit `SQLITE_BUSY` under high contention (busy_timeout=5000ms).
   Pre-existing timing issue; production uses busy_timeout retries.
3. **Export redaction is pattern-based**: secret key names outside the deny-list
   or novel credential shapes may still appear.
4. **Windows lock granularity**: cutover lock uses share-deny-write; ADR single
   app instance assumption holds.
5. **Multi-process long-running stress** beyond concurrent cutover/accept races
   is future work.

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
