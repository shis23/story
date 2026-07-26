# SQLite Production UoW Day Result

- 分支：`codex/sqlite-production-uow`
- 基线：`main@053847f`
- 工作目录：`C:\tmp\storyforge-sqlite`
- 日期：2026-07-13

## 结论

完成一个默认不启用、可独立合并的 SQLite 生产事务切片：`infra-sqlite` 现可在单个
`BEGIN IMMEDIATE` Unit of Work 内原子完成 Turn accept 的核心持久化，包括：

- Conversation active Draft → Final；
- `TurnRecord` / `TurnAttempt` → `Committed`（或 Turn `Degraded`）；
- 全部 `Mutation` 变体应用；
- Campaign revision CAS 与恰好一次递增；
- Chronicle A `RoundSummary` 幂等写入与 `chronicle_revision` 递增；
- `commit_id` 持久化账本幂等；
- 任一步失败时 Conversation、Turn、Attempt、Campaign、Mutation、Chronicle、账本全量回滚。

本线没有把 SQLite 接入 `tauri-app`，没有切换默认后端，也没有 JSON/SQLite 双写。

## API

新增 `crates/infra-sqlite/src/production.rs`：

- `SqliteProductionRepository::bootstrap_campaign`
- `save_turn`
- `get_campaign` / `get_conversation` / `get_turn` / `get_attempt`
- `get_instance` / `get_task` / `get_knowledge` / `list_summaries`
- `accept_turn`
- `AcceptTurnRequest` / `AcceptOutcome`
- 与现有 JSON 路径相同的稳定 SHA-256 `compute_draft_hash`

所有写 API 都要求调用方显式传入 `&mut Database`。没有隐藏的逐 Store 自动提交，
也没有触碰 JSON 文件，因此不会形成双真相源。

## accept 事务语义

`accept_turn` 在取得 `BEGIN IMMEDIATE` 写事务后执行：

1. 从结构化列和 `payload_json` 同时读取 Turn/Attempt，核对 id、Campaign/Conversation/Turn
   归属、状态、base revision、variant、draft hash、时间和父子 Attempt 集合；任一漂移 fail-closed。
2. 查询 `mutation_commits`；重放时校验 ledger 的 Campaign → Turn → Attempt 归属、revision、
   terminal status 与请求 fingerprint。同一合法请求返回 `AlreadyCommitted`，漂移返回冲突。
3. 校验 Turn/Attempt 均为 `AwaitingAcceptance`。
4. 校验请求 MutationBatch 与 Attempt 中已持久化的 batch 完全一致（忽略执行状态字段）。
5. 要求 batch **恰有一个** `FinalizeVariant`，且 id 同时匹配 Attempt 和 Conversation node。
6. 校验请求 hash、Attempt hash、当前 Conversation active content 的 SHA-256 三者一致。
7. 校验 Campaign payload/indexed revision 一致，且等于 Turn base revision 和 batch expected；
   target 必须是 expected + 1。
8. 顺序应用全部 Mutation：Campaign/instance 绝对值变量、knowledge、task、summary、
   variant Final、临时 instance。
   Knowledge 的 `campaign_id` 必须是当前 Campaign；Turn accept 的 summary 只能是 leaf
   Chronicle A（level=0、`covers` 空、`covered_by=None`、Campaign/Conversation/lineage 归属正确）。
9. 新 Chronicle A 首次插入时递增 `chronicle_revision`；相同 ID/相同语义 payload no-op，
   相同 ID/不同 payload 冲突。B/C 半发布显式拒绝。
10. Campaign revision 写为 target；Turn/Attempt 写终态，其他活动 Attempt → Superseded，
   batch status → Committed。
11. 写入 commit ledger 后提交事务。

同一 Campaign 的活动 Turn 创建也在 `BEGIN IMMEDIATE` 内检查；已有非终态 Turn 时拒绝第二个。
`save_turn` 还拒绝把已存在的 `attempt_id` 改挂到另一 Turn。

## schema / migration 变化

新增单向 migration：

- `V002__production_commit_ledger.sql`
- schema version：`1 -> 2`
- 新表：`mutation_commits`
  - `commit_id` 主键
  - `campaign_id` / `turn_id` / `attempt_id` 外键
  - expected/target revision
  - terminal status
  - 规范化请求 `payload_hash`
  - committed timestamp
