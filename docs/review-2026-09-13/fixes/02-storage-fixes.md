# 域2 修复记录：存储与 Turn 生命周期（S-01..S-22）

- **任务**：`task-9`（修复域2：S-01..S-22 全量修复），owner `review-storage`
- **审查基线**：`docs/review-2026-09-13/02-storage-lifecycle.md`（1 P0 / 6 P1 / 11 P2 / 4 P3）
- **状态口径**（严格按 Lead 要求，不谎报）：
  - **已修复** = 代码改动 + 本域测试落地（给出测试名）
  - **已修复(降级)** = 真实风险已收敛，但未采用报告中的完整方案；前置依赖与残余风险写明
  - **判定非问题(附证据)** = 复核后不成立，附代码依据
  - **暂缓(附理由)** = 本轮不修，说明原因与下一步
- **硬约束遵守**：
  - 只改本域文件：`crates/infra-sqlite/**`、`crates/tauri-app/src/{campaign_store,storage,storage_backend,storage_health,json_store,turn_store,turn_lifecycle,turn_coordinator,playthrough_lifecycle,module_store,preset_store,connection_store,compress_job_store,card_studio_store,global_regex_store,sqlite_*,backend_workflows,production_postprocess}.rs`、`crates/tauri-app/tests/sqlite_*.rs`、`backend_parity_suite.rs`
  - **未**改 `docs/**`（除本文件）、`README.md`、`CLAUDE.md`；**未**改 `crates/tauri-app/src/lib.rs`、`commands/**`（域4/域6）、前端、`app-*`（跨域条目一律标 `暂缓(跨域)`）
  - **未**跑 `cargo test --workspace`（Lead 独占）
  - 每条发现都有测试或写明「为何测不了」

---

## 2 结论摘要

| 严重度 | 已修复 | 已修复(降级) | 判定非问题 | 暂缓(含跨域) |
|---|---|---|---|---|
| P0 | 0 | 1（S-01） | 0 | 0 |
| P1 | 5（S-02、S-03、S-04、S-05、S-07） | 0 | 0 | 1（S-06，跨域→域4 `lib.rs`） |
| P2 | 9（S-08、S-09、S-10、S-11、S-12、S-13、S-15、S-16、S-17） | 1（S-14） | 0 | 1（S-18 的 5/6 两个弱覆盖与反模式点） |
| P3 | S-19（除 item6）、S-20（2/5 降级）、S-21、S-22.1/22.4/22.5/22.8 | S-20.2、S-22.7 判定非问题 | 1（S-22.7） | S-19.6、S-22.2/22.3/22.6 |

**P0（S-01）没有完全按报告方案修**：`import_runs` 未加 `skipped_orphan_rows` 列（V009 schema 变更跨域），改为「报告 + 结构化日志 + `storage_health` backend incident」三条可见链路；**但「不完整源目录」这条真正的丢数据路径已经 fail-closed**（见 §3），且不会误伤合法老数据（Lead R1 复检重点，测试钉住 6 种布局）。

---

## 3 P0 详述

### S-01 [P0] 导入源不完整 → 孤儿行静默丢弃、边界丢弃 skip 明细 — **已修复(降级)**

**风险路径**：`readiness` 把「文件缺失」当空集合（与 JSON 同口径）→ `filter_orphan_rows` 把 `campaigns.json` 里引用不到的 cards 相关数据当孤儿过滤 → 用户拿到一个「部分数据」的 SQLite 权威库，且 `ImportSnapshot.skipped` 的明细在 `storage_backend` 边界被丢（只留 bool），重启后无从追责。

**已修（代码）**
1. `crates/infra-sqlite/src/readiness.rs::build_import_snapshot`（S-01 守卫，**R1 重新校准**）：
   - 捕获 `cards_file_present = data_dir.join("cards.json").exists()`、`campaigns_file_present`，以及 `cards_total`/`campaigns_total`。
   - **Rule A（父集合文件缺失 + 依赖非空 → 拒绝）**：`!cards_file_present && campaigns_total > 0` → `Err`，错误文案给出恢复路径（恢复 cards.json 或清理不一致的 campaigns 后重试）。
   - **Rule B（子集合文件缺失 + 有数据 → 拒绝）**：`!campaigns_file_present` 且 instances/knowledge/tasks/round_summaries/turns/conversations/world_info 任一非空 → `Err`，错误里列出具体集合与非零计数，绝不把全部依赖行当孤儿丢掉。
   - **校准理由（必须保留）**：fail-closed **只针对「文件缺失」**（=源目录拷贝/迁移中断）。**文件存在但为空**、或**逐行 dangling** 仍是「跳过 + 计数 + 审计」，因为：
     - 正常写入过的数据目录在存在 campaigns 时必然有 cards.json（因此 Rule A 不会误伤合法老数据）；
     - Gate 8 P2-A3 语义要求「缺集合 = 空集合」，且被 `importer::tests::transaction_rollback_on_fk_violation_mid_import`（dangling card 必须跳过而不是阻塞导入）钉住 —— 本轮最初版本的过度 fail-closed 曾让该测试变红，已按此校准。
   - 新增 `SourceManifestReport.skipped_orphan_rows: usize` + `skipped_detail: Vec<(String, usize)>`（按集合名计数）。
2. 明细贯穿边界：`importer::ImportReport`/`SourceSnapshot`、`cutover::CutoverReport.from_manifest_and_import`（新增第 6 个参数）都带上 `skipped`/`skipped_detail`；`sqlite_runtime` 把已迁移路径的硬编码 `skipped_orphan_rows: 0` 保留为显式注释（确有旧行为）。
3. 可见性（替代「加列」降级方案）：
   - `storage_backend::run_sqlite` 在 `CutoverOutcome::Completed(report)` 时审计 `skipped_detail`：非空 → `tracing::warn!` + `storage_health::record_backend_incident("cutover_skipped_orphans", detail)`（前端 `incidents()` 可见，不依赖重启后的报告）。

**测试**（`crates/infra-sqlite/tests/importer_diagnostics.rs`）
| 测试名 | 钉住的语义 |
|---|---|
| `missing_cards_json_with_campaigns_fails_closed_not_empty_authority` | Rule A：readiness/importer/cutover 三处都 Err；0 campaigns、无 completed import、无 DB/marker |
| `missing_campaigns_json_with_child_rows_fails_closed` | Rule B：错误含具体集合计数（`instances=1`） |
| `empty_cards_json_with_all_campaigns_dangling_is_skipped_with_audit` | 文件存在但空 → 跳过 + 计数 + 审计，不 fail-closed（**⚠️ 该边界已被 R13/§13 按证据推翻**：`cards.json` 存在但 0 张卡 + campaigns 非空 ⇒ fail-closed；测试已更名为 `..._is_rejected`） |
| `empty_campaigns_json_with_child_rows_is_skipped_with_audit` | 同上（子集合方向；R13 未收紧，仍成立） |
| `partial_dangling_campaigns_still_import_with_auditable_detail` | 部分 dangling → 正常导入，`skipped_detail == [("campaigns_no_card", 1)]`，cutover 报告同值（R13 未收紧，仍成立） |
| `legitimate_empty_and_partial_layouts_are_not_rejected` | 合法布局都 validate + import 通过且 0 skipped（R13 起为 **5** 种，新增 `legacy-conversations-no-campaigns`，见 §13/N-R1-01） |
| （既有）`importer::tests::transaction_rollback_on_fk_violation_mid_import` | dangling card 跳过语义不被 S-01 守卫破坏 |

**残余（降级项）**：`import_runs` 表仍无 skip 计数列 → 重启/重开后无法从 DB 读回历史 skip。需要跨域 schema 变更（V009 + 域4 诊断命令），记录在 §9「文档/后续同步条目」。

---

## 4 P1 详述（S-02..S-07）

### S-02 [P1] Stale marker 判据靠自由文本子串 + `DbVersionAhead` 可能触发重新导入 — **已修复**

**修法**
- `cutover::StaleKind`（typed）+ `as_str()`：`marker-corrupt` / `db-probe-failed` / `db-version-ahead` / `db-version-behind` / `db-binding-mismatch` / `marker-absent-orphan-db` / `db-missing` / `backend-unknown` …；`MarkerStatus::Stale { kind, reason }` 全量替换「错误文本匹配」。
- `inspect_marker` 顺序固定：MarkerCorrupt / VersionZero → backend 匹配 → `DbMissing` → `Database::open_readonly` 探测（失败=`DbProbeFailed`）→ `current_version`（`DbVersionAhead`/`DbVersionBehind`）→ `verify_database_with_marker` → `DbBindingMismatch`。
- `is_recoverable_stale` 只放行 `MarkerAbsentOrphanDb | DbMissing`；`DbVersionAhead` **只能** 由 `reconcile_marker_schema_version` 回写 marker（绝不回到 JSON 重导入，避免跨版本数据回退）；`DbProbeFailed`（瞬时占用/权限）不再当作「中断残留」。
- `storage_backend::run_sqlite`：Stale 分支先对 `DbVersionAhead` 做 reconcile → 重新 `inspect_marker`；仍 Stale 且不可恢复 → `BackendWiringError::Selection("stale backend marker (<kind>); refusing to start until resolved: <reason>")`（hard fail-closed，不再有隐式重跑）。

