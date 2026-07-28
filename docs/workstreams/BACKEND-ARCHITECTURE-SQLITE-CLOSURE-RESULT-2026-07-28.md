# 后端架构拆分与 SQLite 收口：Gate 1 验收结果（2026-07-28）

> 状态：**Gate 1 PASS（最终边界返修已完成）**；Gate 2–8（backend facade、SQLite 迁移、平台验收和发布封存）尚未完成。
>
> code-under-test：`main@d340d99`。
>
> document HEAD：本文件所在文档提交；不把文档提交当作被测代码。

## 1. Gate 1 交付物

- 根模块 `crates/tauri-app/src/lib.rs` 从 2,455 行降至 1,320 行。
- 具体命令实现已归位到 `commands/connections.rs`、`conversations.rs`、`turns.rs`、`writing.rs` 及既有域模块。
- 根测试文件 `lib_tests.rs` 只保留模块声明、公共夹具和导入；域测试拆到：
  - `lib_tests_campaigns.rs`
  - `lib_tests_connections.rs`
  - `lib_tests_conversations.rs`
  - `lib_tests_diagnostics.rs`
  - `lib_tests_import_export.rs`
  - `lib_tests_meta.rs`
  - `lib_tests_startup.rs`
  - `lib_tests_turns.rs`
  - `lib_tests_writing.rs`
  - `lib_tests_writing_regenerate.rs`
- 新增完整注册命令快照：`frontend/tests/fixtures/tauri-registered-commands.snapshot.json`。
- 架构测试升级为四层门禁：命令全集、实现不得回流根模块、根测试模块零测试属性、域文件确实拥有可执行测试。

## 2. 基线

| 指标 | 结果 |
| --- | ---: |
| workspace crate | 16 |
| `lib.rs` 行数 | 1,320 |
| Tauri command 属性 | 175 |
| 注册命令 | 175 |
| 前端唯一 invoke | 162 |
| 前端缺失后端命令 | 0 |
| SQLite active flag references | 68 |

## 3. 验证证据

- `cargo fmt --all -- --check`：通过。
- `cargo check -p storyforge --all-targets --no-default-features`：通过。
- `cargo clippy -p storyforge --all-targets --no-default-features -- -D warnings`：通过。
- `cargo test -p storyforge --no-default-features`：351 passed、3 ignored、0 failed。
- `node --test frontend/tests/tauri-command-contract.test.mjs`：8 passed、0 failed。
- `node scripts/architecture/backend-baseline.mjs`：175/175 注册一致，前端缺失 0。
- `git diff --check`：通过。

## 4. Gate 1 结论

Gate 1 可以正式收口。此前审查指出的两个边界问题已处理：

1. `lib.rs` 不再承载 regenerate、连接配置、会话归档、回合小票、后处理重试和质量查询等具体业务实现。
2. 根测试不再是单体搬家；测试按域文件归位，并由架构合同测试持续约束。

## 5. 未完成范围

Gate 1 PASS 不等于项目总体 PASS。以下仍属于后续 Gate：

- Gate 2：统一 JSON/SQLite 业务状态机与写作后处理。
- Gate 3：单一 backend facade。
- Gate 4–5：SQLite 能力补齐、迁移/反向导出、等价性和故障注入。
- Gate 6：Windows、Android、真机和发布证据。
- Gate 7–8：默认切换、兼容回退、文档和发布封存。

## 6. Gate 2 进行中检查点（2026-07-28）

> 状态：**Gate 2 PARTIAL（进行中）**。已完成 Accept 决策核心与 draft-hash 统一；写作阶段、postprocess builder、typed patch、tool loop 统一仍未完成。

### 6.1 已完成批次

| 批次 | 提交 | 内容 |
| --- | --- | --- |
| 文档恢复 | `072fc53` | 恢复完整 635 行 Gate 0–8 计划（用户修改，单独提交）。 |
| Batch 2.1 | `ca048ae` | `compute_draft_hash` 提升为 `crates/domain/src/turn.rs` 单一权威纯函数；JSON（`turn_lifecycle.rs`）与 SQLite（`infra-sqlite/production.rs`）改为 re-export。新增 known-vector 测试证明 hex 输出与旧实现逐字节一致。 |
| Batch 2.2 | `71e7884` | 抽取 backend-agnostic 的 `evaluate_accept_decision` 纯函数（`AcceptDecision` enum + `AcceptDecisionInput`），承载完整决策序言：scope 守卫、幂等终态重放、attempt 状态、derivation、quality gate、draft-hash、revision、batch/terminal-status 准备。JSON `TurnLifecycleService::accept_by_variant` 与 SQLite `sqlite_runtime::accept_by_variant` 均改为先调用此函数，再各自执行后端特有的持久化（JSON 多步 CAS / SQLite 原子 UoW）。新增 7 个纯决策 parity 测试。 |

