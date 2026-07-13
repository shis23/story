# SQLite Chronicle Publication and Migration Safety Result

- Branch: `codex/sqlite-chronicle-migration`
- Baseline: `main@c3a972d`
- Worktree: `C:\tmp\storyforge-sqlite-chronicle`
- Date: 2026-07-13
- Default backend remains JSON (no Tauri / `storage.backend` wiring)

## Objective delivered

Extended the unused SQLite foundation (Turn accept + Chronicle A) into a complete
transactional Chronicle B/C publication path plus migration-safety toolkit,
without enabling SQLite in the production app.

## Schema versions

| Version | Name | Purpose |
| --- | --- | --- |
| V1 | `init_schema_v1` | Core business tables (unchanged) |
| V2 | `production_commit_ledger` | `mutation_commits` for Turn accept idempotency (unchanged) |
| **V3** | **`chronicle_publication_jobs`** | Durable B/C publication job / replay ledger |

### V3 table: `chronicle_publication_jobs`

- `publication_id` PK
- `campaign_id` FK → `campaigns`
- optional `job_id`
- `base_chronicle_revision` / `target_chronicle_revision`
- `parent_ids_json` / `child_covered_by_json`
- `payload_hash` (normalized request fingerprint)
- `status` (`completed` on success)
- `created_at` / `completed_at`
- index on `(campaign_id, completed_at)` and a partial unique index on non-empty
  `job_id`; completed import manifests also have a partial unique index as the
  durable fallback behind the post-`BEGIN IMMEDIATE` duplicate recheck

Migrations remain ordered, checksummed, idempotent, and safe under concurrent
cold start (existing runner rechecks version after `BEGIN IMMEDIATE`).

## APIs

### Chronicle publication (`crates/infra-sqlite/src/publication.rs`)

- `SqliteChronicleRepository::publish_compress`
- `publish_compress_with_fault` (test-only fault injection)
- `seed_summary` (bootstrap / test helper)
- `count_publication_jobs`
- Types: `PublishRequest`, `PublishOutcome` (`Applied` | `AlreadyPublished`),
  `PublishFault`

**Single `BEGIN IMMEDIATE` UoW semantics:**

1. Replay ledger by `publication_id` + payload fingerprint; completed replay
   **re-verifies** parents/covers/child `covered_by` against live rows (drift →
   conflict, not silent no-op). Same id / different payload → conflict.
2. Reject reused non-empty `job_id`; refuse to overwrite an unrelated existing
   `pending_compress_publication` marker.
3. Validate parents (B/C only), parent+child lineage (None rejected), conversation
   scope, continuous non-overlapping covers, child level (B covers A, C covers B),
   turn spans, and source graph presence **before writes**.
4. Write `pending_compress_publication` marker on Campaign.
5. Parent upsert (exact identity / payload; conflict on same id / different body).
6. Child `covered_by` updates + `round_summary_covers` rewrite.
7. Verify marker completeness, bump `chronicle_revision` once, clear marker /
   `context_epoch`, complete job row.
8. Fault points after parent insert, child update, revision bump, and marker
   cleanup each roll back the full publication.

### Migration readiness (`crates/infra-sqlite/src/readiness.rs`)

- `validate_source_manifest` — dry-run JSON tree validation + manifest hash;
  **never opens/writes a live DB**; hash labels/order match importer
  (`round_summaries`, sorted item encoding, path-sorted conversations)
- `create_backup_checkpoint` — unique millisecond/nanos filenames, refuse live
  path / existing-file overwrite, schema_version read from **backup DB**
- `export_readonly_snapshot` — single deferred snapshot transaction; includes
  `chronicle_publication_jobs` / `mutation_commits` / `import_runs`; corrupt
  `payload_json` fails closed; secret field names + free-text patterns redacted
  (`apiKey`, `credential`, `bearer`, `token=…`, …)

### Importer diagnostics (`crates/infra-sqlite/src/importer.rs`)

- Pre-write validation rejects:
  - duplicate Attempt ownership across Turns
  - partial/invalid summary graphs (missing cover parent/child)
  - covers ↔ `covered_by` bidirectional mismatch
  - parent/child scope drift (`campaign_id` / `conversation_id` / `lineage_id`)
- Still all-or-nothing: failed import leaves no business rows
- Attempt upsert also refuse rehanging an existing `attempt_id` onto another Turn