**测试**（`crates/infra-sqlite/tests/cutover.rs`）：`s02_version_ahead_marker_is_reconciled_never_reimported`、`s02_missing_db_with_matching_source_is_recoverable`、`s02_missing_db_with_changed_source_is_refused`、`s02_db_probe_failure_is_refused_not_treated_as_interruption`、`s02_marker_corrupt_and_unknown_backend_kinds_are_typed`、`s02_sqlite_marker_without_db_and_foreign_manifest_is_refused`；`crates/tauri-app/src/storage_backend.rs::sqlite_facade_revalidates_marker_authority_after_resolution`（marker 翻 JSON / 损坏 → fail-closed，带 typed kind）。

### S-03 [P1] 孤儿库所有权探测过宽（marker 缺失 + 任意 StoryForge 库 → 自动重发布） — **已修复**

**修法**
- `orphan_belongs_to_this_cutover(plan)` 双分支收紧：
  - fresh-start：必须 `orphan_db_has_no_user_data(path)`（9 张用户数据表只读计数全 0）+ 身份绑定匹配；
  - legacy：身份绑定匹配 **且** 内容 hash 等于当前源 manifest（`orphan_db_content_hash_matches`）。
- 内容 hash 读取改用 **`immutable=1` 只读连接**（`db_content_hash_immutable_readonly`，与 `owned_by_storyforge_readonly` 同一探测姿势）：普通只读打开 WAL 库会接触 `-wal`/`-shm`，sidecar 被外部占用时会把「读不出」误判成「内容不一致」而拒绝本可自愈的中断残留。读失败仍 fail-closed 并 `warn!`。
- `db_missing_source_matches_marker(plan)`：legacy 布局 → 源 manifest hash 必须等于 marker 记录；否则回退比较 `empty_db_content_hash()`（空库 = fresh-start 语义）。
- `recompute_db_content_hash` 重构为 `recompute_db_content_hash_conn(&Connection)` + 薄包装，供上述 immutable 路径复用（投影逐字节不变）。

**测试**：`s03_fresh_start_orphan_db_with_user_data_is_refused`、`s03_post_cutover_writes_block_stale_json_republish`、`s03_unchanged_source_with_lost_marker_is_recoverable`、`s03_orphan_db_from_other_source_identity_is_refused`、`s03_orphan_db_with_foreign_authority_binding_is_refused`（后者用 `UPDATE authority_binding SET authority_id='foreign-authority', cutover_nonce='foreign-nonce'` 模拟外来权威绑定）。

### S-04 [P1] JSON accept 幂等 Replay 无条件返回成功（不确认持久化） — **已修复**

**修法**（`crates/tauri-app/src/turn_lifecycle.rs`）
- `AcceptDecision::Replay(outcome)` 不再直接 `Ok(outcome)`：必须用 Campaign revision 复确认「journal 声称的副作用确实落盘」——`campaign.revision >= outcome.campaign_revision_after`，否则 `AcceptError::Storage("… 提交记录存在，但 Campaign revision 未达到 N——拒绝当作成功重放")`（fail-closed）。
- 终态标记（accept 收口）失败时：重试 1 次；仍失败 → `record_backend_incident("json_accept_terminal_mark_failed", …)` + `Err(AcceptError::Commit(…))`（不再静默保持 Committing 无痕）。
- `campaign_revision_after = batch.target_revision`（UoW 内已校验 `target == expected + 1`），并显式记录不一致日志。
- `json_writeback_allowed(status)`：已进入 `Committing`/终态的 Turn 拒绝 regenerate / 标记 Stale（`backend_workflows.rs`），与 SQLite 同语义。

**测试**：新增 `replay_fails_closed_when_persisted_revision_lags_the_journal`（接受成功后把 campaign revision 回退到 0 → 第二次 accept 必须 `AcceptError::Storage` 且文案含期望 target revision）；既有 `duplicate_accept_replays_idempotently_after_commit`（正常幂等重放仍 Ok）、`accept_decision_replays_terminal_turn_idempotently`。

### S-05 [P1] JSON 路径缺 `target == expected + 1` 与 final_revision 校验 — **已修复**

**修法**（`crates/tauri-app/src/turn_coordinator.rs`）
- `preflight_mutation_batch`：新增 `batch.target_revision != batch.expected_revision.saturating_add(1)` → `CommitError::MutationConflict("MutationBatch revision 契约不成立: …")`（与 SQLite UoW 同契约）。
- `apply_mutation_batch`：写入后校验 `final_revision == batch.target_revision`（防「写成功但 revision 不等于目标」的静默漂移）。
- `payloads_match` → `Result<bool, CommitError>`（序列化失败 fail-closed，不再当成「不相等/相等」的隐式分支）。

**测试**：新增 `preflight_rejects_batch_whose_target_is_not_expected_plus_one`（3 种非法组合 + 1 种合法组合）；既有 `apply_batch_bumps_revision_once`、`apply_batch_rejects_revision_conflict`、`apply_batch_idempotent_replay`、`apply_batch_sets_global_variable`、`apply_batch_upserts_knowledge_idempotently`、`apply_batch_upsert_instance_idempotent`。

### S-06 [P1] 迁移期 JSON 拷贝失败被吞（`lib.rs`） — **暂缓(跨域)**

**理由**：`crates/tauri-app/src/lib.rs` 属域4（tauri 命令/app 装配）写作用域，task-9 明确不越界。

**已做的缓解**：S-01 的 fail-closed 前置 + cutover 自身「拷贝不完整 → 拒绝导入/拒绝发布」链路（`missing_cards_json_with_campaigns_fails_closed_not_empty_authority` 同时断言「无 DB、无 marker」），使「拷贝失败后仍产出权威库」的路径被挡在 cutover 内部。

**建议（交域4）**：把 `let _ = fs::copy(...)` 改为 `?` + `TauriCommandError::storage`，失败即中止迁移并保留 JSON 权威。

### S-07 [P1] 启动恢复标记 `Failed` 失败被静默吞 — **已修复**

**修法**（`turn_lifecycle.rs::recover_turns_on_startup`）
- 未进入副作用的活动 Turn → 标记 `Failed`；写入失败不再 `let _ =`：`tracing::error!(target: "turn_recovery", …)` + `record_backend_incident("turn_recovery_failed_mark", …)`，文案说明「会阻塞后续 start_writing」。
- 同时把该 Turn 的**活动 Attempt 一并收口**（见 S-21），避免 Turn=Failed + Attempt 活动态并存。

**测试**：既有 `recovery_fails_active_and_keeps_committing_when_finalize_missing`（活动 Turn 变 Failed、Committing 保持可重放）；`transient_recovery_errors_upgrade_to_failed_after_max_retries`（计数与升级阈值）。

---

## 5 P2 详述（S-08..S-18）

### S-08 [P2] 发布前未 checkpoint temp WAL / 发布后无复检 / 丢弃 temp 静默 — **已修复**

- `checkpoint_and_close_temp(plan)`：发布前 `PRAGMA wal_checkpoint(TRUNCATE)`，`busy != 0` 或 checkpoint 后 `-wal` 仍有字节 → fail-closed（绝不发布缺事务的库）。
- `verify_published_database(final, manifest_hash, authority_id)`：发布后独立只读复检（可打开 / schema 版本 / 内容 hash / 身份绑定）。复检后清理**本次复检新建且为空**的 `-wal`/`-shm`（有内容的 sidecar 只告警、绝不删）；这一步同时修掉「只读复检在库旁留下 sidecar，导致外部故障注入（把 `-wal` 路径顶替成目录）与下一次发布的所有权探测看到不确定状态」。
- `discard_temp_db`：失败不再静默（warn/error）。
- 顺序保持 S-09 不变（import → verify → backup → fault → publish）。

**测试**：`crates/infra-sqlite/tests/gate5_fault_matrix.rs`（10 用例，含 `sidecar_cleanup_failure_propagates_and_final_db_untouched`、`sidecar_cleanup_failure_with_existing_owned_db_leaves_final_db_untouched`）、`gate5_migration_matrix.rs`（六阶段 marker-last 矩阵）。

