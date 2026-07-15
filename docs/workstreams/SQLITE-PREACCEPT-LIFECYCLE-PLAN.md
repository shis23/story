# SQLite Pre-Accept Lifecycle 工作计划

> 分支：`codex/sqlite-preaccept-lifecycle`
> 基线：`8732e21`
> 类型：SQLite opt-in 生命周期补全；不改变默认 JSON 行为

## 目标

为 opt-in SQLite 后端补齐 Accept 之前的写作生命周期基础：AI 草稿、Attempt 中间态、auto-fix/postprocess 写回所需的数据模型、事务 API、恢复与故障注入。最终交付应可被后续 Tauri 适配器接入，且不会产生 JSON/SQLite 双权威或部分写入。

## 当前事实

- SQLite opt-in 已覆盖 cutover、Accept/recovery/barrier、Chronicle publication 和 reverse export。
- 仍有 `append_ai_draft`、Attempt 中间态及 autofix/postprocess 写回可能走 JSON store；完整生命周期尚未 SQLite 化。
- 这条线必须与 `codex/production-postprocess-service` 并行，因此不能抢改 Tauri 写作命令。

## 交付范围

1. 审计现有 JSON pre-accept 写入的状态转换和持久化字段，列出每一步的 SQLite 对应物、权威源和恢复规则。
2. 在 `infra-sqlite` 中增加类型化 repository/UoW API，至少能原子处理：
   - 创建或更新草稿与活动 Attempt；
   - 记录 draft hash、quality report、auto-fix 最终文本/版本；
   - 持久化 postprocess/Chronicle 发布前所需的候选状态或可靠 outbox；
   - 按 Turn/Attempt/campaign/conversation scope 查找活动状态与恢复信息。
3. 更新 SQLite migration，版本排序、幂等、升级/降级诊断和备份约束必须保持 fail-closed。
4. 提供适配接口和测试构造器，供后续 Tauri 线接入；接口不能要求调用者同时写 JSON。
5. 如需要 SQLite→JSON 导出，新增字段必须以明确的 supported/unsupported 分类导出，绝不静默丢失。

## 明确不做

- 不改 `crates/tauri-app/src` 的写作命令、启动选择器、`TurnLifecycleService` 或 production command wiring；这些由后处理服务线整合。
- 不切换默认 backend，不改环境变量默认值，不做双写。
- 不修改导入/导出兼容、GUI、Android、M5 runner、`HANDOFF.md` 或 `RELEASE-CHECKLIST.md`。

## 冲突隔离

- 本线只拥有 `crates/infra-sqlite/**`、必要的 domain 类型（若不可避免）和本线 RESULT。
- 若 `tauri-app` 接线成为必需条件，留下清晰 adapter contract 与 blocker，交由集成阶段处理；不要跨线直接改命令。
- 不要通过复制 JSON store 代码制造第二套状态机。

## TDD 与验收

先写失败测试。最低覆盖：

1. 每一类 pre-accept 状态转换的提交与 rollback 原子性。
2. 在每个持久化步骤故障注入后：无孤立 draft、无错误 active Attempt、无双权威、可预测恢复。
3. Campaign/conversation/Attempt scope 不匹配 fail closed。
4. auto-fix 终稿、`draft_hash`、quality report 的一致性及幂等 replay。
5. 并发读/写与 Windows 文件锁/reopen 情况；已有 cutover/marker 语义不能回归。
6. migration 非正/重复/乱序版本、半 schema、升级后的 reverse export 均 fail closed。

最低门禁：

```powershell
cargo fmt --all -- --check
cargo test -p storyforge-infra-sqlite
cargo clippy -p storyforge-infra-sqlite --all-targets -- -D warnings
git diff --check
```

如修改 domain，再补 `storyforge-domain` 测试和 Clippy。不得调用真实模型。

## 交付与提交

- 提交顺序建议：schema/红测 → repository/UoW → 故障注入/导出 → RESULT。
- 新增 `docs/workstreams/SQLITE-PREACCEPT-LIFECYCLE-RESULT.md`，明确“repository 就绪”与“已接 Tauri production command”之间的差异。
- 不 push、不 merge、不 rebase main；保留干净 worktree。