## Commit list

| Commit | Summary |
| --- | --- |
| `917f0de` | docs(workstream): plan SQLite chronicle migration |
| `0b5a426` | feat(infra-sqlite): transactional Chronicle B/C publication UoW |
| `904ff38` | feat(infra-sqlite): migration readiness tools and importer diagnostics |
| `5c30c6c` | docs(workstream): record SQLite chronicle migration result |
| (hardening) | fix(infra-sqlite): harden chronicle publication and migration readiness |
| (this update) | docs(workstream): update chronicle migration hardening evidence |

## Modified files (relative to `c3a972d`, excluding this RESULT until committed)

- `crates/infra-sqlite/Cargo.toml` — enable rusqlite `backup` feature
- `crates/infra-sqlite/migrations/V003__chronicle_publication_jobs.sql`
- `crates/infra-sqlite/src/lib.rs`
- `crates/infra-sqlite/src/migrations.rs`
- `crates/infra-sqlite/src/publication.rs` (new)
- `crates/infra-sqlite/src/readiness.rs` (new)
- `crates/infra-sqlite/src/importer.rs`
- `crates/infra-sqlite/tests/chronicle_publication.rs` (new)
- `crates/infra-sqlite/tests/migration_readiness.rs` (new)
- `crates/infra-sqlite/tests/importer_diagnostics.rs` (new)
- `crates/infra-sqlite/tests/migration_concurrency.rs` (expect schema V3)
- `docs/workstreams/SQLITE-CHRONICLE-MIGRATION-PLAN.md`

## Red → green evidence

### Chronicle publication

RED (stubs only): all 12 `chronicle_publication` tests failed with
`chronicle publication not implemented` / missing seed.

GREEN after implementation:

| Contract | Test |
| --- | --- |
| A→B atomic publish | `publish_a_to_b_commits_parents_covers_revision_and_job` |
| B→C on stage children | `publish_b_to_c_requires_existing_stage_children` |
| identical replay no-op | `repeated_identical_publication_is_noop_replay` |
| same publication id / different payload | `same_publication_id_with_different_payload_is_rejected` |
| overlapping / non-continuous covers | `rejects_overlapping_or_non_continuous_covers` |
| lineage / level / turn span | `rejects_wrong_lineage_parent_level_and_turn_span` |
| same parent id / different body | `same_parent_identity_with_different_payload_is_rejected` |
| fault after parent insert | `fault_after_parent_insert_rolls_back_publication` |
| fault after child update | `fault_after_child_update_rolls_back_publication` |
| fault after revision bump | `fault_after_revision_bump_rolls_back_publication` |
| fault after marker cleanup | `fault_after_marker_cleanup_rolls_back_publication` |
| accept path still usable | `existing_turn_accept_uow_still_green_after_publication_module` |
| child lineage=None rejected | `child_with_missing_lineage_is_rejected` |
| child conversation scope | `child_with_wrong_conversation_is_rejected` |
| completed replay checks live state | `completed_replay_rejects_when_db_state_drifted` |
| job_id uniqueness | `duplicate_job_id_is_rejected` |
| existing pending marker preserved | `existing_pending_marker_is_not_silently_overwritten` |

### Migration readiness / V3

RED: dry-run / backup / export / V3 upgrade failed as not implemented; concurrent
cold-start expected version 2.

GREEN:

| Contract | Test |
| --- | --- |
| dry-run counts, no live DB write | `dry_run_source_manifest_validation_reports_counts_without_writes` |
| dry-run invalid graph issues | `dry_run_flags_partial_and_invalid_source_graphs` |
| backup + live DB unchanged | `backup_checkpoint_writes_manifest_and_does_not_mutate_live_db` |
| export redacts secrets, no live mutation | `readonly_export_contains_no_secrets_and_does_not_mutate_live_db` |
| V1→V3 upgrade keeps data | `v1_and_v2_databases_upgrade_to_v3_publication_schema` |
| concurrent first start → V3 | `concurrent_first_start_migrations_are_idempotent` |
| unit V1 upgrade path | `existing_v1_database_upgrades_to_v2_without_losing_data` (now applies 2+3) |
| dry-run hash == importer hash | `source_manifest_hash_matches_importer_hash` |
| unique backup paths + backup schema | `backup_uses_unique_paths_and_backup_db_schema_version` |
| export jobs/ledger + extended secrets | `export_includes_jobs_ledger_and_redacts_extended_secrets` |
| corrupt payload fails export | `export_rejects_corrupt_payload_json` |