- 新索引：`idx_mutation_commits_campaign(campaign_id, target_revision)`

没有修改 V1 业务表结构，也没有改写 V1 migration。测试
`existing_v1_database_upgrades_to_v2_without_losing_data` 从仅 V1 的数据库升级到 V2，确认
既有 card 数据保留且 ledger 表创建成功。

返修没有新增 V3，也没有改变现有 V2 表结构。migration runner 在取得
`BEGIN IMMEDIATE` 写锁后重新查询 `schema_migrations.version` 和 checksum：若另一连接已完成同一
migration，则当前连接幂等成功；若 checksum 不同则明确失败。这样避免两个冷库连接都在锁外观察到
“未应用”后竞争插入同一 version。

## 返修红测 → 绿测证据

返修严格先添加测试并运行旧实现：

- 首轮新增 production tests：**9 failed**，实际暴露缺失/重复 Finalize、结构列漂移、跨 Campaign
  knowledge、非 leaf Chronicle 等缺口；wrong Finalize 和错误 Conversation scope 在旧实现中已正确回滚。
- 公开 8 连接冷库测试在该次调度中偶然通过，因此增加确定性锁竞态：强制两个连接都在锁外观察
  version 缺失后同时 apply。旧实现稳定失败：
  `UNIQUE constraint failed: schema_migrations.version`。
- `save_turn_rejects_rehanging_an_existing_attempt_id` 旧实现返回 `Ok(())`，确认 Attempt 可被改挂。
- `accept_rejects_chronicle_a_with_wrong_lineage` 旧实现返回 `Applied`。

最小实现完成后，上述测试全部转绿；没有通过放宽断言或跳过测试规避失败。

## 回滚与幂等证据

`crates/infra-sqlite/tests/production_uow.rs` 共 25 个测试，另有 1 个冷库 migration integration test：

| 契约 | 测试证据 |
| --- | --- |
| 完整 accept 原子提交 | `accept_commits_turn_campaign_chronicle_and_variant_atomically` |
| 全部 Mutation 变体 | `accept_supports_every_mutation_variant_in_one_transaction` |
| mutation 后故障回滚 | `injected_failure_rolls_back_every_accept_side_effect` |
| Turn final 后、ledger 前故障回滚 | `failure_after_turn_finalization_still_rolls_back_everything` |
| commit_id 重放幂等 | `repeated_commit_id_is_idempotent_without_revision_bump` |
| 持久化 batch 防篡改 | `request_batch_must_match_the_persisted_attempt_batch` |
| draft 编辑使 hash 失效 | `edited_conversation_content_invalidates_the_persisted_draft_hash` |
| revision CAS 冲突零副作用 | `revision_conflict_has_no_side_effects` |
| 双连接竞争同一 accept | `concurrent_accepts_serialize_to_one_apply_and_one_replay` |
| 单 Campaign 单活动 Turn | `only_one_active_turn_per_campaign_is_allowed` |
| Turn/Attempt 类型 round-trip | `turn_and_attempt_domain_payloads_round_trip` |
| Finalize 恰好一次 | `accept_rejects_batch_without_finalize_variant_with_zero_side_effects` / `accept_rejects_duplicate_finalize_variant_with_zero_side_effects` / `accept_rejects_wrong_finalize_variant_with_zero_side_effects` |
| Turn 结构列/payload 漂移 | `get_turn_rejects_payload_id_drift_instead_of_rehanging_record` / `get_turn_rejects_structured_scope_status_and_base_revision_drift` |
| Attempt 结构列/归属漂移 | `get_attempt_rejects_payload_id_drift` / `get_attempt_and_parent_turn_reject_structured_ownership_and_field_drift` |
| Attempt 禁止重挂 | `save_turn_rejects_rehanging_an_existing_attempt_id` |
| ledger 归属链校验 | `ledger_replay_rejects_campaign_turn_attempt_ownership_drift` |
| Knowledge Campaign 隔离 | `accept_rejects_knowledge_owned_by_another_existing_campaign` |
| Turn 只发布 leaf Chronicle A | `accept_rejects_chronicle_b_in_turn_batch` / `accept_rejects_leaf_summary_with_covers_or_covered_by` |
| Chronicle 来源归属 | `accept_rejects_chronicle_a_with_wrong_source_scope` / `accept_rejects_chronicle_a_with_wrong_lineage` |
| 冷库多连接 migration | `concurrent_first_start_migrations_are_idempotent` / `concurrent_apply_rechecks_version_after_acquiring_write_lock` |