### S-09 [P2] 发布顺序与 marker-last — **已修复（保持并加固）**

`AfterBackup`/`AfterVerify`/`AfterPublishBeforeMarker` 故障点语义不变；`verify_published_database` 只在 rename 之后、marker 之前运行。测试：`gate5_migration_matrix.rs::marker_last_and_failure_retention_for_every_stage` + `gate5_fault_matrix.rs`。

### S-10 [P2] `journal_mode` 返回值被丢弃（非 WAL 文件系统静默降级） — **已修复**

- `configure_connection(conn, expect_wal)`：文件库必须确认 `PRAGMA journal_mode` 真的是 `wal`，否则 `SqliteError::Other("journal_mode WAL was not applied (got …)")`；内存库跳过。`Database::open` → `expect_wal = true`，`open_in_memory` → false。
- `open_readonly` 补 `busy_timeout(5s)`。
- S-22.1 同步在 `connection.rs` 写明 `WAL + synchronous=NORMAL` 的取舍与依赖（提交点 fsync 在 cutover/rollback，幂等重放兜底）。

**测试**：`connection.rs::open_applies_required_pragmas`、`open_creates_parent_directories`、`concurrent_first_open_preserves_wal_and_application_identity`；`gate5_*` 系列全部在 WAL 断言下运行。

### S-11 [P2] `AuthorityLeaseGuard::drop` 提前 `return` 时 OS 锁已释放但簿记仍记持有 — **已修复**

- `HeldLeaseBook.entries: HashMap<PathBuf, HeldLease { mode, holders, file }>`：**真实 fd 托管给记账表**，只有最后一个 holder 释放时记录被移除 → fd 关闭 → OS 锁释放。
- owner 获取路径 `guard.take_file()` 把 fd 交给簿记；重入（`holders > 1`）只递减计数并 return，不会关闭 fd。
- 记账表中毒分支：`error!` + `release_os_lock()` 兜底（不泄漏 fd）；`release_os_lock` 改为持有式 `&mut self` 并取走 file（unix: `flock(LOCK_UN)` + drop；win: drop 句柄）。

**测试**：新增 `reentrant_guard_out_of_order_drop_keeps_os_lock_until_last_holder`（用**独立 fd 直接探测 OS 锁**：先 drop 真实 owner，重入 guard 存活时锁必须仍在；最后 holder 释放后锁必须真正释放且簿记清空）；既有 `same_process_shared_reentrancy_is_noop`、`same_process_shared_to_exclusive_upgrade_is_rejected`、`same_process_exclusive_then_shared_reentrancy`、`convenience_helpers_use_canonical_filename`。

### S-12 [P2] rollback「全有或全无」被 rename 后 fsync 失败打破 / marker 父目录 fsync best-effort — **已修复**

- `written.push((dst, old))` 改为**安装前**登记（安装中途失败也能回滚到旧文件）。
- marker 写入后的父目录 fsync **必须成功**：失败 → `SqliteError::Other("json-authoritative marker written but its directory fsync failed (the rename may not survive a power loss): …")`。
- `fsync_file`/`fsync_parent_dir` 抽到新模块 `crates/infra-sqlite/src/fs_atomic.rs`（S-20.5 去重），cutover/rollback 共用。

**测试**：`rollback.rs`：`rollback_happy_path_flips_marker_and_preserves_db_bytes`、`rollback_fault_after_export_keeps_sqlite_authoritative`、`rollback_fault_after_self_check_keeps_sqlite_authoritative`、`rollback_marker_write_failure_keeps_sqlite_authoritative`、`rollback_fault_after_json_install_is_idempotent_rerunnable`、`rollback_self_check_rejects_tampered_manifest`、`rollback_install_replaces_old_conversations_in_data_dir` 等 16 个。

### S-13 [P2] JSON 级联删除形同虚设（删除后再 `get` 恒 None）+ 错误被吞 — **已修复**

`storage_backend::delete_character_full_cascade`（JSON 路径）重写：**先取** `stored`（源 id 由此得出）→ 去重 extra source ids → **先级联**（MVU 翻译、卡壳）→ 任一步失败即返回带操作语义的错误（`"删除角色级联失败（MVU {source_id} 未清理，角色未删除，可重试）: {e}"` / `"…（卡 {} 未清理…）"`），最后才删主记录（失败可重试、不丢数据）。

**测试**：`crates/tauri-app/tests/command_atomicity.rs::json_delete_card_cascades_all_associated_data`、`crates/tauri-app/tests/sqlite_command_atomicity.rs::sqlite_delete_card_cascades_all_associated_data`。

### S-14 [P2] 失败被吞 — **已修复(降级)**

- `importer`：`INSERT import_runs … 'failed'`（失败路径的诊断行）不再 `let _ =`，改为 `if let Err(record_error) = … { tracing::error!(run_id, error, import_error, "failed to persist failed import_runs row (diagnostics only)") }`。
- `module_store`：内置默认 prompt profile / agent profile config 的持久化失败不再是 `let _ =`，改为 `tracing::error!` + 「下次启动会重试」文案。
- **降级说明**：这两处只提升可见性（行为不变），因此没有断言式测试——判别测试需要「注入写盘失败 + 捕获日志」，属跨域基础设施；已由 `tracing` + `record_backend_incident` 链路人工可查。
- **暂缓(跨域)残余**：`crates/tauri-app/src/commands/turns.rs`（soft_delete_variant 分支）、`commands/characters.rs`、`src/lib.rs` 内的同类 `let _ =`，均在域4/域6 写作用域内（§10 台账）。

### S-15 [P2] 快照复用的 stale turn 覆盖 + 删除活动不检查在途 Turn — **已修复**

- `production::fail_incomplete_turns`：改为在**同一事务内**逐条 `load_validated_turn(tx, &turn.turn_id)?` 重读最新记录，跳过已是终态的 Turn（不再用事务外的旧快照覆写）。
- `preaccept::fail_incomplete_preaccept`：同样在事务内重读，跳过 `Committing`/终态，写回前 `let turn = current;` 重绑定。
- `playthrough_lifecycle::delete_campaign_playthrough_with_deleter`：在 `active_campaign_update` 锁内新增**活动 Turn 屏障** —— `get_active_turn(campaign_id)` 为 `Some` → `TauriCommandError::validation("该活动还有未结束的回合（turn_id，状态 …），请先提交或放弃后再删除")`；读取失败 → storage 错误（不静默放行）。

**测试**：新增 `playthrough_lifecycle.rs::delete_playthrough_is_blocked_while_a_turn_is_active`（活动 Turn → 拒绝删除且 Campaign/会话不动；Turn 终态化后删除成功且落盘）；既有 `preaccept_lifecycle.rs` / `postprocess_*` 重放系列覆盖事务内重读路径；`sqlite_preaccept_production_lifecycle.rs` 覆盖 SQLite 侧恢复。

### S-16 [P2] JSON / SQLite 语义差距（Replay 契约、正文读取、revision 契约） — **已修复**

| 子项 | 修法 | 测试 |
|---|---|---|
| Replay 无条件成功 | 见 S-04 | `replay_fails_closed_when_persisted_revision_lags_the_journal` |
| `target != expected + 1` | 见 S-05 | `preflight_rejects_batch_whose_target_is_not_expected_plus_one` |
| `read_variant_content` 把「会话/节点/active 缺失」折叠成空串 → 误报 `DraftHashMismatch` | 改为 `Result<String, AcceptError>`，新增 `AcceptError::VariantContentUnavailable(String)`（文案指名缺失对象）；**作用域校验先于读正文**（`ensure_accept_scope` 提前调用），保持 `ConversationScopeMismatch` 优先级与 SQLite 一致 | `accept_reports_missing_variant_content_separately_from_hash_mismatch`（删会话文件 / 清空 nodes 两种场景），既有 `accept_rejects_conversation_scope_mismatch`（回归钉住优先级） |
| SQLite 侧同类折叠 | `sqlite_runtime` 同位置改为 skip 缺失节点/无 active 版本 → `AcceptError::VariantContentUnavailable`（与 JSON 同类型） | 见「无法测试的原因」① |

**① 无法测试的原因（SQLite 侧）**：`sqlite_runtime::activate` 是**进程级单例**，tauri-app 的集成测试二进制只能 activate 一次；要构造「节点无 active 版本」需要直接改 SQLite 会话行（facade 无 discard API），在 crate 内单测里 activate 会污染同一进程其它 lib 测试。因此该分支靠**同一 `AcceptError` 变体 + 代码评审**保证，JSON 侧由上述测试钉住同一语义。