### Importer diagnostics

RED:

- duplicate Attempt ownership imported successfully (should reject)
- partial summary graph failed only as opaque FK error

GREEN:

- `importer_rejects_duplicate_attempt_ownership_with_clear_diagnostics`
- `importer_rejects_partial_summary_graph_without_half_import`
- `importer_still_accepts_valid_source_all_or_nothing`
- `importer_rejects_covers_covered_by_mismatch_and_scope_drift`

## Actual gate results (scoped only; no full workspace)

| Gate | Result |
| --- | --- |
| `cargo fmt -p storyforge-infra-sqlite -p storyforge-domain -- --check` | PASS |
| `cargo test -p storyforge-domain` | PASS: 243 passed |
| `cargo test -p storyforge-infra-sqlite --all-targets` | PASS: 21 unit + 19 publication + 15 readiness + 4 importer diagnostics + 1 migration concurrency + 25 production UoW |
| `cargo clippy -p storyforge-domain -p storyforge-infra-sqlite --all-targets -- -D warnings` | PASS |
| `git diff --check c3a972d..HEAD` | PASS |

Existing Turn accept UoW suite (`production_uow.rs`, 25 tests) remains green.

## Final review hardening

The final review pass closed the remaining fail-closed gaps without wiring the
SQLite backend into the app:

- importer duplicate detection is rechecked after acquiring the write lock,
  with a database unique-index fallback and a deterministic two-connection race
  test;
- completed Chronicle publication replay revalidates the exact parent payload,
  exact cover edge set, child reverse edges, target Campaign revision, cleared
  marker, context epoch revision, and live scope/lineage/level/span invariants;
- readiness validation rejects null collections and incomplete Chronicle graph
  identity, level, or turn-span data;
- backup checkpoints run `PRAGMA integrity_check` against the backup and do not
  expose sensitive labels or live absolute paths;
- read-only exports include `schema_migrations`, redact all operational tables
  recursively, and omit the live database path from the manifest.

## Default-backend proof

- No `tauri-app` dependency on `storyforge-infra-sqlite`
- No `storage.backend` change
- No JSON dual-write / JSON delete-or-move
- Publication / readiness / importer APIs require an explicit `Database` or
  source path and are unused by the production startup path

## Rollback evidence

1. **Publication faults**: four injected failure points leave
   `chronicle_revision`, parents, covers, marker, and job table at pre-publish
   state.
2. **Backup**: `create_backup_checkpoint` copies via rusqlite online backup and
   writes a separate manifest; live campaign row counts / schema version unchanged.
3. **Export**: secret-like keys stripped from export artifacts; live payload
   still contains the planted secret string, proving live rows were not rewritten.
4. **Importer**: corrupt / conflicting source trees return
   `CorruptImportInput` diagnostics with zero business rows.

## Remaining cutover work (explicitly out of scope)

1. App wiring / backend selector for SQLite
2. Production heal path using publication jobs (JSON path still owns runtime heal)
3. Full SQLite → JSON reverse migration for operational rollback
4. Android target compile / device file-lock validation
5. Multi-process long-running stress beyond concurrent migration + existing accept races
6. Enabling default `storage.backend=sqlite`

## Risks

1. Publication UoW is production-shaped but **not app-wired**; JSON compress
   publish remains the live path.
2. Export redaction is field-name + free-text pattern based; novel secret key
   names outside the deny/pattern list may still appear.
3. Backup requires the rusqlite `backup` feature (bundled SQLite); increases
   compile surface slightly when this crate is linked.
4. Importer diagnostics now cover Attempt ownership, bidirectional summary
   covers, and basic parent/child scope; broader entity-graph checks remain
   future work.
5. `job_id` uniqueness is enforced when provided; callers that omit `job_id`
   rely on `publication_id` identity only.

## Merge recommendation

**Recommend merge** of this branch into the integration line as an **unused**
SQLite capability slice.

Do **not** merge together with:

- default backend switch
- Tauri wiring
- JSON/SQLite dual-write
- Android availability claims
- edits to `docs/HANDOFF.md` (none made)