双连接竞争结果严格为一个 `Applied`、一个 `AlreadyCommitted`，最终 Campaign revision 和
chronicle revision 均只递增一次，证明 `BEGIN IMMEDIATE` + ledger 在当前单文件/多连接模型下
可序列化同一 accept。SQLite 连接仍使用 ADR 既定 `busy_timeout=5000`。

两处故障注入分别位于 mutation 应用后、以及 Turn/Attempt/Conversation/Campaign 已写但 ledger
尚未写入时；测试均确认事务后所有业务表和 ledger 恢复到 accept 前状态。

## 门禁结果

仅运行要求范围内的门禁，未跑全 workspace：

| 命令 | 结果 |
| --- | --- |
| `cargo fmt -p storyforge-infra-sqlite -p storyforge-domain -- --check` | PASS |
| `cargo test -p storyforge-domain` | PASS：243 passed |
| `cargo test -p storyforge-infra-sqlite` | PASS：20 unit + 26 integration passed |
| `cargo clippy -p storyforge-domain -p storyforge-infra-sqlite --all-targets -- -D warnings` | PASS |
| `git diff --check 053847f..HEAD` | PASS |

## 默认行为保持

- 未修改 `tauri-app`、JSON Store、app 启动或 storage selector。
- 未新增 `tauri-app -> storyforge-infra-sqlite` 依赖。
- 未新增或修改 `storage.backend` 配置；默认生产后端仍为 JSON。
- 未读取后回写、删除或迁移用户 JSON；SQLite API 只操作显式传入的数据库。
- 未实现 JSON/SQLite 双写。

因此当前合并只提供 production-ready repository/UoW 原语，不改变任何用户可见的默认路径。

## 范围收缩与后续接线

Day Plan 原拟同时实现 Chronicle B/C publication UoW。代码审查后为保持切片可审查性，本提交优先
完成 Turn accept + Chronicle A，并将 B/C 留作后续独立切片。当前 SQLite schema 已能保存
`round_summaries` / `round_summary_covers`，但尚无类型化 B/C publication API 去原子完成：

- parent B/C upsert；
- child `covered_by` 与 parent `covers` 双向校验；
- `chronicle_revision` 幂等递增；
- `pending_compress_publication` 清理/兼容导入语义。

后续 app 接线也应作为独立变更：显式 backend selector、一次性迁移/备份/反向恢复、SQLite
Conversation/Turn/Campaign store 选择，以及真实启动恢复流程。禁止通过双写过渡。

## 风险与未宣称事项

1. **Android 未验证**：只沿用 `rusqlite bundled` 选择；没有 Android target 编译、真机路径或锁测试，
   不宣称 Android 可用。
2. **生产 app 未接线**：JSON accept 仍使用跨文件 journal/replay；本 UoW 只有显式 SQLite 调用者使用。
3. **备份/反向迁移未实现**：默认切换前仍需 checkpoint backup、SQLite → JSON/导出或其他可操作回退。
4. **B/C publication 未实现**：见上节，不能据此宣称 Chronicle 全链路已 SQLite 化。
5. **多进程长期压力未验证**：测试覆盖两个连接的竞争 accept；ADR 的单 app 实例假设保持不变。
6. **Importer 早期诊断仍可增强**：production repository 的读取和 `save_turn` 已对 Attempt 归属
   fail-closed；一次性 JSON importer 对重复 Attempt owner 的更早错误提示可作为迁移工具后续增强项。

## Commit 列表

| Commit | 说明 |
| --- | --- |
| `64c9251` | docs(workstream): plan SQLite production UoW slice |
| `124c3b7` | feat(infra-sqlite): transactional production turn accept repository |
| `089daac` | docs(workstream): record SQLite production UoW result |
| `31332db` | fix(infra-sqlite): harden production UoW invariants |
| （本次 RESULT 更新） | docs(workstream): update SQLite production UoW hardening evidence |

## 建议合并范围

建议合并本线全部 commit，作为未启用的 SQLite production UoW 基础。不得在同一合并动作中顺带：

- 切换默认 backend；
- 接线 `tauri-app`；
- 增加 JSON/SQLite 双写；
- 宣称 Android 或完整 Chronicle B/C 已验证。