### S-17 [P2] `validate_runtime_authority` 只比路径（运行期无 marker 复检、`check_marker_status` 无生产调用方） — **已修复**

`StorageFacade::revalidate_marker_authority()`：SQLite facade 在启动/运行期**再读一次 marker**（`check_marker_status` 由此获得生产调用方）——
- `SqliteAuthoritative | Absent | Stale{MarkerAbsentOrphanDb}` → 放行；
- `JsonAuthoritative` → `"storage authority changed underneath this process: backend marker is now json-authoritative; refusing to open the SQLite facade"`；
- 其它 Stale → `"storage authority is no longer verifiable (<typed kind>): <reason>"`。

**测试**：新增 `storage_backend.rs::sqlite_facade_revalidates_marker_authority_after_resolution`（有效 marker 通过 → 翻成 JSON 权威 → 拒绝，文案含 `changed underneath` → marker 损坏 → 拒绝且含 typed `marker-corrupt`）；既有 `stale_marker_refused_for_both_env_values`、`env_json_with_valid_sqlite_marker_fails_closed`、`valid_sqlite_marker_wins_without_env`。

**注意（供文档/运维）**：`DbProbeFailed` 现在会挡住启动（transient 占用也不例外），且 `DbVersionAhead` 的 reconcile 会**先回写 Foreign marker 的 schema_version**，随后仍可能因绑定不一致 fail-closed —— 即「回写版本」不等于「承认权威」，需要文档写明。

### S-18 [P2] 关键守卫零覆盖 / 两类假测试 — **已修复(降级)**（缺口 1/2/3/4 已补；5/6 暂缓）

| 缺口 | 处理 | 证据/测试 |
|---|---|---|
| 1. `accept_turn` 对 Superseded/Stale attempt 的拒绝无测试 | **已补** | `production_uow.rs::accept_rejects_superseded_and_stale_attempts_without_side_effects`（Turn 保持 AwaitingAcceptance、Attempt 置 Superseded/Stale → `Conflict` + campaign/turn/variant/summaries/ledger 零副作用） |
| 2. 事务中途失败时「已 INSERT 的行」回滚未覆盖 | **已补** | `production_uow.rs::injected_failure_rolls_back_every_accept_side_effect` 扩展为「全 mutation」batch（UpsertInstance/SetVariable/UpsertKnowledge/UpsertNewTask/SetTaskStatus），故障注入后断言 `character_instances`/`character_knowledge`/`story_tasks` 全空 |
| 3. Degraded 不可解除无 SQLite 集成测试 | **已由现有测试覆盖并明确引用** | `crates/tauri-app/tests/sqlite_optin_lifecycle.rs::sqlite_optin_cutover_write_regenerate_force_accept_and_restart_recovery`：force accept → `Degraded`；随后 `force_accept=false` 的重复 accept → 仍 `Degraded` + `commit_as_degraded=true`（不可升级为 Committed） |
| 4. commit 期失败传播未钉住 | **已补 + 提升可观测性** | `sqlite_runtime` 移除 `let _ = outcome;` 死语句，`AlreadyCommitted` 记 `tracing::info!`（幂等重放可查）；失败传播由 `production_uow.rs` 的 fault 注入系列与 `sqlite_optin_lifecycle.rs` 的 `fail_draft_uow_for_test` 路径钉住 |
| 5. 弱覆盖：真实 `integrity_check`/备份失败/磁盘满、migration SQL 自身失败、真双进程冷启动、SecretRef 跨 cutover | **暂缓** | 需要跨域/环境能力：磁盘满与 integrity 故障需要平台级故障注入（本沙箱不可控）；双进程冷启动需要独立 helper 二进制（`platform_locking.rs` 已有 lease helper，可扩展）；SecretRef 存活涉及 `connections.json`（域6/域4 的迁移集合，不在域2 manifest 投影内） |
| 6a. `backend_parity_suite.rs:2340` env-gated 重启子进程 no-op pass | **暂缓** | 改成默认执行会把「起真子进程 + 全量启动恢复」塞进默认套件，成本与时序风险跨域（Lead 决定是否拆独立 job）；已在代码注释标注 |
| 6b. perf 测试无阈值且未 `#[ignore]` | **暂缓** | 阈值需与 CI 机器/基线绑定，属质量工程决策（见 §9 文档同步条目） |
| 6c. 弱断言 `assert!(err.is_err())` | **暂缓** | 同类弱断言散落在域4/域6 测试文件，跨作用域 |

---

## 6 P3 详述（S-19..S-22）

### S-19 [P3] 注释/文档漂移 — **已修复**（item 6 跨域暂缓）

1. `production.rs:1-4` 模块注释改为现状（Gate 7 起 SQLite 已是默认权威，本模块经 `sqlite_runtime` 接入）。
2. `contract.rs` 头注释重写（「仅测试用适配基线，非生产路径」）+ 测试夹具注释更正（Gate 7 后「核心文件缺失 = 空集合」，夹具写空文件是为镜像正常数据目录并避开 S-01 fail-closed 前置）。
3. `cutover.rs` 的「下次 inspect_marker 看到 Absent 并自动重跑」更正为：下次看到 `Stale(MarkerAbsentOrphanDb)`，只有通过「身份 + 内容 hash」校验才允许自动重发布，否则 fail-closed。
4. `evaluate_accept_decision` 的 `Replay` 契约文档改为「调用方必须重新确认持久化，落盘 revision 不覆盖本次 accept 时 fail-closed」。
5. `read_variant_content` 改为 `Result` 并区分错误类型（见 S-16）。
6. `commands/diagnostics.rs` 存储面措辞（`connections.json`/`embed.json`）：**暂缓(跨域)**，域4 文件。

### S-20 [P3] 死代码/误导代码 — **已修复(降级)**

1. `sqlite_runtime.rs` 两句纯死语句（`let _ = outcome;` / `let _ = matches!(…AlreadyCommitted)`）→ 删除，替换为 `AlreadyCommitted` 的 `tracing::info!` 幂等重放日志。
2. `contract.rs::SqliteCampaignContract` 死代码 → **已修复(降级)**：不删除（它的 3 个测试覆盖 `migrations::migrate` + `UnitOfWork` 跨表事务 + importer→契约读，删除会丢覆盖），而是把模块标注为「仅测试用适配基线、禁止生产依赖」。真正的生产路径是 `production.rs` + `sqlite_runtime`。
3. `storage_backend.rs` 删除后 `get(id)` 恒 None 的级联 → 见 S-13（已重写）。
4. `let _ =` 型 guard 结果被丢弃 → 见 S-13（已改为传播错误）。
5. `fsync_file`/`fsync_parent_dir` 重复实现 → 抽到 `crates/infra-sqlite/src/fs_atomic.rs`；活动状态字面量列表 → 新增 `pub const ACTIVE_TURN_STATUS_SQL`（`infra-sqlite/src/production.rs`）并在 `sqlite_runtime.rs`、`sqlite_meta_repo.rs`、`production.rs` 三处统一引用。

**测试**：`production.rs::active_status_tests::active_turn_status_sql_list_matches_domain_is_active`（解析常量、与 `TurnStatus::is_active()` 集合逐项比对、断言终态不在列表内 —— 防未来枚举与 SQL 漂移）。

### S-21 [P3] 恢复升级 `Failed` 时未同步收口 Attempt — **已修复**

`turn_lifecycle.rs::record_recovery_issue`：超过 `MAX_RECOVERY_RETRIES` 升级 `TurnStatus::Failed` 时，**遍历所有非终态 Attempt 置 `Failed`**（旧实现只在显式传入 attempt_id 时才处理，而「缺 Committing Attempt」「缺 intended_terminal_status」两条路径都传 `None`）；`recover_turns_on_startup` 的 S-07 分支同样收口活动 Attempt。

**测试**：`recovery_upgrade_to_failed_closes_active_attempts`（MAX 次内 Turn/Attempt 都保持可重放；第 MAX+1 次 Turn=Failed 且 Attempt=Failed，断言「终态 Turn 上不允许残留任何活动 Attempt」）。

### S-22 [P3] 低风险观察（逐点） — **混合**

