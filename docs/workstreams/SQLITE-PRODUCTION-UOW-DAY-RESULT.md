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

1. 查询 `mutation_commits`，同一 `commit_id` + 同一规范化请求返回
   `AlreadyCommitted`；同 ID 不同 payload 返回冲突。
2. 校验 Turn/Attempt 均为 `AwaitingAcceptance`。
3. 校验请求 MutationBatch 与 Attempt 中已持久化的 batch 完全一致（忽略执行状态字段）。
4. 校验请求 hash、Attempt hash、当前 Conversation active content 的 SHA-256 三者一致。
5. 校验 Campaign payload/indexed revision 一致，且等于 Turn base revision 和 batch expected；
   target 必须是 expected + 1。
6. 顺序应用全部 Mutation：Campaign/instance 绝对值变量、knowledge、task、summary、
   variant Final、临时 instance。
7. 新 Chronicle A 首次插入时递增 `chronicle_revision`；相同 ID/相同语义 payload no-op，
   相同 ID/不同 payload 冲突。
8. Campaign revision 写为 target；Turn/Attempt 写终态，其他活动 Attempt → Superseded，
   batch status → Committed。
9. 写入 commit ledger 后提交事务。

同一 Campaign 的活动 Turn 创建也在 `BEGIN IMMEDIATE` 内检查；已有非终态 Turn 时拒绝第二个。

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

## 回滚与幂等证据

`crates/infra-sqlite/tests/production_uow.rs` 共 11 个测试：

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
| `cargo test -p storyforge-infra-sqlite` | PASS：19 unit + 11 integration passed |
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

## Commit 列表

| Commit | 说明 |
| --- | --- |
| `64c9251` | docs(workstream): plan SQLite production UoW slice |
| `124c3b7` | feat(infra-sqlite): transactional production turn accept repository |
| （本 RESULT） | docs(workstream): record SQLite production UoW result |

## 建议合并范围

建议合并本线全部 commit，作为未启用的 SQLite production UoW 基础。不得在同一合并动作中顺带：

- 切换默认 backend；
- 接线 `tauri-app`；
- 增加 JSON/SQLite 双写；
- 宣称 Android 或完整 Chronicle B/C 已验证。