### 6.2 决策核心统一的边界

- 纯决策（`evaluate_accept_decision`）只读已加载的 `TurnRecord`/`TurnAttempt`/draft 文本/campaign revision，不接触任何存储。
- 后端差异只落在持久化：JSON 保留多步 CAS（AwaitingAcceptance → Committing → apply MutationBatch → terminal mark，predicate miss 仍产生 `AcceptError::CasFailed`）；SQLite 保留原子 `SqliteProductionRepository::accept_turn` UoW（revision/cas 失败仍映射为 typed `RevisionConflict`/`Commit`）。
- 幂等终态重放统一为 `AcceptDecision::Replay`；各后端只负责重新确认盘上持久性。
- 未削弱 revision CAS、fail-closed、`AcceptError` 分类或 active-turn barrier。
- 未改变任何 Tauri command 名称、参数、DTO、事件或前端 IPC 合同。

### 6.3 验证证据（Batch 2.1 + 2.2）

- `cargo fmt --all -- --check`：通过。
- `cargo check -p storyforge --all-targets --no-default-features`：通过。
- `cargo clippy -p storyforge --all-targets --no-default-features -- -D warnings`：通过。
- `cargo test -p storyforge --no-default-features`：358 passed（基线 351 + 7 新 parity 测试）、3 ignored、0 failed。
- `node --test frontend/tests/tauri-command-contract.test.mjs`：8 passed、0 failed。
- `node scripts/architecture/backend-baseline.mjs`：175/175 注册一致，前端缺失 0。
- `git diff --check`：通过。
- code-under-test（本检查点）：`main@71e7884`。

### 6.4 尚未完成（Gate 2 剩余，按计划 §7）

- **Batch 2.3（postprocess mutation builder）**：`build_json_mutation_batch`（`production_postprocess.rs:928-1080`）与 `build_runtime_mutation_batch`（`:1086-1334`）仍为两套并行实现；角色解析/同名冲突/传播策略/在场豁免/临时角色/变量归属在 `writing.rs:1581-1993` 与 `production_postprocess.rs:1093-1176` 各写一遍；JSON 路径丢弃已组装的 `CampaignRuntimeContext`（`runtime_support.rs:74-77`）；Chronicle A seq 的 JSON（扫描存储）vs SQLite（turn 号）分歧未消除。
- **Batch 2.4（typed patch preview/apply 纯函数）**：Preview 读取 propose 时预存的 `patch.diff`（`meta_typed.rs:222`），Accept 忽略 diff 并通过 `apply_to_snapshot` + `validate_typed_patch_targets` + `apply_typed_action` 三处重新推导（`meta_typed.rs:307-353/379-463/509-687`）；一个 `TypedPatchAction` 的语义仍写在四个地方。
- **Batch 2.5（tool loop 合并）**：`run_tool_loop`/`run_tool_loop_streaming`/`run_tool_loop_with_layout`（`app-agent/runtime.rs:282/420/573`）仍为三个近似实现。
- **Batch 2.6（writing 阶段复用）**：start_writing 的 Director/Subagent/Editor 阶段（`app-pipeline/lib.rs:822-1242`）与 regenerate（`:2105-2346` + `run_editor_and_commit:2906`）仍各写一遍；`EditorStarted` 双发未消除。
- **Batch 2.7（取消/失败/重试事件发射）**：`SubagentCancelled` 仍混淆真取消与 LLM 失败；`run_shared_postprocess_background` 的 cancel skip 仍重发为 `PostProcessFailed`。
- **Batch 2.8（RESULT 收口）**：待上述批次完成后追加最终 Gate 2 结论。

### 6.5 Gate 2 通过条件核对（计划 §7.5）

- [x] ~~JSON/SQLite Accept 对照测试使用同一输入得到同一领域结果和同类错误~~ — Batch 2.2 已通过纯决策 parity 测试证明（7 个场景）。
- [ ] start/regenerate 不再复制完整阶段实现 — Batch 2.6 待完成。
- [ ] tool loop 只有一个权威实现 — Batch 2.5 待完成。
- [ ] postprocess mutation 规则只有一个权威实现 — Batch 2.3 待完成。
- [ ] typed patch preview 与 apply 使用同一纯函数 — Batch 2.4 待完成。

### 6.6 剩余风险

- Batch 2.5/2.6 触及 7,868 行的 `app-pipeline/lib.rs`，改动面大；须纯抽取+委托、mock-LLM 集成测试护栏，不得改行为。
- Batch 2.3/2.4 触及 postprocess 与 typed patch 的数据写入路径，须先补 parity 测试再迁移，避免削弱字段一致性。
- 真实模型证据（Gate 6）不在此阶段运行；deterministic mock-LLM 测试足够验证 Gate 2。