| # | 观察 | 判定 | 说明 |
|---|---|---|---|
| 1 | `synchronous=NORMAL` 断电窗口 | **已修复**（注释） | `connection.rs` 写明取舍、依赖（cutover/rollback 提交点 fsync + 幂等重放），并注明「若要提交即持久需 FULL + 显式 fsync」 |
| 2 | cutover 对源目录读两次（manifest hash 与 importer 落库之间源变动） | **暂缓** | 结果 fail-closed（不会产出错误 marker），只是「随机失败、重试才好」；合并为一次读需要重构 `readiness`/`importer` 的公共 API（跨函数签名 + 域1 协作），收益/风险比在收尾阶段不划算 |
| 3 | 多进程首开无租约窗口（`run_cutover` 释放独占租约 → `hold_process_shared_lease` 之间） | **暂缓** | 需要把「acquire 独占 → 发布 → 降级为 shared」做成**单次持锁交接**（Unix flock 无法原子降级，需引入 lease 文件+握手）。跨进程时序未能复现（原报告置信度中），改动窗口期风险高于收益 |
| 4 | 导出让位目录 `pre-export-*` 永不清理 | **已修复** | `exporter.rs::prune_old_pre_export_backups`：只匹配 `.{name}.pre-export-` 前缀的**目录**，保留最新 `MAX_PRE_EXPORT_BACKUPS = 2` 份，其余 best-effort 删除（失败仅 warn），非目录/非 UTF-8/无关目录一律不碰 |
| 5 | 导出 hash 与导入 manifest hash 命名空间不同却被注释称「同口径」 | **已修复**（注释） | `compute_export_hash` 上方写明两套命名空间**不可互比**及原因（导出侧空集合也写前缀，导入侧可选集合仅非空参与），并确认代码中无跨比路径 |
| 6 | rollback 安装的 JSON 树缺 `active_campaign.json`，且不清理陈旧指针 | **暂缓** | 需要产品决策（rollback 后活跃指针取「迁移前 JSON 值」还是「置空」）+ 跨 cutover/rollback 的指针快照链路；启动时有存在性校验，只 warn 丢弃，无数据损坏 |
| 7 | `derivation == None` 被判为「无失败」→ `Committed`（非 `Degraded`） | **判定非问题(附证据)** | `enable_postprocess`/`enable_summarizer` 由 profile 关闭时**本就没有推导记录**（`PipelineEvent::PostProcessSkipped`，不落 `derivation`），此时若把 `None` 当失败会把「按配置跳过后处理」的合法 accept 全部**不可逆地降级为 Degraded**。生产写入路径在有后处理时都会写 `Some(...)`；`None` 仅见于关闭后处理与 legacy 半写记录。因此保留现有语义并在 `evaluate_accept_decision` 文档中说明 |
| 8 | SQLite 按 variant 选 Attempt 不过滤终态（含 Superseded/Stale）→ 错误类型与 JSON 分叉 | **已修复** | `sqlite_runtime::accept_by_variant` 改为**优先取 `AwaitingAcceptance` 的 Attempt**，无活动 Attempt 时才回退任意状态（终态 Attempt 供 V7 幂等重放/typed 错误分类）；同处正文读取改为 typed `VariantContentUnavailable`（见 S-16 ① 测试限制说明） |

**测试**：`exporter.rs::prune_keeps_newest_pre_export_backups_and_touches_nothing_else`（最新 2 份保留、最旧清理、无关目录与同前缀普通文件不碰、`keep >= 数量` 时不动）。

---

## 7 门禁（本域命令 + 退出码 + 通过数）

> 由本成员运行的命令（**未**跑 `cargo test --workspace`，Lead 独占）：

| 命令 | 退出码 | 结果 |
|---|---|---|
| `cargo check -p storyforge --all-targets` | 0 | Finished dev profile（0 error） |
| `cargo check -p storyforge-infra-sqlite --all-targets` | 0 | Finished dev profile（0 error） |
| `cargo test -p storyforge --lib` | 0 | **470 passed / 0 failed / 3 ignored** |
| `cargo test -p storyforge-infra-sqlite` | 0 | **316 passed / 0 failed**（21 个 binary：unit 48、cutover 28、production_uow 31、importer_diagnostics 21、rollback 16、gate5_fault_matrix 10、gate5_migration_matrix 26、其余 136） |
| `cargo test -p storyforge --test sqlite_*`（15 个 `sqlite_*` 二进制，按名逐个传参；排除昂贵的 `sqlite_bigdata_perf`） | 0 | **19 passed / 0 failed**（含新增 `sqlite_meta_campaign_binding`） |

**跨域阻塞记录**：本轮修复期间 `crates/domain/src/story_task.rs:597` 一度编译红（他人编辑中途，非本域文件），导致一次 `cargo test -p storyforge-infra-sqlite` 以 exit 101 中止；domain 恢复绿色后已重跑全部本域门禁。

**前端/npm/vitest 门禁**：本域改动未触及前端（仅 Rust 存储层），且成员进程受沙箱限制无法运行前端测试运行器——如需前端回归由 Lead 收口。**本域无前端文件改动**。

---

## 8 改动文件清单（域2）

**infra-sqlite（`crates/infra-sqlite/`）**
- `src/fs_atomic.rs`（**新增**：`fsync_file` / `fsync_parent_dir`）
- `src/lib.rs`（注册 `pub mod fs_atomic;`）
- `src/error.rs`（`ImportSourceIncomplete`）
- `src/readiness.rs`（S-01 守卫 + `skipped_orphan_rows`/`skipped_detail` + `recompute_db_content_hash_for_test`）
- `src/importer.rs`（报告透传 skipped 明细；失败 `import_runs` 行写入不再静默）
- `src/cutover.rs`（StaleKind typed / 恢复资格判定 / 孤儿库身份+内容绑定 / S-08 checkpoint+复检+sidecar 清理 / immutable 内容 hash / S-19 注释）
- `src/rollback.rs`（S-12 written 时机 + marker 父目录 fsync 传播 + fs_atomic 复用）
- `src/connection.rs`（S-10 WAL 断言 + readonly busy_timeout + S-22.1 注释）
- `src/lease.rs`（S-11 记账表托管 fd + 乱序 drop 测试）
- `src/production.rs`（S-19 模块注释、S-20 `ACTIVE_TURN_STATUS_SQL`、S-15 事务内重读、S-20 漂移测试）
- `src/preaccept.rs`（S-15 事务内重读）
- `src/contract.rs`（S-19 注释修正；S-20.2 标注为测试基线）
- `src/exporter.rs`（S-22.4 让位备份有界保留 + 测试、S-22.5 命名空间注释）
- `tests/cutover.rs`（S-02/S-03 共 11 个新测试 + 辅助函数）
- `tests/importer_diagnostics.rs`（S-01 共 6 个新测试 + 夹具）
- `tests/production_uow.rs`（S-18.1 新测试、S-18.2 扩展故障注入用例）

**tauri-app（`crates/tauri-app/`）**
- `src/turn_lifecycle.rs`（S-04/S-16/S-19/S-21：Replay fail-closed、`VariantContentUnavailable`、`read_variant_content -> Result`、作用域优先、恢复收口 Attempt）
- `src/turn_coordinator.rs`（S-05/S-16：revision 契约、final_revision、`payloads_match -> Result`）
- `src/turn_store.rs`（移除重复 `get_turn` 定义）
- `src/storage_backend.rs`（S-13 级联重写、S-17 marker 复检 + `revalidate_marker_authority`、Stale 分支 typed 拒绝、cutover skipped 审计、`db_path.clone()` 借用修复）
- `src/storage_health.rs`（`record_backend_incident` + D-3 的 `record_write_fence_state` 接线 + 2 条测试）
- `src/connection_store.rs`（D-2：SecretRef 解析失败 fail-closed 的判别测试；实现本就无 ref-as-key 回退）
- `src/lib.rs`（**仅一行**：D-3 的 `record_write_fence_state()` 调用，Lead 指派落点；其余未动）
- `src/sqlite_runtime.rs`（S-20 死语句清理、S-20 活动状态常量复用、S-22.8 Attempt 选择、S-16 SQLite 正文读取）
- `src/sqlite_meta_repo.rs`（S-20 活动状态常量复用）
- `src/backend_workflows.rs`（`json_writeback_allowed` 守卫、补偿错误日志、M-16 变量键归一化）
- `src/playthrough_lifecycle.rs`（S-15 删除屏障 + 测试）
- `src/module_store.rs`（S-14 持久化失败可见）
- `tests/sqlite_meta_campaign_binding.rs`（**新增**：M-03 跨域请求的 SQLite 侧锚点/revision 归属覆盖）

**文档**：`docs/review-2026-09-13/fixes/02-storage-fixes.md`（本文件）

---

## 9 需文档同步条目（交 task-f7；本成员未改 `docs/**` 正文）

