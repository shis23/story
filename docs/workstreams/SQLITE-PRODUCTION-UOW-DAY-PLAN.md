# SQLite Production UoW Day Plan

- 分支：`codex/sqlite-production-uow`
- 基线：`main@053847f`
- 工作目录：`C:\tmp\storyforge-sqlite`
- 日期：2026-07-13

## 目标切片

交付一个可独立合并、默认不启用的 SQLite 生产事务适配层：在单个
`BEGIN IMMEDIATE` Unit of Work 内完成一次 Turn accept 的核心持久化，覆盖：

1. 类型化 `TurnRecord` / `TurnAttempt` 保存与读取；
2. `AwaitingAcceptance -> Committing -> Committed/Degraded` 状态校验；
3. `MutationBatch` 的稳定 `commit_id` 幂等应用；
4. Campaign `revision` 的 compare-and-swap 与恰好一次递增；
5. Chronicle A `RoundSummary` upsert 与 `chronicle_revision` 恰好一次递增；
6. 任一步骤失败时 Turn、Attempt、Campaign、Mutation 与 Chronicle 全量回滚。

本切片同时补一个独立的 Chronicle B/C publication 事务 API，在同一事务内写入
parent summaries、child `covered_by` / `covers`、递增 `chronicle_revision`、清理
`pending_compress_publication`。SQLite 路径不需要复制 JSON 半提交协议，但读取和
幂等语义必须与现有 JSON heal 结果一致。

## 现有语义基线

- JSON TurnStore 在任何 Campaign 副作用前将最终 `MutationBatch` 与 Turn/Attempt
  `Committing` 状态原子写入 `turns.json`，并以 CAS 阻止并发 accept/postprocess 覆盖。
- accept 只允许活动 `AwaitingAcceptance` Attempt；校验 Campaign revision、
  `variant_id + draft_hash`，成功后设置 `accepted_attempt_id`，其他活动 Attempt
  变为 `Superseded`。
- `MutationBatch` 使用绝对值更新和预分配稳定 ID；同一 revision 的重放不重复 bump，
  payload 不一致必须冲突。
- Chronicle A 首次插入递增 `chronicle_revision`；重复相同 summary 为 no-op，
  相同 ID 不同 payload 为冲突。
- JSON B/C 发布用 `pending_compress_publication` 跨文件恢复；完成态要求 parent 存在、
  child 指向对应 parent、revision 已推进、epoch/marker 已清理。

## TDD 执行顺序

1. 先添加失败的 repository/UoW contract 测试：类型 round-trip、accept happy path、
   mutation 载荷、Chronicle A、全量回滚、重复 commit、revision conflict、并发 writer。
2. 实现最小类型化 repository 与 accept UoW，使测试转绿。
3. 添加失败的 Chronicle publication 测试：成功、重复 publication、缺 child/parent、
   payload 冲突、故障注入回滚。
4. 实现 publication UoW，并重构共享的 payload/upsert/CAS 辅助。
5. 仅运行 `infra-sqlite` / `domain` 相关 fmt、test、clippy 与 diff-check。

## API 与数据约束

- 适配层位于 `crates/infra-sqlite`；直接使用 `storyforge-domain` 类型，不修改默认 app
  启动或 `tauri-app` accept 路径。
- repository API 必须要求显式 `&mut Database` / Unit of Work，不提供逐 Store 自动提交。
- `turns.payload_json` 保存完整 `TurnRecord`，`turn_attempts.payload_json` 保存完整
  `TurnAttempt`；结构化列用于状态/CAS/索引，读取时校验父子一致性。
- Campaign 与 mutation 目标继续复用 v1 表；若无法可靠记录 `commit_id` 幂等性，新增
  单向 V002 migration 和提交账本表，禁止用内存状态冒充持久化幂等。
- 同一 Campaign 只允许一个非终态 Turn；数据库约束或事务内查询必须阻止并发创建。
- 锁语义以 `BEGIN IMMEDIATE` + `busy_timeout=5000` 为准：并发 writer 要么串行成功，
  要么得到明确 busy/timeout 错误，不得半提交或丢失 revision 更新。

## 验收测试

- TurnRecord/Attempt 完整 domain round-trip，包括 `pending_state_changes`、quality、临时实例、
  provenance 与全部状态字段。
- accept 首次成功：MutationBatch 全部可见，Campaign revision `expected -> target`，
  Chronicle A 首次插入时 `chronicle_revision + 1`，Turn/Attempt 终态正确。
- accept 重复调用：同一 `commit_id` 返回幂等成功，revision/chronicle revision 不再递增。
- revision、draft hash、Attempt 状态、mutation payload 任一冲突均无任何持久化副作用。
- 故障注入发生在 mutation/summary/Turn finalization 中间时，所有表恢复到事务前状态。
- 两个连接竞争同一 Campaign 时，不出现两个成功的同 revision accept；锁释放后可恢复。
- Chronicle publication 重放不重复 bump；缺失 child、parent payload 冲突和中途失败均回滚，
  成功后 marker 清空且覆盖关系完整。

## 禁止事项与非目标

- 不修改 `storage.backend` 默认值，不把 SQLite 接入默认启动路径。
- 不做 JSON/SQLite 双写，不删除、移动或改写现有 JSON 数据。
- 不修改 `tauri-app`；如发现接线需要 app API，记录到 RESULT 的后续工作。
- 不宣称 Android 可用；本线不做 Android target 编译或真机验证。
- 不跑全 workspace 门禁，不触碰 release/M5/Phase B 证据线。

## 本线门禁

- `cargo fmt -p storyforge-infra-sqlite -p storyforge-domain -- --check`
- `cargo test -p storyforge-domain`
- `cargo test -p storyforge-infra-sqlite`
- `cargo clippy -p storyforge-domain -p storyforge-infra-sqlite --all-targets -- -D warnings`
- `git diff --check 053847f..HEAD`

## 结果文档

完成后新增 `docs/workstreams/SQLITE-PRODUCTION-UOW-DAY-RESULT.md`，记录：

- repository/UoW API 与未接线边界；
- schema/migration 变化；
- 回滚、幂等、revision CAS、锁竞争的测试证据；
- JSON 默认行为保持情况；
- Android、备份/反向迁移和 app 接线剩余风险。