1. **S-01 语义**：`docs/ARCHITECTURE.md`/数据模型文档的「缺失集合=空」条款需补充例外——**cards.json/campaigns.json 文件缺失且存在交叉引用时 fail-closed**（不完整源目录），文件存在但为空仍是空集合；同时写明 skip 明细通过 warn + backend incident 暴露（`import_runs` 无该列）。
2. **S-10**：明确 SQLite 必须运行在 WAL 文件系统（不支持 WAL 时启动 fail-closed），并写明 `synchronous=NORMAL` 的断电取舍与提交点 fsync 边界。
3. **S-17**：写明 `DbProbeFailed` 会阻断启动、`DbVersionAhead` 由 marker reconcile 处理且「回写版本 ≠ 承认权威」（绑定不一致仍 fail-closed）。
4. **S-18/6b**：perf 测试（`gate5_bigdata_perf.rs`、`sqlite_bigdata_perf.rs`）目前只有 `println!`、无阈值未 `#[ignore]`，需决定「独立 perf job + 阈值」或标记 ignore；`backend_parity_suite.rs:2340` 的重启子进程检查默认 no-op，需决定是否默认执行/拆 job。
5. **S-22.2/22.3/22.6**：cutover 源读两次、首开无租约窗口、rollback 缺 `active_campaign.json` 三点应作为已知限制写入运维/架构文档（当前均为 fail-closed 或仅告警，不损数据）。
6. **S-22.4 保留策略**：导出让位备份保留最新 2 份（`MAX_PRE_EXPORT_BACKUPS`），需在产品/运维文档写明，以免用户以为旧备份一直存在。

---

## 10 遗留与阻塞

**跨域暂缓（不在 task-9 写作用域，需对应 owner 承接）**
| 项 | 文件 | 承接建议 |
|---|---|---|
| S-06 迁移拷贝失败被吞 | `crates/tauri-app/src/lib.rs` | 域4：`let _ =` → 传播错误 |
| S-14 残余吞错 | `commands/turns.rs`（soft_delete_variant）、`commands/characters.rs`、`lib.rs` | 域4/域6：补日志或返回错误 |
| S-19.6 诊断文案 | `commands/diagnostics.rs` | 域4：措辞与存储面同步 |
| S-18.6a/6b 弱断言与假测试 | `backend_parity_suite.rs`、`gate5_bigdata_perf.rs`、`sqlite_bigdata_perf.rs` | Lead 决定（成本/CI 策略） |

**测试限制（已如实记录，非同语义缺口）**
1. SQLite 侧 `VariantContentUnavailable` 分支无法做集成测试：`sqlite_runtime::activate` 进程级单例（每个测试二进制只能 activate 一次），crate 内 activate 会污染同进程其它 lib 测试；facade 无 discard API 构造「节点无 active 版本」。JSON 侧同语义已由测试钉住。
2. **前端测试运行器未运行**（沙箱限制，成员进程 `spawn EPERM`）；本域无前端改动，不需前端回归。
3. `cargo test --workspace` 未由本成员运行（Lead 独占）。

**风险提示（供 Lead 收口）**
- 本域改动跨两个 crate（infra-sqlite + tauri-app 存储层）且包含「发布/租约/恢复」这类时序敏感逻辑，最终判定请以 Lead 的全量门禁（`cargo test --workspace` + 前端）为准。
- 期间工作区曾出现 3 次它域编译红（`turn_store.rs:195` 重复 `get_turn`、`storage_backend.rs:2003` moved `db_path`、`turn_lifecycle.rs:1729` `data_dir` 字段）——前两处是历史遗留、第三处是本轮 S-16 测试的字段名，均已修复并通过 `cargo check -p storyforge --all-targets`（exit 0）。

---

## 11 收尾期跨域"静默回退到错误默认值"三项（Lead 指派，域2 落点）

Lead 在收口阶段转来域1 清单中的三条同类型缺陷（都属于"静默回退到错误默认值/错误语义"），均已处理：

### D-1 [跨域] `importer.rs` 的 `"Day 1"` 兜底与域侧单一口径分歧 — **已修复**

- **问题**：`crates/infra-sqlite/src/importer.rs` 在 `variables["story_clock"]` 与旧顶层字段**都缺失**时写死 `"Day 1"`；域侧唯一权威是 `storyforge_domain::variables::DEFAULT_STORY_CLOCK = "第1天"`（`Campaign` 的 serde default / 新建 Campaign 同值）⇒ 迁移进来的老数据与新建数据会长期并存两套默认值。
- **修法**：兜底改为引用同一个域常量（`storyforge_domain::variables::DEFAULT_STORY_CLOCK`），不再有第二套字面量。
- **测试**：`crates/infra-sqlite/src/importer.rs::tests::legacy_campaign_without_story_clock_uses_the_domain_default`——老数据同时缺两个 story_clock 来源 → 导入后 `SELECT story_clock FROM campaigns` 等于 `DEFAULT_STORY_CLOCK` 且 `!= "Day 1"`；随后改成显式 `"Day 9"` 重新导入 → 显式值不被默认值覆盖。
- **未改的字面量（有意）**：`contract.rs:82/172`、`importer.rs:1259` 的 `"Day 1"` 是**测试夹具里代表历史用户自定义值**（`variables` 为空、顶层字段存在），用于证明「record 里的值优先于默认值」；改成默认值会让这些用例失去判别力。

### D-2 [跨域] SecretRef 解析失败不得回退成 ref-as-key — **已复核：实现本就 fail-closed；补测试钉住**

- **复核证据（4 处调用点全部 fail-closed，无 ref-as-key 回退）**：
  1. `infra-util/src/secret_store.rs::resolve_secret_value`：`is_secret_ref(value) ? secret_store.get_secret(value) : Ok(value)` —— 是 ref 就只走凭据库，Err 原样上抛，**不会**把 ref 文本当值返回；`SystemSecretStore::entry` 对非 ref 输入直接 `Err("无效 SecretRef")`。
  2. `tauri-app/src/connection_store.rs::resolve_connection`（`:289`）：`resolve_secret_value(...)?` 传播错误；`set_active`（`:212`）同样 `?`；`active_connection`（`:234-239`）失败 → `tracing::warn!` + `None`（不返回带 ref 文本的连接）。
  3. `tauri-app/src/lib.rs:552-559`（embed.json）：`Err` → `warn!` + `return None`。
  4. `harness-real-llm/src/lib.rs:587`：`map_err(...)?`。
  - 另：`secure_api_key` 对已是 ref 的输入原样返回（不二次包裹），因此「列表把 ref 展示给前端 → 前端原样存回」不会把 ref 变成密钥名。
- **测试**（新增，落在域2 的 `connection_store.rs`）：`unresolvable_secret_ref_fails_closed_without_ref_as_key_fallback`——真保存一条连接（明文被迁移成 SecretRef）→ 清空凭据库条目（模拟换机器/清 Keychain）→ 断言 `resolved()` 返回 Err（含 `missing secret`）、`set_active()` Err、`active_connection()` None；并断言盘上仍是同一个 ref、凭据库里**没有**出现以 ref 为 key 的自指条目（失败路径不写任何条目）。

### D-3 [跨域接线] `write_fence` 冻结状态并入既有 `storage_health` — **已修复**

- **问题**：域1 的 `write_fence::freeze` 已接入向量库写入路径，但冻结只有日志/`PermissionDenied`，前端健康面（`storage_health_report`）看不到。
- **落点裁定**：域1 建议的直接落点 `lib.rs:841-843` 正确（向量存储创建处），但**登记逻辑写在域2 自己的 `storage_health.rs`**，`lib.rs` 只加一行调用 `crate::storage_health::record_write_fence_state();`（Lead 指定的最小接线；该文件属域4，本轮为此一处一行改动，已在此明示）。
- **去重策略（`record_write_fence_state`）**：
  - 已被 `json_store`（`record_unrecoverable`）登记为阻断事件的路径**跳过**——那条更完整（`.corrupt` 备份、`recovered_from_tmp`、原始解析错误），且 `push_incident` 按 path 去重，重登记会把它覆盖成信息更少的一条；
  - 其余冻结条目（如 `vectors.json`）按**真实文件路径**登记 `blocking = true` 事件，使前端 `storage_health_acknowledge(path)` 能直接对该文件解冻（若用 `backend:<kind>` 这类合成路径，解冻会落到不存在的路径上）；
  - `error` 前缀带域1 约定的 kind 串 `write_fence_frozen`（`StorageIncident` 无独立 kind 字段；不改前端 DTO 契约）；
  - 幂等：同路径重复扫描只保留一条。
- **测试**：`storage_health.rs::frozen_store_surfaces_as_write_fence_frozen_incident`（冻结 `vectors.json` → 健康面出现 `write_fence_frozen` + 原因、`blocking=true`、重复扫描仍 1 条、`acknowledge` 能真正解冻）；`storage_health.rs::write_fence_sweep_keeps_the_richer_json_store_incident`（json_store 已登记的损坏路径不被 sweep 覆盖，保留原始错误文本）。
- **域1 无需改任何接口**（只用了 `frozen_entries()`）。若域1 后续新增冻结点，无需再改域2：sweep 会自动带上。

---

## 12 收尾复跑的最终门禁（含 D-1..D-3）

| 命令 | 退出码 | 结果 |
|---|---|---|
| `cargo check -p storyforge --all-targets` | 0 | Finished（0 error） |
| `cargo check -p storyforge-infra-sqlite --all-targets` | 0 | Finished（0 error） |
| `cargo test -p storyforge --lib` | 0 | **470 passed / 0 failed / 3 ignored**（含 D-2/D-3 的 3 条新测试） |
| `cargo test -p storyforge-infra-sqlite` | 0 | **316 passed / 0 failed**（含 D-1 的迁移默认值测试） |
| `cargo test -p storyforge --test sqlite_*`（15 个二进制） | 0 | **19 passed / 0 failed** |

> 注：D-3 的一行接线落在 `crates/tauri-app/src/lib.rs`（域4 文件）——这是 Lead 明确指派的落点；除此之外本域未改任何域4/域6/前端文件。

---

## 13 R13（第二轮复检回归修复：N-R1-01 / N-R1-02 / N-R1-04）

- **来源**：`docs/review-2026-09-13/round2/R1-domain-storage-recheck.md`（冻结）附录 A 最小复现；Lead 指派 task-36（P1 启动阻断回归，优先于 task-35/R12）。
- **详述**：`docs/review-2026-09-13/round2/R13-readiness-legacy-layout-fix.md`（含负控命令与平台限制）。
- **写入范围**：`crates/infra-sqlite/**` + 本文件 + R13 报告；`crates/tauri-app/src/storage_backend.rs` **无改动**（应用层已 fail-closed：`recover_or_verify` 错误经 `BackendWiringError::Cutover` 上抛，无 JSON 回退）。

### N-R1-01 [P1] Rule B 把全部 conversations 当 campaign 依赖 → 合法非 Campaign 老用户启动即被拒 — **已修复**

- **根因**：conversations 是 legacy 布局里唯一允许 `campaign_id: null`（非 Campaign 聊天）的集合（`strict_validate_entries` 用 `opt_str_strict`，而 instances/knowledge/tasks/round_summaries/turns 用 `req_str`）。Rule B 却用 `conversations.len()` 统计「campaign 依赖」，于是合法布局「`cards.json` + `campaign_id: null` 的会话 + 无 `campaigns.json`」被判 `ImportSourceIncomplete`；默认后端 SQLite ⇒ 升级后启动即失败。
- **修法**：新增 `readiness.rs:155-165 conversation_is_campaign_scoped`（`campaign_id` 为非空字符串）；Rule B 只统计该子集（`:290-293`）。其余集合的 `campaign_id` 是领域必填，`len()` 即依赖数，不过滤。
- **测试**：`importer_diagnostics.rs:1409-1497` `legitimate_empty_and_partial_layouts_are_not_rejected` 新增第 ⑤ 组 `legacy-conversations-no-campaigns`（断言 Ok + `Completed` + `skipped_orphan_rows == 0` + `conversations == 1`）；**反向锁** `importer_diagnostics.rs:1501-1537` `campaign_scoped_conversations_still_require_campaigns_json`（campaign 归属会话 + 缺 `campaigns.json` ⇒ readiness 与 importer 都拒，文案点名 `conversations=1`）。
- **失败可控**：临时反转回 `conversations.len()` ⇒ 第 ⑤ 组 `FAILED`（`importer_diagnostics.rs:1484`）；还原后绿。

### N-R1-02 [P2] `cards.json=[]` + campaigns 非空 ⇒ 整棵 campaign 树静默丢成空库 — **已修复（采纳 fail-closed）**

- **证据**：`campaign_store.rs::delete_card`（`:298`）在「快照 + 候选副本 + 补偿」内级联删，落盘顺序 **`cards.json`(:377) 先、`campaigns.json`(:378) 后**；正常完成/补偿后两文件一致 ⇒「0 张卡 + 非空 campaigns」只可能来自写盘中断、部分拷贝、磁盘故障或手工编辑（正是 S-01 要拦的源目录不完整）。放行代价 = 0 campaigns 落库 + `Completed` + 权威 marker 固化；误伤面仅「删最后一张卡时被强杀」，且错误文案给出处置。
- **修法**：新增 Rule A′（`readiness.rs:254-279`，判据 `:271`）：`cards_file_present && cards_total == 0 && campaigns_total > 0` ⇒ `ImportSourceIncomplete`（文案 `cards.json exists but is empty …`，与「缺失」可区分）。**判据刻意不溢出**：`cards.json` 非空 + 个别悬空仍走「跳过 + 计数」（测试内反例断言）；`empty_campaigns_json_with_child_rows_is_skipped_with_audit`（campaigns 空 + 子行）不受影响。
- **测试**：原 `empty_cards_json_with_all_campaigns_dangling_is_skipped_with_audit` 改写为 `importer_diagnostics.rs:1158-1262 empty_cards_json_with_all_campaigns_dangling_is_rejected`，四层断言（readiness 错误文案 / importer 无行且无 `completed` run / cutover 不发布 DB 不写 marker / 部分悬空反例仍 `Completed`）。
- **失败可控**：临时 `if false && …`（等价移除 Rule A′）⇒ 该用例 `FAILED`（`importer_diagnostics.rs:1179`）；还原后绿。
- **代价 / 回滚**：极端路径「删最后一张卡时被强杀」会看到启动被拒（可操作）；若 Lead 更偏好不阻断启动，回滚点 = 删除 `readiness.rs:271-279` 判据块 + 该用例改回「跳过 + 计数」，其余改动不受影响。

### N-R1-04 [P3] `Path::exists()` 把「读不了」吞成「不存在」 — **已修复**

- **修法**：新增 `readiness.rs:113-152` `PathPresence{Missing,Present}` + `path_presence()`（只把 `ErrorKind::NotFound` 当缺失，其余 stat 失败 ⇒ 新错误 `SqliteError::ImportSourceUnreadable`，`error.rs:42-47`）+ 纯判定函数 `stat_error_means_missing`。
- **语义裁定**：`unreadable` = **fail-closed 且独立错误**（可选集合的「缺失 = 空集合」语义不得吞掉权限/IO 错误，否则静默空库；两种状态的处置完全不同）。
- **8 个调用点**：`readiness.rs` 的 `build_import_snapshot` 守卫（`:168-171`）、S-01 的 `cards_file_present`/`campaigns_file_present`（`:224-225`）、`read_json_array`（`:2069`）、`read_world_info_dir`（`:565`）、`read_conversation_dir`（`:2092`）、`importer.rs:81-85`；**外加** `cutover.rs:519-545 legacy_json_layout_present`（fresh-start 探测：stat 报错按「存在」处理 ⇒ 读不了绝不被判成「全新用户」去建空库）。
- **顺带修掉的同类缺口**：`cutover.rs:375-405 inspect_marker`——原 `.exists()` 短路使 ACL 拒绝时带权威绑定的 marker 被判 `Absent`；仓库既有 `StaleKind::MarkerUnreadable`（`:287`）正为此准备，现 stat 失败 ⇒ 该 Stale（零枚举/零调用方变更）。`cutover.rs:891-901 reconcile_marker_schema_version` 的 stat 失败改为传播错误（两个调用点都按 fail-closed 处理）。
- **测试**：`readiness.rs:2195-2257 path_presence_tests` 3 条（判定函数：`NotFound` vs `PermissionDenied`/`InvalidInput`/`TimedOut`/`Other`；真实临时目录三态；NUL 路径 ⇒ `ImportSourceUnreadable`）。
- **平台限制（诚实记录）**：沙箱内无法造出真实 `EACCES`（Windows 无 ACL 处置手段且当前用户为所有者），端到端权限复现记为残留风险 R13-R1。
- **残留 `.exists()`（有意未改，方向均 fail-closed 或极窄前提）**：`cutover.rs:442`（⇒ `Stale::DbMissing`）、`cutover.rs:852`（⇒ 不匹配 ⇒ 拒绝恢复）、`cutover.rs:1718`（⇒ `Err`）、`cutover.rs:1736/1935/2146`（原子写让位/清理判定）、`readiness.rs:1384-1385/1450`（备份清单命名/覆盖检查）；`exporter.rs`/`rollback.rs`/`lease.rs` 的导出回滚工具面另计。

### R13 门禁（本成员实跑）

| 命令 | 退出码 | 结果 |
|---|---|---|
| `cargo check -p storyforge-infra-sqlite --all-targets` | 0 | Finished（0 error） |
| `cargo fmt -p storyforge-infra-sqlite -- --check` | 0 | 无 diff |
| `cargo clippy -p storyforge-infra-sqlite --all-targets -- -D warnings` | 0 | Finished（0 warning） |
| `cargo test -p storyforge-infra-sqlite` | 0 | **24 target / 320 passed / 0 failed / 0 ignored**（§12 为 316，本轮 +4：3 单测 + 1 集成） |
| 定向三例（⑤ 组 / 反向锁 / N-R1-02） | 0 | 3 passed / 0 failed / 26 filtered |
| 负控 ×2（反转 Rule B、关闭 Rule A′） | 101（预期红） | 分别 1 例红（`:1484`、`:1179`）；还原后绿 |
| `cargo test -p storyforge --lib` | 0 | 477 passed / 0 failed / 3 ignored（工作树含其它成员并行改动） |
| `cargo test -p storyforge --test backend_parity_suite` | 0 | 2 passed / 0 failed |
| `cargo test -p storyforge --test sqlite_optin_lifecycle` | 0 | 1 passed / 0 failed |
| `cargo test -p storyforge --test sqlite_command_lifecycle` | 0 | 1 passed / 0 failed |
| `cargo test --workspace` / `npm test` / vitest | 未运行 | Lead 指令（不跑 workspace 级 cargo / 不跑 npm）+ 沙箱 `spawn EPERM`；引用 `fixes/GATE-REPORT.md` |

---

## 14 R12（W-11 下游 fail-open 裁定 + N-R7-04 交叉引用）

- **来源**：task-35（R12 收口，P2）；上游 W-11（`docs/review-2026-09-13/03-writing-pipeline.md:337-361` + `fixes/03-pipeline-fixes.md` §W-11 移交下游）与 N-R7-04（`round2/R7-cross-domain-seams.md:54,77`）。
- **详述**：`docs/review-2026-09-13/round2/R12-storage-failopen-closure.md`。
- **写入范围（实际）**：`crates/tauri-app/src/production_postprocess.rs` + 本文件 + R12 报告；`crates/infra-sqlite/**`、`storage_backend.rs` **零改动**；`commands/writing.rs`（孪生门禁）**不在写作用域，未改**。

### W-11 下游 `source_propagation_blocks` 三处 fail-open — **判定非问题 ×2 + 暂缓 ×1（全部升级为可观测）**

- **谓词与调用者**：共享门禁 `production_postprocess.rs:1183-1234`（整条 update 预检 `:1268`、逐 target `:1307-1317`），宿主 `build_knowledge_mutations`（`:1240`）被 `build_json_mutation_batch`（`:980`）与 `build_runtime_mutation_batch`（`:1403`）共用 ⇒ **默认 SQLite 后端与 JSON mutation 路径同一门禁**；孪生 `commands/writing.rs:1513-1565`（调用点 `:1319`、`:1347`）只在 JSON 直写路径生效（`backend_workflows.rs:1314-1318` 在 SQLite 权威时直接 `Err`）。`false` = **放行**（条目会被写入，`UpsertKnowledge` 在 `:1334`）。
- **F1 `source_character_id` 缺失 ⇒ 放行 = 判定非问题(附证据)**：广播的**文档形状**不带该字段——`app-agent/src/prompts/postprocess.rs:34-38` 示例 `{"broadcast":"all"}` 无 source，`:35` 明示 character_id 可为"任意角色名"，`:111` propagation 缺省 open；域注释仅标"ToldByOther 时填"（`domain/character_knowledge.rs:194-197`）。fail-closed 会让全部无源广播失效（功能回归）。
- **F2 源名字解析不出 ⇒ 放行 = 判定非问题(附证据)**：上游 W-11 降级方案明示只对**唯一命中**归一，"未命中或同名多实例保留原值（不猜），由下游解析兜底"（`fixes/03-pipeline-fixes.md` §W-11；实现 `app-agent/src/postprocess.rs:145-151`）⇒ 解析不出是被设计的合法状态（删角色/临时角色/旁白/同名多实例）。残留：落库条目 `source_character_id = None`（`:1285-1286` 同一 resolve）⇒ 溯源丢失（R12-R2）。
- **F3 源可解析但无同文本条目（策略未知）⇒ 放行 = 暂缓(附理由+建议)**：判据完全依赖源侧已落库条目 + 文本匹配（`source_entry_for:1107-1123`；`knowledge_text_matches`（`writing.rs:1567-1579`）归一化全等或 min_len ≥ 8 的包含）。这是三处里唯一直接对应"private 事实洗白外传"的面，但 `told_by_other` 的常见合法形态正是"源条目不存在/措辞不同" ⇒ fail-closed 会静默丢掉合法"被告知"写入（正文与知识库分歧）。**建议**：域1 增 `source_entry_id`（稳定标识）+ 域3 归一阶段回填，域2 按 id 直查策略、文本匹配降级为兜底（跨域，单独立任务）。
- **本轮落地（行为不变）**：`PropagationGate{blocked, unresolved}`（`:1125-1169`）+ 原因常量（`:1172/1174/1176`）；三处 fail-open 返回 `allow_unresolved(...)`；整条预检在 `unresolved` 非空时 `tracing::warn!(target: "knowledge_propagation", reason, source, text, …)`（`:1268-1281`）——**此前完全静默**，现可按 reason 计数。`blocked` 取值与改造前逐分支一致（`:1212-1232`）。
- **回归锁**：`:1558`（三处 fail-open 必须继续放行 + 原因）、`:1606`（显式策略判定不变：Private 阻止 / Open 放行 / GroupRestricted 组内放行·全体阻止 / 非传播无 unresolved）、`:1657`（端到端：文档化广播形状仍写 2 条 `UpsertKnowledge`，若被 fail-closed 会变 0）。
- **开关建议**：若产品要"宁丢不漏"，做成显式配置（默认现状 + 严格模式只收紧 F2/F3，**F1 必须保持放行**），开关落在 `build_knowledge_mutations` 入参，不要被 `unresolved` 隐式驱动。
- **残留**：R12-R1（F1 残留泄漏面需域3 把 source 改广播必填）、R12-R2（溯源丢失）、R12-R3（孪生门禁未同步 warn，写作用域外）、R12-R4（F3 miss 率待 warn 度量）、R12-R5（`knowledge_text_matches` 阈值刻意未动）。

### N-R7-04 回执 — **交叉引用（已接通）**

- R3 §9 已给出完整调用链并判"已接通"：`round2/R3-tauri-frontend-recheck.md:225-268`（判据 A/B/C）；原始条目 `round2/R7-cross-domain-seams.md:54,77`；R13 §1.5 另记录同一启动链（`lib.rs:1229-1230` → `storage_backend.rs:1882/1916/1929` → `recover_or_verify:1938-1939`，无 JSON 回退）。
- **R12 新增信息**：本轮未触碰该链路（`storage_backend.rs` 零改动，`production_postprocess.rs` 只改知识传播门禁）⇒ R3 回执继续有效，不再重复论证。

### R12 门禁（本成员实跑）

| 命令 | 退出码 | 结果 |
|---|---|---|
| `rustfmt --edition 2024 --check crates/tauri-app/src/production_postprocess.rs` | 0 | 无 diff |
| `cargo check -p storyforge --lib` | 0 | Finished（0 error） |
| `cargo clippy -p storyforge --lib -- -D warnings` | 0 | Finished（0 warning / 0 error） |
| 定向新测 ×3（`propagation_gate` ×2 + `broadcast_without_source`） | 0 | 3 passed / 0 failed |
| `cargo test -p storyforge --lib` | 0 | **480 passed / 0 failed / 3 ignored**（§13 为 477 ⇒ 本轮 +3，工作树含其它成员并行改动） |
| `cargo test -p storyforge --test backend_parity_suite` | 0 | 2 passed / 0 failed |
| `cargo test -p storyforge --test sqlite_character_lifecycle` | 0 | 1 passed / 0 failed |
| `cargo test -p storyforge-infra-sqlite` | 未运行 | R12 未触碰 infra-sqlite（§13 实测 320 passed / 0 failed） |
| `cargo test --workspace` / `npm test` / vitest | 未运行 | Lead 指令 + 沙箱 `spawn EPERM`；引用 `fixes/GATE-REPORT.md` |


