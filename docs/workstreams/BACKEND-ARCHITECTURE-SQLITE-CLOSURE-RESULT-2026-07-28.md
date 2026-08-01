# 后端架构拆分与 SQLite 收口：执行结果（2026-07-28 起）

> 状态：**Gate 1 PASS；Gate 2 PASS；Gate 3 PASS；Gate 4 PASS（2026-07-31）**；Gate 5–8 尚未完成。
>
> code-under-test：`main@a2e8d7e` 加 Gate 3 完成提交（未 push；SHA 以 git log 为准）。
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

## 7. Gate 2 Batch 2.3 检查点（postprocess mutation builder 统一）

### 7.1 范围与策略

Batch 2.3 消除 JSON 与 SQLite 两条 postprocess mutation 构建路径的重复实现。

**改动前**：`build_json_mutation_batch`（`production_postprocess.rs:928-1080`）与 `build_runtime_mutation_batch`（`:1086-1334`）各自维护一套角色解析、同名冲突、在场豁免、广播分发、传播策略、变量归属与任务段逻辑；JSON 路径委托 `writing.rs:1581-1993` 的 store-bound 辅助，SQLite 路径用闭包内联实现等价逻辑。

**改动后**：抽取 9 个共享纯函数，JSON 与 SQLite 共用同一组 knowledge / variable / task 段构建：

- `merge_temporary_instances(runtime, temps)` — 快照实例 + attempt 临时实例去重合并。
- `compute_name_collisions(instances)` — ≥2 次出现的 name 集合（同名时 name 路失效）。
- `resolve_instance_by_id_or_name(instances, value)` — id 优先、name 兜底。
- `instance_matches_group(runtime, instance, group)` — definition_id → definitions_by_id 反查 group。
- `source_entry_for(runtime, source_id, text)` — runtime.knowledge 中最新匹配条目。
- `source_propagation_blocks(update, target, resolve, group_member, source_lookup)` — Open/Private/GroupRestricted 传播策略判定（纯函数，数据源无关）。
- `build_knowledge_mutations(...)` — 广播分发 + 单目标 + 在场/同名/传播收紧。
- `build_variable_mutations(...)` — 角色级 name/id 解析 + 在场收紧；全局级无约束。
- `build_task_mutations(...)` — 已有任务状态更新 + 新建任务。
- `build_summary_mutation(persist_ctx, summary, a_seq, lineage)` — Chronicle A 摘要组装（seq 由调用方决定）。

**JSON 路径**（`build_json_mutation_batch`）改为：把 `store` 当前状态投影成 `CampaignRuntimeContext`（`project_store_runtime_context`），再委托上述共享段；revision 基线与 Chronicle A seq 仍从 store 实时读取（CAS 与 code 间隙容忍需要）。

**SQLite 路径**（`build_runtime_mutation_batch`）改为：直接复用共享段，闭包提升为命名函数。

### 7.2 保留的有意差异（不可削弱）

- **Chronicle A seq**：JSON 扫描 `store.list_summaries` 取 `max(parsed A-seq, A.turn) + 1`（容忍 code 间隙/重排）；SQLite 用 `persist_ctx.turn`（runtime 快照无活动摘要索引）。这是既有有意行为，非缺陷；统一不要求抹平。
- **revision 基线**：JSON 从 `store.get_campaign` 实时读（CAS 写入需要当前值）；SQLite 从 `runtime.campaign.revision` 读（快照已包含）。
- **非 Campaign 旧路径**：`persist_postprocess_outcome_to_store`（`runtime_support.rs:1944`，identity=None 的 fallback）仍用 `writing.rs` 的 store-bound 辅助；该路径不在本批次收敛范围（计划 §7.3 只收敛 ProductionPostprocessService 内部）。

### 7.3 新增 parity 测试

- `json_and_runtime_builders_produce_equivalent_mutations`（`production_postprocess.rs`）：用同一 JSON store 建立活动状态，分别走 JSON builder 与投影成 runtime 快照后走 SQLite builder，断言两者产生的 mutation 签名序列（剔除非确定 entry_id/commit_id）逐条相等；覆盖 Group 广播、变量更新、新建任务、revision 基线。当前 PASS，证明除 Chronicle A seq 外两条路径已字段等价。

### 7.4 验证证据

- `cargo fmt --all -- --check`：通过。
- `cargo check -p storyforge --all-targets`：通过。
- `cargo clippy -p storyforge --all-targets -- -D warnings`：通过。
- `cargo test -p storyforge --no-default-features`：359 passed（Batch 2.2 后 358 + 1 新 parity 测试）、3 ignored、0 failed。
- `production_postprocess::tests` 全 17 项通过，含既有 `pending_temporary_instances_resolve_in_json_batch`（V2 临时角色解析回归）与 `pending_temporary_instances_resolve_in_runtime_batch`。
- `node --test frontend/tests/tauri-command-contract.test.mjs`：8 passed、0 failed。
- `node scripts/architecture/backend-baseline.mjs`：175/175 注册一致，前端缺失 0，sqlite activeFlagReferences 68 不变。
- `git diff --check`：通过（仅 LF→CRLF 行尾提示，无空白错误）。

### 7.5 未削弱项核对

- [x] 角色解析、同名冲突、在场豁免、广播分发、传播策略、变量归属：JSON 与 SQLite 现共用同一组纯函数，字段语义一致。
- [x] Chronicle A seq 的 JSON 扫描 vs SQLite turn 号差异：保留，未抹平。
- [x] revision CAS 基线：JSON 仍从 store 实时读，SQLite 从快照读；未削弱。
- [x] V2 临时角色解析：JSON 路径投影后仍经 `merge_temporary_instances` 纳入解析域（回归测试通过）。
- [x] 命令名/参数/DTO/事件/前端 IPC 合同：未改动（175/175 不变）。

### 7.6 性能侧效（正向）

- JSON 路径原先每次 group 判定都遍历 `store.list_cards()`（O(cards × defs) per broadcast target）；现改为投影时一次性构建 `definitions_by_id` 哈希表，后续 group 判定为 O(1) 查表。多目标广播场景下净优化。

### 7.7 code-under-test

本检查点提交后记录具体 SHA。

### 7.8 Gate 2 剩余（按计划 §7）

- **Batch 2.4（typed patch preview/apply 纯函数）**：Preview 读 propose 预存 diff，Accept 三处重新推导；待统一。
- **Batch 2.5（tool loop 合并）**：三个近似 `run_tool_loop*` 待合并为单一可配置执行器。
- **Batch 2.6（writing 阶段复用）**：start_writing / regenerate 的 Director/Subagent/Editor 仍各写一遍；`EditorStarted` 双发待消除。
- **Batch 2.7（取消/失败事件发射）**：`SubagentCancelled` 混淆真取消与 LLM 失败；cancel skip 重发为 `PostProcessFailed`。
- **Batch 2.8（RESULT 收口）**：待上述批次完成后追加最终 Gate 2 结论。

## 8. Gate 2 Batch 2.4 检查点（typed patch preview/apply 纯函数统一）

### 8.1 范围与策略

Batch 2.4 消除 typed patch 的 Preview 与 Accept 两套前置条件判定逻辑。

**改动前**：
- `meta_typed::validate_typed_patch_targets`（`meta_typed.rs:379-463`）自带一套 target 存在性 + SyncInstanceVariables definition/schema 校验；
- `app_meta::is_patch_stale`（`typed_patch.rs:139-164`）另有一套 target 存在性扫描；
- `app_meta::apply_to_snapshot`（`typed_patch.rs:167`）的 `apply_action` 在 target 缺失时返回 `TargetMissing`（第三处存在性判定）；
- Preview（`meta_preview_typed_patch`）只调 `is_patch_stale`，Accept 调 `is_patch_stale` + `validate_typed_patch_targets` + `apply_to_snapshot`，三者判定面不一致（Preview 通过但 Accept 失败的窗口）。

**改动后**：新增 `app_meta::validate_patch_preconditions(&TypedPatch, &PreviewInput) -> Result<(), TypedPatchError>` 作为单一权威前置条件纯函数：
- target 存在性复用私有 `action_target_missing`（`is_patch_stale` 与本函数共用，存在性扫描只此一份）；
- `SyncInstanceVariables` 的 definition_id 匹配、definition 存在、add_keys 在 schema 内的校验归并于此；
- `RepointInstanceDefinition` 的 `new_definition_id` 存在性校验归并于此；
- 缺失 target 返回 `TargetMissing`，definition/schema 不匹配返回新增的 `PreconditionFailed`。

`meta_typed::validate_typed_patch_targets` 改为委托该纯函数（映射 `TypedPatchError → TauriCommandError`），删除重复的存在性/前置条件循环及不再使用的 `find_instance`/`ensure_instance_exists`/`ensure_definition_exists`/`ensure_task_exists` 辅助。

`meta_preview_typed_patch` 也改调 `validate_patch_preconditions`：Preview 与 Accept 现共用同一前置判定，Preview 看到的可接受性与 Accept 一致（不再出现「Preview 通过但 Accept 失败」）。

### 8.2 保留的后端差异（不可削弱）

- 写盘机制：`apply_typed_action`（`meta_typed.rs:509`）仍是 store-bound 写盘辅助；SQLite 活跃时 typed patch accept 仍由 `ensure_typed_patch_backend_supported` 拒绝（Gate 4 才补齐 SQLite Meta UoW）。本批次只统一**前置条件判定**，不动写盘路径。
- `apply_to_snapshot`（纯函数预演）仍由 Accept 在写盘前调用（`meta_typed.rs:319-337`），作为 dry-run 保护；它与 `validate_patch_preconditions` 互补：前者验语义可应用，后者验 target/definition 前置。

### 8.3 新增 parity/契约测试

- `typed_patch_preconditions_share_one_pure_function_between_preview_and_accept`（`lib_tests_meta.rs`）：钉住三点——target 存在 + definition 匹配 + schema 含 add_key 时通过；definition_id 不匹配被纯函数拒绝；target instance 缺失被纯函数拒绝。
- 既有 `test_meta_accept_typed_patch_preflights_all_actions_before_writing`（验证 RepointInstanceDefinition 的 `new_definition_id` 缺失在写盘前失败）继续通过，证明归并后行为不变。
- 既有 `test_meta_accept_typed_patch_rejects_stale_definition_binding`（SyncInstanceVariables definition 不匹配）继续通过。

### 8.4 验证证据

- `cargo fmt --all -- --check`：通过。
- `cargo clippy -p storyforge --all-targets -- -D warnings`：通过。
- `cargo test -p storyforge-app-meta`：110 passed、0 failed。
- `cargo test -p storyforge --no-default-features`：360 passed（Batch 2.3 后 359 + 1 新契约测试）、3 ignored、0 failed。
- `tests::meta` 全 14 项通过（含既有 stale/preflight/prune/dismiss 回归）。
- `node --test frontend/tests/tauri-command-contract.test.mjs`：8 passed、0 failed。
- `node scripts/architecture/backend-baseline.mjs`：175/175 注册一致，前端缺失 0，sqlite activeFlagReferences 68 不变。
- `git diff --check`：通过（仅 LF→CRLF 行尾提示）。

### 8.5 未削弱项核对

- [x] target 存在性：`is_patch_stale` 与 `validate_patch_preconditions` 共用 `action_target_missing`，只此一份。
- [x] SyncInstanceVariables definition/schema 前置：归并进纯函数，既有错误消息（"已不再使用 definition"、"缺少变量 schema"）保留。
- [x] RepointInstanceDefinition new_definition_id 存在性：归并进纯函数，"Definition 不存在" 错误保留。
- [x] Preview 与 Accept 前置判定一致：两者现共用 `validate_patch_preconditions`。
- [x] 写盘路径（`apply_typed_action`）、SQLite backend 拒绝（`ensure_typed_patch_backend_supported`）、active-turn barrier：未改动。
- [x] 命令名/参数/DTO/事件/前端 IPC 合同：未改动（175/175 不变）。

### 8.6 Gate 2 剩余（按计划 §7）

- **Batch 2.5（tool loop 合并）**：`run_tool_loop`/`run_tool_loop_streaming`/`run_tool_loop_with_layout`（`app-agent/runtime.rs`）三个近似实现待合并为单一可配置执行器。
- **Batch 2.6（writing 阶段复用）**：start_writing / regenerate 的 Director/Subagent/Editor 仍各写一遍；`EditorStarted` 双发待消除。
- **Batch 2.7（取消/失败事件发射）**：`SubagentCancelled` 混淆真取消与 LLM 失败；cancel skip 重发为 `PostProcessFailed`。
- **Batch 2.8（RESULT 收口）**：待上述批次完成后追加最终 Gate 2 结论。

## 9. Gate 2 Batch 2.5 检查点（tool loop 合并为单一核心执行器）

### 9.1 范围与策略

Batch 2.5 消除 `app-agent/src/runtime.rs` 三个近似工具循环的重复实现。

**改动前**：
- `run_tool_loop`（非流式，供 chronicle compressor / postprocess / summarizer / mvu import）
- `run_tool_loop_streaming`（流式 + `progress_tx` + `completion_probe`，供 character_extractor / meta_conversation / mvu_import）
- `run_tool_loop_with_layout`（流式 + `MessageLayout` 初始消息 + round1 segment-diff 诊断，供 app-pipeline 的 director/writer/subagent 与 spawn_subagents）

三者循环骨架（cancel 检查 → prompt hook → ChatRequest → LLM → drift recovery → 工具执行 → 终止判定）逐行相同，仅在消息来源、LLM 调用方式、进度转发、完成探测、layout 诊断处分叉；每份约 130 行，共 ~390 行重复。

**改动后**：抽取私有 `run_tool_loop_core(config, messages, tool_registry, cancel, log_tag, progress_tx, completion_probe, pre_hook_segs)` 作为单一权威循环。三个 public 函数变为薄包装，各自提供变体特有输入（签名与调用点不变）：
- `run_tool_loop`：`log_tag=""`、`progress_tx=None`、`completion_probe=None`、`pre_hook_segs=None`。
- `run_tool_loop_streaming`：`log_tag="[stream]"`、`progress_tx=Some`、`completion_probe=传入`、`pre_hook_segs=None`。
- `run_tool_loop_with_layout`：`log_tag="[layout]"`、`progress_tx=Some`、`completion_probe=传入`、`pre_hook_segs=Some(layout.segment_fingerprint())`。

**行为保留**：
- 非流式 LLM 调用仍走 `chat()` + 手写 cancel select；流式走 `chat_stream`（cancel 内置）。两者在 core 内按 `progress_tx.is_some()` 分支。
- no-tool-call 分支统一为 F2/F3 形态（`tools_empty` 早返回 + `completion_probe` 早终止 + drift reminder + 空响应处理）；F1（plain）传 `completion_probe=None`，退化为既有 drift-recovery 行为（无早终止）。
- layout 的 round1 segment-diff 诊断仅在 `pre_hook_segs=Some` 时触发。
- 终止工具判定、`capture_reasoning_round`、`with_captured_reasoning`、`MaxRoundsExceeded` 语义完全不变。

### 9.2 验证证据

- `cargo fmt --all -- --check`：通过。
- `cargo clippy -p storyforge-app-agent --all-targets -- -D warnings`：通过（`run_tool_loop_core` 加 `#[allow(clippy::too_many_arguments)]`，8 个参数是三版差异面的自然投影）。
- `cargo test -p storyforge-app-agent`：125 passed、0 failed。
- `cargo clippy -p storyforge --all-targets --no-default-features -- -D warnings`：通过。
- `cargo test -p storyforge --no-default-features`：360 passed、3 ignored、0 failed（与 Batch 2.4 后相同，无回归）。
- **parity 关键**：三组并行测试 `malformed_terminal_arguments_do_not_stop_plain_tool_loop` / `..._streaming_tool_loop` / `..._layout_tool_loop`（plain/streaming/layout 各一）全部通过，证明三版 terminal-recovery 行为逐字等价。
- `node --test frontend/tests/tauri-command-contract.test.mjs`：8 passed、0 failed。
- `node scripts/architecture/backend-baseline.mjs`：175/175 注册一致，前端缺失 0，sqlite activeFlagReferences 68 不变。
- `git diff --check`：通过。

### 9.3 规模与未削弱项

- 代码规模：`runtime.rs` 净 **−156 行**（2982 → 2824）；三份 ~130 行循环体合并为一份 ~150 行 core + 三个 ~15 行包装。
- [x] 工具循环骨架只有一个权威实现（计划 §7.5）。
- [x] plain 版 drift-recovery（无 completion_probe）行为保留：F1 传 None，drift reminder 分支不变。
- [x] streaming / layout 版 `completion_probe` 早终止保留。
- [x] layout 版 round1 segment-diff 诊断保留。
- [x] 取消语义（pre-round check、prompt-hook select、LLM 调用内 cancel）三版均保留。
- [x] 三个 public 函数签名与全部调用点（chronicle_compressor / postprocess / summarizer / mvu_import / character_extractor / meta_conversation / app-pipeline ×7 / spawn_subagents）未改动。
- [x] 命令名/参数/DTO/事件/前端 IPC 合同：未改动（175/175 不变）。

### 9.4 已知前置问题（非本批次引入）

- `harness-real-llm` 的 `knowledge_propagation_real_llm.rs` / `writeback_isolation.rs` / `bronze_deterministic.rs` 引用 `storyforge_tauri_app::normalize_knowledge_update_for_postprocess` / `is_postprocess_instance_present` 报 `E0603` private。这是 Gate 1（`d340d99` 把 `commands` 设为私有 `mod commands;`）遗留的可见性问题，本批次未引入（stash 验证：main@08400a8 同样失败）。这些测试属 `default-features`（真实模型 harness），不在 Gate 2 确定性门禁（`--no-default-features`）内；修复需在 lib.rs 补 re-export 或迁移 harness 调用路径，建议作为独立 follow-up，不混入 Gate 2。

### 9.5 Gate 2 剩余（按计划 §7）

- **Batch 2.6（writing 阶段复用）**：start_writing / regenerate 的 Director/Subagent/Editor 仍各写一遍；`EditorStarted` 双发待消除。
- **Batch 2.7（取消/失败事件发射）**：`SubagentCancelled` 混淆真取消与 LLM 失败；cancel skip 重发为 `PostProcessFailed`。
- **Batch 2.8（RESULT 收口）**：待上述批次完成后追加最终 Gate 2 结论。

## 10. Gate 2 Batch 2.6 检查点（EditorStarted 双发消除；阶段抽取 PARTIAL）

### 10.1 已完成：EditorStarted 双发消除

regenerate 的路径 B（`rerun_editor_only`）与路径 C（`rerun_subagents`）在调用 `run_editor_and_commit` 前各自发射了一次 `EditorStarted`，而 helper 内部（`lib.rs:2934`）又发射一次，导致**双发**。前端 `usePipeline.js` 的 `editor_started` 处理器会重置 `editor.output = ''`，双发会清掉已经开始流式的编剧输出（实际产品缺陷）。

**修复**：删除路径 B（`lib.rs:2399`）与路径 C（`lib.rs:2628`）的冗余 `EditorStarted` 发射；保留各自的 `StateChanged{Editing}`（helper 设 state 但不重发 `StateChanged`）。`run_editor_and_commit` 内部发射点（`:2934`）保留为唯一规范发射点。

**新增契约测试** `test_regenerate_editor_only_emits_editor_started_once`：RED 阶段实测双发（count=2），修复后 GREEN（count=1）。

### 10.2 PARTIAL：Director / Subagent 阶段抽取（推迟）

计划 §7.2 要求「把 Director、Subagent、Editor 阶段抽成可复用 stage functions；start_writing 与 regenerate 共享同一阶段实现」。本批次**未完成**此抽取，原因与边界如下（诚实记录，不虚报 PASS）：

**重复现状**（`app-pipeline/src/lib.rs`，7,868 行）：
- **Director 阶段**：`start_writing_with_mode_at`（`:822-950`）与 regenerate 路径 A（`:2113-2236`）近逐行重复（`has_available_characters` 守卫、`make_director_config` + registry + whitelist、`recent_history_with_epoch` + chronicle 分区、`director_layout`、`DirectorProgress` forwarder、`run_tool_loop_with_layout` + plan 探测、`parse_plan_from_response` + `DirectorDone`）。唯一真实差异：intent 来源（start_writing 用用户 intent；路径 A 从 `provenance_old.plan.scene_brief` 推导）与 node-id 来源。
- **Subagent 阶段**：`start_writing_with_mode_at`（`:952-1085`）与 regenerate 路径 A（`:2238-2346`）近逐行重复（`Delegating` 状态、`SubagentStarted` 循环、`char_specs`/temporaries、`effective_runtime`、`max_concurrent`/`summary_block`/`far_block`/`subagent_base`、结果循环 `SubagentDone`/`SubagentCancelled`、全失败守卫）。真实差异：start_writing 有 SequentialCrew 分支；路径 A 总走 `spawn_subagents`。
- **Editor 阶段**：`start_writing_with_mode_at` 内联（`:1087-1242`）与 `run_editor_and_commit`（`:2906-3095`）近逐行重复（`make_editor_config`、`NarrativeContract`、`redact_performances_for_editor`、`editor_layout`、`EditorProgress`、`run_tool_loop_with_layout`、`apply_editor_output_regex` + `DraftReady`）。真实差异：落地语义（start_writing 用 `append_ai_draft` 新节点；helper 用 `replace_active_variant`/`add_variant` 变体）与返回类型。

**推迟理由**：
1. 三阶段都涉及深度 `&mut self` 状态变更、事件发射与编排逻辑；start_writing 有 SequentialCrew 分支，落地语义不同（新节点 vs 变体）。抽取需要把 intent 来源、SequentialCrew 分支、node-id 来源、落地语义全部参数化，改动面覆盖 ~600 行核心写作路径。
2. 计划硬约束「不在机械拆分阶段顺手改行为」「不削弱已有错误检查/权限检查」；当前确定性测试门禁（mock-LLM）虽全绿，但 Director/Subagent 抽取在没有真实模型回归证据下风险偏高，可能引入 start_writing / regenerate 之间的微行为分叉。
3. EditorStarted 双发是**确定的产品缺陷**（前端流式输出被清），本批次已确定性修复并测试；阶段抽取属**架构整洁性**目标，可独立排期，不应在缺乏充分回归护栏时强行合并。

**建议后续**：Director/Subagent/Editor 阶段抽取应在 Gate 6（真实模型证据）建立后，以独立子批执行——先抽 Editor（落地语义参数化，复用 `run_editor_and_commit`），再抽 Director（intent 来源参数化），最后抽 Subagent（SequentialCrew 分支参数化）。每步配 mock-LLM parity 测试 + 真实模型冒烟。

### 10.3 验证证据（EditorStarted 修复）

- `cargo fmt --all -- --check`：通过。
- `cargo clippy -p storyforge-app-pipeline --all-targets -- -D warnings`：通过。
- `cargo test -p storyforge-app-pipeline`：110 passed（既有 109 + 1 新契约测试）、0 failed。
- `cargo test -p storyforge --no-default-features`：360 passed、3 ignored、0 failed（无回归）。
- 既有 `test_regenerate_editor_only` / `test_regenerate_subagent_only` / `test_regenerate_full_with_hint` 等回归全通过。
- `node --test frontend/tests/tauri-command-contract.test.mjs`：8 passed、0 failed。
- `node scripts/architecture/backend-baseline.mjs`：175/175 注册一致，前端缺失 0，sqlite activeFlagReferences 68 不变。
- `git diff --check`：通过。

### 10.4 未削弱项核对

- [x] `EditorStarted` 在 regenerate 路径 B/C 只发一次（契约测试钉住）。
- [x] `StateChanged{Editing}` 在路径 B/C 仍发射（helper 不重发，故保留前置发射）。
- [x] 路径 A（rerun_director）与 `regenerate_sequential_suffix` 本就单发，未受影响。
- [x] start_writing / start_duet 的内联 EditorStarted 单发，未改动。
- [x] 命令名/参数/DTO/事件词汇/前端 IPC 合同：未改动（175/175 不变；`editor_started` 事件本身保留，只是不再双发）。

### 10.5 Gate 2 剩余（按计划 §7）

- **Batch 2.7（取消/失败事件发射）**：`SubagentCancelled` 混淆真取消与 LLM 失败；cancel skip 重发为 `PostProcessFailed`。
- **Batch 2.8（RESULT 收口）**：待 2.7 完成后追加最终 Gate 2 结论（含 2.6 阶段抽取 PARTIAL 的诚实记录）。

## 11. Gate 2 Batch 2.7 检查点（取消/失败事件发射区分）

### 11.1 已完成：postprocess cancel 不再误报为 failed

`run_shared_postprocess_background`（`runtime_support.rs:100-110`）在 postprocess 被取消（`skipped_reason == "cancelled"`）时，原先发 `PostProcessFailed { reason: "postprocess cancelled" }`。前端 `usePipeline.js` 的 `postprocess_failed` 处理器把 postprocess 置为 `status: 'error'`（显示「后处理失败」），对用户主动取消是误导。

**修复**：取消时改发 `PostProcessSkipped { reason: "postprocess cancelled" }`（前端置 `status: 'idle'`，无错误态）。真实存储/derivation 失败仍走 `Err(e)` 分支发 `PostProcessFailed`（`runtime_support.rs:111-118`），不受影响。

### 11.2 PARTIAL：SubagentCancelled 混淆真取消与 LLM 失败（受约束推迟）

计划 §7.2 要求「统一取消、失败、重试和事件发射；消除双发、漏发和'失败被标成取消'」。子 Agent 结果循环（`app-pipeline/lib.rs:1062-1070` 路径 A 同 `:2318-2336`）在 `Err(e)` 时统一发 `SubagentCancelled`，无论 `e` 是 `AgentError::Cancelled`（真取消）还是 `AgentError::Llm(...)`（真失败）。

**未修复理由**：`PipelineEvent` 枚举（`domain/src/agent.rs:438`）只有 `SubagentCancelled`，没有 `SubagentFailed` 变体。区分真取消与真失败需要新增事件变体（或给 `SubagentCancelled` 加 `reason` 字段），这会改变事件词汇表与前端 IPC 合同，违反硬约束「不改变现有事件名和前端 IPC 合同，除非有独立迁移计划」。故本批次只修了 postprocess cancel/failed 混淆（无需新事件），SubagentCancelled 的区分留待独立的事件词汇演进计划。

**现状缓解**：全失败的子 Agent 集仍会触发 `PipelineError::InvalidState` 中止（`:1078-1085`），编排层不会因混淆而静默继续；只是单个失败子 Agent 在前端显示为「取消」而非「失败」。

### 11.3 验证证据

- `cargo fmt --all -- --check`：通过。
- `cargo clippy -p storyforge --all-targets --no-default-features -- -D warnings`：通过。
- `cargo test -p storyforge --no-default-features`：360 passed、3 ignored、0 failed（无回归）。
- 既有 `cancel_after_runner_discards_outcome_and_does_not_await_acceptance`（验证 `skipped_reason == "cancelled"`）继续通过。
- `node --test frontend/tests/tauri-command-contract.test.mjs`：8 passed、0 failed。
- `node scripts/architecture/backend-baseline.mjs`：175/175 注册一致，前端缺失 0，sqlite activeFlagReferences 68 不变。
- `git diff --check`：通过。

### 11.4 未削弱项核对

- [x] postprocess 取消发 `PostProcessSkipped`（前端 idle），不再误发 `PostProcessFailed`（前端 error）。
- [x] 真实 postprocess 存储/derivation 失败仍发 `PostProcessFailed`。
- [x] 事件词汇表（`PostProcessDone`/`PostProcessFailed`/`PostProcessSkipped`）未新增/改名。
- [x] 命令名/参数/DTO/前端 IPC 合同：未改动（175/175 不变）。

## 12. Gate 2 收口（Batch 2.8）

### 12.1 Gate 2 总结论：PARTIAL

Gate 2（业务状态机与重复编排收敛）按计划 §7 分 8 个子批执行。结论为 **PARTIAL**：核心状态机统一（Batch 2.1–2.5、2.7 的可修部分）已完成并测试；两处需独立迁移计划或真实模型回归护栏的项目诚实记为 PARTIAL/推迟。

### 12.2 各批次结论

| 批次 | 内容 | 结论 | 提交 |
|---|---|---|---|
| 2.1 | `compute_draft_hash` 提升为 domain 单一权威 | ✅ PASS | `ca048ae` |
| 2.2 | `evaluate_accept_decision` 后端无关 Accept 决策核心 | ✅ PASS | `71e7884` |
| 2.3 | postprocess mutation builder 统一（JSON/SQLite 共享纯函数） | ✅ PASS | `7ba5df1` |
| 2.4 | typed patch preview/apply 前置条件纯函数统一 | ✅ PASS | `efd442f` |
| 2.5 | 三个 tool loop 合并为单一 `run_tool_loop_core` | ✅ PASS | `6c57b1a` |
| 2.6 | EditorStarted 双发消除 | ✅ PASS | `d57ebf5` |
| 2.6 | Director/Subagent/Editor 阶段抽取 | ⚠️ PARTIAL（推迟） | — |
| 2.7 | postprocess cancel 不再误报 failed | ✅ PASS | `0904379` |
| 2.7 | SubagentCancelled 区分真取消/失败 | ⚠️ PARTIAL（需新事件变体） | — |
| 2.8 | RESULT 收口 | ✅ 本节 | — |

文档检查点提交：`a04f960`（2.1+2.2）、`beb7870`（2.3）、`08400a8`（2.4）、`4b0e395`（2.5）、`2b3b6f2`（2.6）、本节（2.7+2.8）。

### 12.3 Gate 2 通过条件核对（计划 §7.5）

- [x] JSON/SQLite Accept 对照测试使用同一输入得到同一领域结果和同类错误 — Batch 2.2（7 parity 测试）。
- [ ] start/regenerate 不再复制完整阶段实现 — **PARTIAL**：EditorStarted 双发已消除（2.6），Director/Subagent/Editor 阶段抽取推迟（2.6 PARTIAL）。
- [x] tool loop 只有一个权威实现 — Batch 2.5（`run_tool_loop_core`）。
- [x] postprocess mutation 规则只有一个权威实现 — Batch 2.3（共享纯函数 + parity 测试）。
- [x] typed patch preview 与 apply 使用同一纯函数 — Batch 2.4（`validate_patch_preconditions`）。

### 12.4 推迟项与后续计划

1. **Director/Subagent/Editor 阶段抽取**（2.6 PARTIAL）：~600 行核心写作路径，差异面（intent 来源、SequentialCrew 分支、落地语义）需参数化。建议 Gate 6（真实模型证据）后以独立子批执行（Editor→Director→Subagent），每步配 mock-LLM parity + 真实模型冒烟。
2. **SubagentCancelled vs 失败**（2.7 PARTIAL）：需新增 `SubagentFailed` 事件变体或给 `SubagentCancelled` 加 `reason` 字段，属事件词汇演进，应有独立迁移计划（含前端适配），不混入 Gate 2。
3. **harness-real-llm 可见性**（2.5 已知前置）：`normalize_knowledge_update_for_postprocess`/`is_postprocess_instance_present` 在 Gate 1 后变 private，`harness-real-llm`（default-features）测试报 E0603。需在 lib.rs 补 re-export 或迁移 harness 调用路径；不在确定性门禁内，建议独立 follow-up。

### 12.5 Gate 2 验证证据汇总（最终态）

- `cargo fmt --all -- --check`：通过。
- `cargo clippy -p storyforge --all-targets --no-default-features -- -D warnings`：通过。
- `cargo clippy -p storyforge-app-agent --all-targets -- -D warnings`：通过。
- `cargo clippy -p storyforge-app-pipeline --all-targets -- -D warnings`：通过。
- `cargo clippy -p storyforge-app-meta --all-targets -- -D warnings`：通过。
- `cargo test -p storyforge --no-default-features`：**360 passed**（Gate 1 基线 351 + 9 新 parity/契约测试）、3 ignored、0 failed。
- `cargo test -p storyforge-app-agent`：125 passed。
- `cargo test -p storyforge-app-pipeline`：110 passed。
- `cargo test -p storyforge-app-meta`：110 passed。
- `node --test frontend/tests/tauri-command-contract.test.mjs`：8 passed、0 failed。
- `node scripts/architecture/backend-baseline.mjs`：175/175 注册一致，前端缺失 0，sqlite activeFlagReferences 68 不变。
- `git diff --check`：通过。
- 命令属性/注册 **175/175**，前端唯一 invoke **162**，缺失后端命令 **0**——IPC 合同未改。

### 12.6 未削弱项最终核对

- [x] revision CAS、active-turn barrier、fail-closed、`AcceptError` 分类、`quality_accept_decision`：Batch 2.1/2.2 统一后未削弱。
- [x] postprocess 字段一致性（角色解析/同名/在场/广播/传播/变量归属/任务）：Batch 2.3 parity 测试钉住。
- [x] typed patch target/definition/schema 前置：Batch 2.4 归并，错误消息保留。
- [x] tool loop cancel/drift/terminal 语义：Batch 2.5 三版 parity 测试钉住。
- [x] EditorStarted 单发、postprocess cancel 不误报 failed：Batch 2.6/2.7 修复。
- [x] SQLite 活跃时不访问 legacy JSON authority、无双写：未涉及（本 Gate 不动存储路由）。
- [x] 命令名/参数/DTO/事件名/前端 IPC 合同：全程未改（175/175、162 invoke、0 missing）。

### 12.7 下一阶段

Gate 2 PARTIAL 后进入 **Gate 3（单一 backend facade）**。Gate 3 目标：进程启动时解析一次 backend，构造显式 facade/port 集合；命令层不读全局 `is_sqlite_active()` flag（当前 68 处）。Gate 2 的推迟项（阶段抽取、SubagentCancelled）不阻塞 Gate 3 facade 抽象，可在 facade 稳定后并行补齐。

## 13. Gate 2 verifier 返修与最终验收（2026-07-29）

### 13.1 返修范围与结果

前次第 12 节记录的是 verifier 介入前的历史检查点；verifier 随后将目标标为 incomplete。本次按实际代码逐项复现、补测试并修复，结果如下：

| 项目 | 返修结果 | 证据 |
|---|---|---|
| postprocess 实例解析优先级 | 精确 ID 优先，只有 ID 不存在时才按名称回退 | `instance_resolution_prefers_exact_id_over_an_earlier_name_match` |
| SQLite Accept scope/error parity | scope 与 terminal replay 先于 live conversation/campaign 读取；错误分类与 JSON 一致且无副作用 | `sqlite_optin_cutover_write_regenerate_force_accept_and_restart_recovery` 中的 scope 零副作用断言及 replay 回归 |
| harness helper 可见性 | crate root 只 re-export 两个既有 helper | `cargo test -p harness-real-llm --no-run` |
| Editor 阶段复用 | 标准 start、Duet 与 regenerate 的 editor 执行共用 `run_editor_stage`；落地由调用方保留 | 事件顺序、单发与 full-regenerate 回归 |
| Director 阶段复用 | 标准 start 与 full regenerate 路径 A 共用 `run_director_stage` | 配置、历史、hint、progress、plan parse 回归 |
| Subagent 阶段复用 | 标准/Sequential start 与 full regenerate 路径 A 共用 `run_subagent_stage` | runner 选择、临时实例、失败守卫、provenance 回归 |

三段抽取均保持现有 command、DTO、事件名称和前端 IPC 合同；Duet、subagent-only reroll、Sequential suffix 等语义不同的编排没有被错误并入通用路径。每一段均经过独立代码审查并得到 APPROVE。

### 13.2 Gate 2 通过条件复核（计划 §7.5）

- [x] JSON/SQLite Accept 使用同一领域决策并得到同类结果/错误；本次补齐 SQLite scope/replay 调用顺序。
- [x] start/regenerate 不再复制完整 Director、Subagent、Editor 阶段实现；三段共享 helper 均有行为契约测试。
- [x] tool loop 只有一个权威 `run_tool_loop_core`。
- [x] postprocess mutation 规则只有一套共享纯函数实现；本次补齐 ID 优先语义回归。
- [x] typed patch preview/apply 使用同一 `validate_patch_preconditions` 纯函数。

结论由第 12 节的历史 **PARTIAL** 更新为最终 **PASS（5/5）**。

### 13.3 最终验证证据

- `cargo fmt --all -- --check`：通过。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过。
- `cargo test --workspace`：通过，退出码 0；需要真实 LLM 凭证或 OS credential-store 的测试按既有约定 ignored。
- `cargo test -p storyforge --no-default-features`：361 passed、3 ignored、0 failed。
- `cargo test -p storyforge-app-pipeline --lib`：117 passed、0 failed。
- `cargo test -p harness-real-llm --no-run`：通过，原 E0603 已消除。
- `frontend/npm.cmd test`：476 passed、0 failed。
- `frontend/npm.cmd run build`：通过；仅保留既有动态/静态 import 与大 chunk 警告。
- `node scripts/architecture/backend-baseline.mjs`：175 个 command 属性/175 个注册，162 个前端唯一 invoke，0 missing，16 crates，`activeFlagReferences=68` 未变化。
- `git diff --check`：通过。

### 13.4 明确不在 Gate 2 内的后续项

- `SubagentCancelled` 区分真取消与 LLM 失败需要事件词汇/前端 IPC 迁移，继续作为独立计划项；当前 all-failed guard 仍 fail-closed。
- SQLite typed patch 的持久化能力补齐属于 Gate 4，不属于 Gate 2 的 preview/apply 前置条件统一。
- 真实付费模型与 Windows/Android 现场证据分别属于后续 Gate 6/平台验收，本次不虚报。
- 当前修复尚未提交；提交后应把本节 code-under-test 从工作树描述替换为确切 SHA。

### 13.5 下一阶段

进入 **Gate 3（单一 backend facade）**。Gate 3 的目标保持不变：启动时一次性解析 backend，构造显式 facade/port 集合，并逐步消除命令层对 `is_sqlite_active()` 的 68 处直接读取。

## 14. Gate 3 Batch 3.1：显式 facade 与首个垂直切片（2026-07-29）

### 14.1 已完成

- `StorageFacade` 持有 `PinnedBackend` 和规范 `data_dir`；生产 setup 把 backend resolution 直接注入 `AppState`，测试构造显式使用 JSON facade。
- `AppState` 与 facade 必须共享同一目录，构造时 fail-fast；ConversationStore authority、pipeline defer-land、启动 recovery 和 active pointer 逻辑改读注入 facade。
- 能力矩阵显式区分 supported、degraded、unsupported、migration required、read-only recovery；SQLite Campaign instance、变量命令、知识/任务命令、WorldInfo 与 Gate 4 能力均保持显式 unsupported，没有被聚合能力名伪装成 supported。
- Campaign list/get/get-active 通过 facade 统一 JSON/SQLite 分派并保留 `card_id` 过滤；Campaign lifecycle 与 Conversation 级联删除通过能力声明保持显式 unsupported；active Campaign 选择保留 JSON 指针落盘和 SQLite 仅进程内选择的既有差异。
- 删除 AppState 默认/隐式 backend 构造入口；SQLite runtime 重复激活仅允许同一规范路径，AppState 构造校验 facade backend、数据目录与实际 SQLite authority 一致，发现分叉即 fail closed。
- 前端 IPC 命令名与显式参数未变；新增的 Tauri `State` 参数由框架注入。

### 14.2 TDD 与验证

- RED：facade/能力类型不存在时目标 Rust 测试按预期编译失败；AppState 未提供显式 backend 构造时第二个目标测试按预期失败；分支合同在实际 63、目标 55 时按预期失败。
- GREEN：`storage_backend::tests` 7/7；`cargo test -p storyforge --no-default-features` 364 passed、3 ignored、0 failed；`cargo clippy -p storyforge --all-targets --no-default-features -- -D warnings` 通过；前端命令合同 8/8；fmt 与 `git diff --check` 通过。
- 架构基线：commands 175/175、frontend invokes 162、missing 0、crates 16；`activeFlagReferences` **68 → 55**，其中 facade/runtime 允许 2 处，`applicationFlagReferences=53`。

### 14.3 当前结论与下一切片

Gate 3 为 **IN PROGRESS**，不是 PASS。下一批应迁移 Turn/Attempt/Accept 与 Writing/runtime-support，让应用服务不再读取 ambient flag；随后迁移 Meta、WorldInfo、Character 和 Chronicle compressor，直至命令/应用服务内 `is_sqlite_active()` 为 0，并验证 SQLite 活跃时没有 JSON 写入者构造机会。

## 15. Gate 3 完成：单一 backend facade（2026-07-31）

### 15.1 交付内容

- 新建 `crates/tauri-app/src/backend_workflows.rs`（命名 backend adapter，~1,200 行）：
  - **Backend-neutral workflow DTO**：`DraftAttemptRequest` / `RegenerateAttemptRequest` / `DraftAttemptOutcome`——只用 domain 类型（`Id`、`Provenance`、`CharacterInstance`），应用层不再暴露 `storyforge_infra_sqlite::preaccept::*` 请求/结果类型。
  - **`TurnWorkflow`**：`AppState::new_with_backend` 构造一次注入（`AppState::turn_workflow`），命令层调用 `create_draft_attempt` / `append_regenerate_attempt` / `accept_by_variant` / `edit_variant_with_stale_mark`——JSON/SQLite 分派（含 SQLite UoW 后 conversation cache invalidate、JSON 软删补偿与 Failed 标记）全部内聚在 adapter。
  - **`BackendTurnAttemptSink`**（从 `commands/writing.rs` 移入）：`sync_autofix_with_provenance` / `attach_postprocess` 的 SQLite typed 预校验与 JSON 条件 mutate 保留原语义。
  - **`build_postprocess_service` 工厂**：Postprocess batch source（`Runtime` vs `JsonStore`）在 adapter 边界按 backend 选择一次；SQLite 无 CampaignRuntimeContext 时保持显式 fail-closed，不悄悄回退 JSON。
  - 迁移吸收：`prepare_start_conversation(_async)`、MVU 两个 `*_for_backend` 收集器 + `collect_mvu_from_sqlite`、`persist_postprocess_outcome_async`、`fill_campaign_runtime_from_sqlite`（crate root 保留 re-export，harness 兼容）、`load_campaign_context_snapshot_for_backend`、Chronicle compressor 全家（`maybe_spawn_chronicle_compress` / `recover_compress_jobs_on_startup` / `spawn_compress_job_worker` 等）、`campaign_health_issues_for_backend`、`conversation_card_name_for_backend`、`campaign_scoped_regex_scripts_for_backend`、`character_instance_dto_for_backend`。
- `StorageFacade`：删除六个 SQLite-only API（`create_draft_attempt` / `append_regenerate_attempt` / `sync_autofix` / `apply_postprocess` / `mark_stale_after_edit` / `accept_by_variant`），不再向调用方暴露 infra-sqlite 类型；新增 `save_active_pointer`（JSON 写指针文件 / SQLite no-op，吸收 campaigns 与 playthrough 指针逻辑）与 `defer_pipeline_conversation_land`。
- `runtime_support.rs` 不再读取 backend flag；`persist_temporary_instances_async`（dead_code）删除，单测保留 `persist_temporary_instances_to*`。
- 静态门禁双保险：
  - 新增 Rust 测试 `lib_tests_backend.rs`：遍历 `src/` + `src/commands/`，白名单 = {`lib.rs`（bootstrap）、`storage_backend.rs`、`sqlite_runtime.rs`、`backend_workflows.rs`}，白名单外的 `.is_sqlite()`/`.is_json()` 一律报错；并钉住 commands 不得使用 ambient `get_store()`。
  - `scripts/architecture/backend-baseline.mjs` 新增 `applicationMethodFlagReferences` / `facadeMethodFlagReferences` / `methodFlagReferencesByFile` 字段。
- 修复陈旧测试 `character_delete_checks_campaign_lifecycle_before_any_legacy_mutation`（断言字面量 `get_store()` → `character_store`）。
- **评审修复（2026-07-31，两处 P1 行为回归 + 二次评审修正）**：
  - `TurnWorkflow::create_draft_attempt`/`append_regenerate_attempt` 的 SQLite 失败写回改为**条件化**（`mutate_turn_if`，predicate = scope 匹配 && 状态仍为 `Generating`）——首稿/重生成 UoW 失败时，仅对仍占用 barrier 的 Generating Turn 标 `Failed` + `failure_reason`；已推进到 `DraftReady`/`Committed` 的 Turn（重复或竞态请求被 UoW 拒绝）**绝不降级**。写回错误经 `tracing::error` 报告，不吞掉。回归段两条：fault 注入（`PreacceptFault::BeforeCommit` 真实回滚路径）下 Generating Turn → `Failed`；Committed Turn 被拒绝后状态与 `failure_reason` 原样保持。
  - Accept 的 `campaign_revision_after` 改为直接取 UoW 校验过的 `batch.target_revision`（UoW 内 `campaign.revision = target` 且校验 `target == expected + 1`；replay 路径本就使用持久化 batch 的 target_revision），**删除提交后二次读取**——提交成功却因读失败误报 `AcceptError::Commit` 会让调用方跳过 conversation invalidate、summary 索引与 Chronicle enqueue。回归段断言：首轮 accept 返回 `campaign_revision_after == 1`、第二轮 accept 返回值与盘上权威 revision 一致。
- **评审修复（中等项）**：`list_conversations` 从每会话一次 `list_cards()` 退化为单次快照（新批量 adapter `conversation_card_names_for_backend`，一次 `list_cards()` 建映射）；`lib_tests_backend` 静态门禁改为递归扫描且白名单按 **src/ 相对路径**精确匹配（嵌套同名文件如 `commands/storage_backend.rs` 无法绕过）；frontend 合同测试新增 `applicationMethodFlagReferences == 0` 断言。

### 15.2 修改前后后端分支统计

| 指标 | Batch 3.1 结束时 | Gate 3 完成后 |
|---|---|---|
| commands / runtime_support / playthrough 内 `.is_sqlite()` / `.is_json()` | 30 处 | **0** |
| 六个 SQLite-only facade API（暴露 infra-sqlite 类型） | 6 | **0** |
| `applicationMethodFlagReferences`（baseline） | —（新增字段） | **0** |
| `facadeMethodFlagReferences`（baseline） | —（新增字段） | 59（backend_workflows 22 / lib.rs 5 / storage_backend 32） |
| `activeFlagReferences`（`is_sqlite_active(`） | 2 | 2（facade 内） |
| `applicationFlagReferences` | 0 | 0 |
| `ambientCharacterStoreReferences` | 0 | 0 |
| `facadeSelectedWriterConstructors` | 4 | 4 |
| `applicationSelectedWriterConstructors` | 0 | 0 |

### 15.3 Gate 3 四项通过标准逐项核对

- [x] **Tauri command 和应用服务内 `is_sqlite_active()` 为 0**：commands/、`runtime_support.rs`、`playthrough_lifecycle.rs` 的 `.is_sqlite()` / `.is_json()` 全部归零（30 → 0），`is_sqlite_active(` 自 Batch 3.1 起仅剩 facade 内部 2 处。
- [x] **backend flag 只存在于 bootstrap/facade 构造和专用测试**：静态门禁测试按文件白名单钉住；`baseline.mjs` 同口径统计 `applicationMethodFlagReferences=0`。
- [x] **SQLite 活跃时 JSON 写入者没有构造机会**：`StorageFacade::new` 在 SQLite 分支不构造四个 JSON store（`has_json_writers()=false`），既有 `sqlite_*_capability` 测试与 `facadeSelectedWriterConstructors=4`（仅 facade）不变。
- [x] **新增命令无需自行添加 JSON/SQLite 分支**：Turn/Attempt/Accept/Postprocess 经注入的 `TurnWorkflow`/sink/工厂；campaign 指针、health、card-name、regex、MVU 收集经命名 adapter 函数；命令层只剩 capability 声明式门控与 `*_for_backend` 调用。

### 15.4 关键行为保持

- 六个 SQLite-only API 的语义原样迁入 adapter：JSON 软删补偿、Turn Failed 标记、错误消息文案（前端依赖展示）不变；SQLite UoW 后 conversation invalidate 保留。
- SQLite 下既有能力缺口**不静默回退**：role_type 富化（SQLite 保持 `None`）、conversation card_name（`None`）、campaign-scoped regex（走 character 维度）均为文档化 Gate 4 数据完整性事项；chronicle compressor 在 SQLite 下继续显式跳过。
- `meta_health_check` 的 JSON missing-campaign 错误从 `not_found` 分类变为 `storage` 分类（消息文本不变）；已确认前端不按 `TauriCommandError.type` 分支，无行为影响。
- `edit_variant` 等 IPC 命令签名不变；`set_active_campaign_in_state` 签名去掉 `json_active` 参数（测试同步更新）。

### 15.5 验证证据（全部通过，含评审修复后的复验）

- `cargo fmt --all -- --check`：通过。
- `cargo check --workspace`：通过，零警告。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过。
- `cargo test --workspace`：全部通过（tauri-app 374 passed / 3 ignored；含新增 `gate3_backend_flag_whitelist_is_exactly_bootstrap_facade_and_adapter`、`gate3_commands_never_use_the_ambient_character_store`、optin lifecycle 的 revision 一致性回归段与 TurnWorkflow 首稿失败标 Failed 回归段）。说明：首次全量运行时 `infra-sqlite importer::tests::concurrent_same_manifest_converges_to_one_completed_run` 出现一次并发窗口失败，孤立重跑两次与全量复跑均通过——既有并发 flake，与本次改动无关（该测试与 importer 代码均未触碰）。
- `node scripts/architecture/backend-baseline.mjs`：`applicationMethodFlagReferences=0`、`applicationFlagReferences=0`、`applicationSelectedWriterConstructors=0`。
- `node --test frontend/tests/tauri-command-contract.test.mjs`：8/8 通过（含新增 `applicationMethodFlagReferences == 0` 断言）。
- `npm test`（frontend）：476 passed、0 failed。
- `npm run build`（frontend）：通过（仅既有 chunk-size 警告）。
- `git diff --check`：通过。

### 15.5.1 关于「一次性 port 选择」的口径说明

Gate 3 计划 §8.1 允许「trait 或启动时选择的 **enum-dispatched struct**」。`TurnWorkflow`/`BackendTurnAttemptSink`/`build_postprocess_service` 采用后者：`AppState::new_with_backend` 构造时注入，其持有的 `StorageFacade.pinned` 是进程级 `OnceLock` 的一次性不可变选择；每次方法调用读 `is_sqlite()` 读取的是该不可变选择（分派），不是运行时探测/重新选择。评审按「严格 trait 实现选择」口径指出未达到「构造时选 JsonTurnWorkflow/SqliteTurnWorkflow 具体实现」——这是实现风格差异，不属于行为缺口；若后续 Gate 需要 trait 化（如为 harness 注入 fake workflow），可在 facade 稳定后无损演进。四条通过条件与「命令不分支」口径下结论不变。

### 15.6 提交

- Gate 3 完成提交：未 push；提交 SHA 以 `git log` 为准（本段不引用自身提交，避免 amend 自指失效）。

### 15.7 下一阶段（Gate 4）与安全后续项（不属本 Gate）

- Gate 4 起补齐 SQLite 缺口：Meta UoW（typed patch preview/accept/dismiss、active-turn barrier、故障注入回滚）、MVU schema apply/definition 更新/backfill、WorldInfo、CharacterCommands/ImportExport、Chronicle compressor 的 SQLite-native 实现，以及 card_name/role_type/campaign-scoped regex 数据完整性收口。
- 记录为后续任务（本次**不处理**）：CardShell 路径泄露审查、Import/Export 导出脱敏等无关安全待办。

## 30. Gate 4 完成：SQLite 缺口补齐（2026-07-31）

> 结论：**Gate 4 PASS**。四项通过条件逐项核对见 §30.4；本 Gate 只实现了
> SQLite-native 能力，未改变默认后端（仍是 JSON），未 push。
>
> code-under-test：`main@102c8d0` 之上新建 Gate 4 提交（SHA 以 git log 为准）。

### 30.1 交付内容

**一、Meta 原子事务（SQLite-native）**
- 新 `crates/tauri-app/src/sqlite_meta_repo.rs`：`SqliteMetaRepository::apply_typed_patch_actions`
  单事务 UoW，承载 8 类 `TypedPatchAction`；每条 action 做 campaign scope 校验
  （instance/task/knowledge 归属、definition 绑定、new_definition_id 存在性），
  任一失败整体回滚。`MetaPatchFault::{AfterFirstAction, AfterAllActions}` 故障注入。
- `meta_propose_campaign_repairs` / `meta_preview_typed_patch` / `meta_accept_typed_patch`
  改为后端无关快照（`load_meta_snapshot_for_backend`，JSON store 或 SQLite 权威），
  propose/preview/accept 共用同一 `check_campaign_health` + `build_patch_for_issue` +
  `validate_patch_preconditions` 纯函数链；accept 的写盘经
  `apply_typed_patch_actions_for_backend` 分派（JSON 逐 action + 全局锁 / SQLite 单事务）。
- **revision 校验**：`TypedPatch.campaign_revision: Option<u64>`（serde default 兼容），
  propose 盖章、accept 比对，不一致标 Stale 拒绝；active-turn barrier（
  `reject_if_active_turn`）与 scope/stale 校验保持。
- legacy `meta_accept_patch`：活动 Campaign 分支改为经 facade `set_world_info`
  （SQLite 走 V006 表）；无活动 Campaign 的 CharacterStore 分支在 SQLite 下保持
  显式 capability fail-closed（CharacterStore 仍未 SQLite 化，见 §30.6）。
- `meta_backend.rs` 删除 `ensure_typed_patch_backend_supported` /
  `ensure_json_meta_backend_supported` 两个过时守卫——「unsupported until an
  atomic SQLite Meta UoW」字符串全库归零。

**二、MVU 补齐**
- 新 `crates/tauri-app/src/sqlite_mvu_repo.rs`：`SqliteMvuRepository::apply_schema`
  单事务 UoW（translation → source 卡反查 → definition 归属校验 → 归一化 +
  `compute_apply_preview` → card payload 更新 → 跨 campaign instance 默认值回填），
  `MvuApplyFault::{AfterCardUpdate, AfterInstanceBackfill}` 故障注入。
- `meta_preview_mvu_apply` / `meta_apply_mvu_schema` 经 backend_workflows 分派
  （`preview_mvu_apply_for_backend` / `apply_mvu_schema_for_backend`）；JSON 既有
  `meta_apply_mvu_schema_in_store` 保留为 JSON writer，测试签名改为 `&Id`。

**三、Chronicle compressor（SQLite-native）**
- 新 V006 迁移：`chronicle_compress_jobs`（campaign 级 open 去重部分唯一索引、
  attempts/max_attempts/status 列）+ `campaign_world_info` 表。
- 新 `crates/tauri-app/src/sqlite_compress_jobs.rs`：enqueue（幂等去重）、原子
  claim（Pending→Running）、succeeded/failed_or_retry（仅 Running 终态化，迟到
  结果安全）、Running→Pending 崩溃恢复、uncovered A/B 计数（SQLite 权威）。
- `backend_workflows` chronicle 段重写：`maybe_spawn_chronicle_compress` /
  `recover_compress_jobs_on_startup` / `spawn_compress_job_worker` 三处 JSON/SQLite
  分派；worker 可注入压缩执行器（生产 = `run_compress_if_needed` 真实 LLM，测试 =
  确定性 `publish_with_deterministic_texts`）；发布走既有
  `SqliteChronicleRepository::publish_compress` UoW（job_id 唯一索引拒绝重复发布/
  迟到结果，revision CAS 冲突 → job 回队重试）。
- **删除两个「sqlite backend skips」生产分支**；Gate 3 的
  `should_recover_json_compress_jobs` 删除，对应测试改写为「SQLite 门面不构造
  JSON CompressJobStore」契约。

**四、story_clock 与数据完整性**
- 唯一权威 = `variables["story_clock"]`（字符串值）。`Campaign` 新增
  `story_clock_diverged()` / `repair_story_clock_authority()`（可审核修复：不一致
  时字段从 variables 权威同步并记 warning，绝不静默任选；非字符串 variables 项
  不是合法权威，维持字段回退）。SQLite `get_campaign`/`list_campaigns` 与 JSON
  `CampaignStore` 读取均执行修复；`runtime_support`/campaigns DTO/fork 全部改读
  `current_story_clock()`；importer 索引列与 exporter 归一均以 variables 为准。
- **reverse export**：导出 campaign 载荷前归一 story_clock；新增 `campaign_world_info/`
  、`compress_jobs.json`、`mvu_translations.json` 导出（JSON 布局无损表达）；
  `mutation_commits` / `chronicle_publication_jobs` 显式分类为 unsupported（只读
  台账）；**pending pre-accept outbox 行存在时明确阻止导出**（活跃状态无法无损
  表达）；manifest hash 覆盖新增集合；importer 回读三者并保持老目录 hash 稳定。
- **WorldInfo**：V006 `campaign_world_info` 表 + `sqlite_runtime::get/set/mutate/
  ensure/delete` + facade 7 个分派方法 + world_info 命令全部改经 facade；
  惰性模板 seed 在 SQLite 下从 card payload `raw_card_json.character_book` 解析。
- **card_name / role_type / campaign-scoped regex**：`conversation_card_names_for_backend`
  （SQLite 一次 `list_card_names()` 快照）、`character_instance_dto_for_backend`
  （SQLite 从卡 payload 富化 role_type）、`campaign_scoped_regex_scripts_for_backend`
  （SQLite 从卡 payload 解析 scoped regex），三个「SQLite 保持 None/空」缺口关闭。
- **FK/scope/orphan**：FK ON 由 connection.rs PRAGMA 强制并有校验测试；Meta/MVU
  仓库行级 scope 校验；orphan Turn/Attempt 恢复（`fail_incomplete_turns` +
  `fail_incomplete_preaccept`）与 compress job 崩溃恢复在启动路径均覆盖。
- **能力矩阵**：SQLite 下 WorldInfo / TypedMetaPatch / MvuSchemaApply /
  ChronicleCompressor / StoryClock 由 Unsupported/Degraded 转 Supported；
  ActiveCampaignPersistence 保持 Degraded（进程内选择，文档化行为）；剩余
  Unsupported = CampaignLifecycle / CardCommands / CharacterCommands / ImportExport /
  KnowledgeTaskCommands / VariableCommands（§30.6）。

### 30.2 新增测试（先写失败/回滚测试，再实现）

| 测试 | 位置 | 覆盖 |
|---|---|---|
| 8 个 Meta UoW 单测（含 AfterFirstAction/AfterAllActions 回滚、scope 拒绝、stale definition、repoint 校验） | `sqlite_meta_repo.rs` | 单事务 + 故障注入回滚 |
| 6 个 MVU UoW 单测（含 AfterCardUpdate/AfterInstanceBackfill 回滚、跨 campaign 回填、NoChanges/归属拒绝） | `sqlite_mvu_repo.rs` | 单事务 + 故障注入回滚 |
| 6 个 compress job 单测（claim 原子、崩溃恢复、max-attempts、迟到结果终态保护、uncovered 计数） | `sqlite_compress_jobs.rs` | 队列事务语义 |
| `sqlite_chronicle_compressor.rs` 集成（阈值→入队→claim→确定性 A→B 发布→迟到结果拒绝→发布故障注入回滚→job 回队→Running→Pending 恢复→迟到 worker 终态拒绝） | tests/ | 崩溃/迟到/revision 冲突故障注入 |
| `sqlite_meta_lifecycle.rs` 重写（健康→提案→原子 apply→回滚→scope 拒绝→WorldInfo 读写路由） | tests/ | Meta + WorldInfo SQLite 全链路 |
| `sqlite_mvu_translations.rs` 扩展（preview→apply→回滚→schema/instance 验证） | tests/ | MVU SQLite 全链路 |
| `reverse_export_gate4.rs`（story_clock 归一、pending outbox 阻止导出、world info/compress jobs/MVU round trip + 幂等） | infra-sqlite tests/ | 反向导出一致性 |
| domain story_clock 分歧/修复/非字符串 3 测 + infra 载入修复测 | domain / reverse_export_gate4 | 单一权威 + 可审核修复 |

### 30.3 验证证据（全部通过）

- `cargo fmt --all -- --check`：通过。
- `cargo check --workspace`：通过，零警告。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过。
- `cargo test --workspace`：76 个测试二进制全部通过（storyforge lib 394 passed /
  3 ignored；domain 313 passed；app-meta / app-agent / app-pipeline 等全绿）。
- `cargo test -p storyforge --test sqlite_optin_lifecycle`、`sqlite_preaccept_production_lifecycle`、
  `sqlite_meta_lifecycle`、`sqlite_mvu_translations`、`sqlite_chronicle_compressor`：各 1 passed。
- `cargo test -p storyforge-infra-sqlite`：全绿（含新增 reverse_export_gate4 4 项）。
- `node scripts/architecture/backend-baseline.mjs`：175/175 命令、162 invoke、0 missing、
  `applicationMethodFlagReferences=0`、**`unsupported: []`**（四个过时标记族归零）。
- `node --test frontend/tests/tauri-command-contract.test.mjs`：8/8（Gate 4 断言
  `unsupported.length === 0`）。
- `frontend npm test`：476 passed / 0 failed；`npm run build`：通过（仅既有 chunk 警告）。
- `git diff --check`：通过（仅 LF→CRLF 行尾提示）。

### 30.4 Gate 4 通过条件逐项核对

- [x] **SQLite 不再因 Meta、MVU、Chronicle 等核心操作回退或拒绝**：能力矩阵
  WorldInfo/TypedMetaPatch/MvuSchemaApply/ChronicleCompressor/StoryClock → Supported；
  `sqlite_meta_lifecycle` / `sqlite_mvu_translations` / `sqlite_chronicle_compressor`
  集成测试走通全链路。
- [x] **`rg "unsupported until an atomic SQLite Meta UoW"` 无生产调用点**：守卫函数
  已删除，全库 0 命中。
- [x] **`rg "sqlite backend skips"` 无核心能力跳过点**：两处跳过分支已由 SQLite-native
  实现取代，全库 0 命中（baseline `unsupported: []` 钉住）。
- [x] **所有新增事务均有真实 fault-injection rollback 测试**：Meta（2 注入点）、
  MVU（2 注入点）、Chronicle 发布（AfterParentInsert 注入 + job 回队）、compress
  job 终态（Running-only 保护），均有回滚断言。
- [x] **全量验证通过、工作树干净**（提交后）。

### 30.5 关键行为保持

- Gate 3 约束全程未破：commands/应用服务无新增 JSON/SQLite 判断（静态白名单测试
  通过）；backend 分派只在 facade / backend_workflows / sqlite_runtime；SQLite
  活跃时四个 JSON writer 无构造机会（facadeSelectedWriterConstructors=4 不变）；
  `applicationMethodFlagReferences=0` 不变。
- 前端 IPC 合同未改：命令 175/175、invoke 162、0 missing；前端契约测试 8/8。
- JSON 路径行为未变：typed patch JSON 写盘（全局锁 + 逐 action）、MVU JSON apply、
  JSON compressor worker、WorldInfo JSON 文件路径全部保留原语义；既有 JSON 测试
  全绿。
- story_clock 修复为确定性可审核（warning 日志），未破坏既有 `set_variable` 双同步。

### 30.6 诚实记录：仍为 Unsupported 的 SQLite 缺口（Gate 4 外）

> 二审（2026-07-31）修订：CharacterCommands、ImportExport 已按 Gate 3 交接
> §15.7 的范围补齐（见 §30.9 P1-4），从本清单移除；legacy Meta patch 的
> CharacterStore 回写分支已改走 facade（见 §30.9 P1-4 闭环），不再拒绝。

- CampaignLifecycle（create/fork/delete）、CardCommands、KnowledgeTaskCommands、
  VariableCommands 在 SQLite 下仍显式 Unsupported（capability 声明式
  fail-closed，不静默空列表）。
- 这些项的「unsupported 字段归零」要求在 Gate 4 通过条件之外；能力矩阵测试与
  baseline 均已钉住剩余集合，不构成静默回退。

### 30.7 提交

- Gate 4 完成提交（`5dfa719`）：未 push；SHA 以 git log 为准（本段不引用自身提交）。
- Gate 4 评审修复提交（二审，见 §30.9）：独立提交，未 amend；SHA 以 git log 为准。

### 30.8 下一阶段

Gate 5（迁移、等价与恢复）：对 Gate 4 新增的 world info / compress jobs / MVU 导出
回读路径做大数据与等价矩阵；随后 Gate 6 真实模型与平台证据。

### 30.9 Gate 4 评审修复（2026-07-31 二审）

> 一审判定 INCOMPLETE（六个问题：P1-1 至 P2-6），本段逐项记录修复与证据。
> 修复为独立提交（未 amend `5dfa719`），未 push。

**P1-1 Chronicle A→B→C 连续压缩中断（属实）**：worker 对每批结果复用同一
`job_id`，V003 `idx_chronicle_publication_jobs_job_id` 唯一索引把第二批误判为
重复/迟到结果丢弃，job 停在 Running。
- 修复：V007 迁移新增 `chronicle_publication_jobs.batch_index` 列并改唯一索引为
  `(job_id, batch_index)`；`PublishRequest.batch_index` 贯通 publication UoW /
  sqlite_runtime / worker（`enumerate` 逐批传号）；迟到结果判定区分 job 状态：
  终态 = 真迟到丢弃，仍 open = 重试部分完成 → `mark_failed_or_retry`（耗尽
  attempts 至 Failed），绝不卡 Running。
- 回归测试：新增 `crates/tauri-app/tests/sqlite_chronicle_multibatch.rs`
  （同一 job 下 A→B 与 B→C 两批均 Applied、同批次重复仍拒绝、job 可达
  Succeeded）；`sqlite_chronicle_compressor.rs` 故障注入改用未占用批次键，
  断言注入错误（此前被迟到拒绝抢先，注入实际未生效）。

**P1-2 Meta active-turn/revision 检查不在事务里（属实）**：先检查、再读快照、
后开事务，检查与写入间存在竞态窗口。
- 修复：`SqliteMetaRepository::apply_typed_patch_actions(…, expected_revision)`
  在**事务内**执行 active-turn 查询（turns 表活动态）与 revision 比对，与写入
  同一 UoW 原子化；`meta_accept_typed_patch_with_writer` 把提案盖章
  `campaign_revision` 传入写盘闭包（JSON 路径保持既有全局锁语义，双保险）。
- 回归测试：`revision_mismatch_rejected_without_writes`、
  `active_turn_blocks_accept_without_writes`（含真实 running turn 行）。

**P1-3 同一 patch 多变量更新互相覆盖（属实）**：每个 action 从事务开始时旧
campaign 快照克隆，后项覆盖前项。
- 修复：`apply_action` 接收 `&mut Campaign`，`UpdateCampaignVariable` 在同一
  可变对象上顺序应用。
- 回归测试：`multiple_campaign_variable_updates_all_apply`（两键 + 同键覆盖）。

**P1-4 CharacterCommands / ImportExport 范围缺口（属实）**：Gate 3 交接
§15.7 明确列入 Gate 4，交付未实现，能力矩阵仍 Unsupported。
- 修复（子代理 + 主会话闭环）：V007 新增 `characters` 角色库表（镜像 JSON
  CharacterStore）；sqlite_runtime 新增 save/list/get/delete/mutate_character、
  `delete_card_payload` 级联、`import_campaign_bundle_into_db` 单事务导入
  （`BundleImportFault` 三阶段注入回滚）；storage_backend 能力矩阵翻转 +
  14 个 facade 分派方法；characters.rs / import_export.rs 命令改走 facade
  （零新增 JSON/SQLite 判断）；bundle 导出/导入与 JSON 路径共用同一 DTO 与
  纯函数（`rewrite_bundle_ids`、`validate_bundle_summary_graph`）；
  commands/meta.rs legacy 世界书回写分支改走 facade（子代理遗留闭环）。
- 测试：`crates/tauri-app/tests/sqlite_character_lifecycle.rs`（角色库生命周期、
  世界书编辑、bundle roundtrip、三种 fault 整体回滚、级联删除、PNG 导出链）。

**P2-5 预演与 SQLite 落盘 is_temporary 不一致（属实）**：解除 definition 绑定
时纯函数预演设 `is_temporary=true`，落盘未设（SQLite 与 JSON 两处都漏）。
- 修复：`sqlite_meta_repo.rs` 与 `meta_typed.rs` 两处落盘路径补上。
- 回归测试：`repoint_to_none_marks_instance_temporary`。

**P2-6 story_clock 非字符串 authority 静默回退（属实）**：非字符串变量项被
当作无效权威、静默回退顶层字段；测试固定了该行为。
- 修复：`Campaign::repair_story_clock_authority` 改为三态
  `StoryClockRepair`（NoChange / FieldRepaired / InvalidAuthorityNormalized），
  非字符串 authority 显式归一化为顶层字段字符串并产出可审计日志；SQLite 与
  JSON 载入路径均接入；`current_story_clock()` 文档更新（损坏态与归一化结果
  一致，不做静默任选）。
- 测试：`test_story_clock_non_string_authority_is_normalized_not_silently_ignored`
  改写固定旧行为的测试。

**验证证据（全部通过）**：`cargo fmt --check` / `cargo check --workspace` /
`clippy --workspace --all-targets -D warnings` / `cargo test --workspace`
（47 个测试二进制，1754 passed）/ 7 个 sqlite 集成测试（optin、preaccept、
meta、mvu、chronicle_compressor、chronicle_multibatch、character_lifecycle）/
`backend-baseline.mjs`（162 invoke、0 missing、`unsupported: []`、
`applicationMethodFlagReferences=0`）/ 前端契约 8/8 / `npm test` 476/476 /
`npm run build` / `git diff --check` 全绿。V007 版本号断言同步至 7
（migrations / audit_snapshot / migration_concurrency / migration_readiness /
preaccept_lifecycle）。

### 30.10 Gate 4 三审修复（2026-07-31）

> 二审判定 INCOMPLETE（2 P1 + 1 P2），本段逐项记录修复与证据。独立提交，
> 未 amend，未 push。

**P1 Chronicle 崩溃恢复批次键不稳定（属实）**：worker 曾用本次运行的数组序号
作批次号——第一批 A→B 发布后崩溃，恢复运行只剩 B→C，会重新编号为 0，与已发布
批次冲突并反复重试直到耗尽。
- 修复：批次键改为**稳定语义值** `out.output_level.as_u8()`（B=1、C=2），与
  已发布批次绝不撞车；`PublishRequest.batch_index` 文档钉死「必须语义稳定，非
  数组序号」；崩溃恢复（Running→Pending 重置后重跑）天然幂等。
- 回归测试：新增
  `crates/tauri-app/tests/sqlite_chronicle_crash_recovery.rs`——A→B 发布后
  `compress_reset_running_to_pending` 模拟崩溃，恢复段 B→C 用稳定键发布成功、
  同键迟到拒绝、job 正常终态化；`sqlite_chronicle_multibatch.rs` 改为用
  `output_level.as_u8()` 派生键。

**P1 角色删除报告成功但只删一半（属实）**：`delete_character` 先删角色库记录，
随后级联（MVU / card / campaign）错误全被 `let _ =` 吞掉；SQLite
`delete_card_payload` 只清理 summaries/tasks/knowledge/instances/world_info，
未清理 turns/attempts/preaccept_outbox/mutation_commits/chronicle jobs，真实玩过
的 Campaign 会因外键删除失败却返回成功。
- 修复：
  - `sqlite_runtime::delete_character_full_cascade(id, extra_source_ids)` 单事务：
    角色库行 + 候选 source 级联 MVU + 卡 + `delete_card_cascade_tx`
    （依赖序完整清理 mutation_commits → chronicle publication/compress jobs →
    preaccept_outbox → turn_attempts → turns → conversations → summaries/covers →
    tasks/knowledge/instances/world_info → campaigns）。
  - `delete_card_payload` 复用同一 cascade，删除改为**依赖序全量**清理。
  - `commands/characters.rs::delete_character` 改经 facade
    `delete_character_full_cascade`，错误传播给调用方（`删除失败（已整体回滚）`）；
    顺带移除了不存在的 `vector_store.delete_by_character` 无效调用。
  - `storage_backend` 新增 `delete_character_full_cascade` facade 方法
    （JSON 分支保持既有级联语义）。
- 回归测试：`sqlite_character_lifecycle.rs` 新增 8b 段——真实带依赖
  （turn_attempts/mutation_commits/compress_jobs/preaccept_outbox）的 Campaign，
  `delete_character_full_cascade` 后所有依赖表归零。

**P2 SQLite 世界书编辑后 tool_ctx 不刷新（属实）**：
`rebuild_world_info_in_tool_ctx` 仍走 `json_character_store` 重建，SQLite 下失败
静默 return，编辑落库但写作上下文用旧世界书。
- 修复：改造为纯 backend-neutral facade 读取——活跃 Campaign 世界书走
  `storage.get_world_info`（SQLite 走 V006 表），角色库走
  `storage.list_characters`；失败路径改为告警日志（不再静默 return）。characters.rs
  全文件零 JSON store 直连（静态守卫测试钉住）。
- 测试：`lib_tests_campaigns.rs` 更新能力矩阵断言（CharacterCommands /
  ImportExport → Supported，SQLite facade 无 JSON writer）；既有 `character_delete_
  fails_closed` 静态守卫保持。

**验证证据（全部通过）**：`cargo fmt --check` / `cargo check --workspace` /
`clippy --workspace --all-targets -D warnings` / `cargo test --workspace`（全绿）/
8 个 sqlite 集成测试（含新增 crash_recovery）全过 / `backend-baseline.mjs`
（`unsupported: []`、`applicationMethodFlagReferences=0`）/ 前端契约 8/8 /
`npm test` 476/476 / `npm run build` / `git diff --check`。

### 30.11 Gate 4 四审修复（2026-07-31）

> 三审判定 INCOMPLETE（2 P1 + 1 P2），本段逐项记录修复与证据。独立提交，
> 未 amend，未 push。

**P1 SQLite 切换活动仍调用 JSON store（属实）**：`set_active_campaign` 先更新
活跃指针，随后在 WorldInfo Supported 时调 `json_campaign_store`——SQLite 下
JSON store 不存在，命令返回失败但指针已改、世界书未载入。
- 修复：整段改经 backend-neutral facade——`get_world_info` / `get_campaign` /
  `get_card` / `ensure_world_info_from_book` / 新 `resolve_character_world_info_
  template`（角色库内嵌书，JSON 复用既有语义、SQLite 读 V007 角色库）/
  `template_world_info_from_card`（ST 模板）。读取错误全部传播（`?`），不再
  静默。
- 测试：`lib_tests_campaigns.rs` 命令级测试
  `set_active_campaign_command_routes_world_info_through_facade`（JSON 后端
  真实命令成功 + 活跃指针设置 + 会话缓存断言）。

**P1 角色删除后应用内状态仍是旧的（属实）**：`delete_character` 数据库事务
完整，但删除成功后未清被删 Campaign 的 `active_campaign`、未与
`active_campaign_update` 锁协调、未删/失效会话缓存——ConversationStore 持续
返回已删除会话，后续修改可能重新写回。
- 修复：命令在删除前收集受影响 campaign ids 与绑定的 conv ids；删除成功后：
  在 `active_campaign_update` 锁内清活跃指针（若指向被删 Campaign），逐一
  `conv_store.delete(conv_id)`（删文件 + 清缓存，彻底消除重写回风险），
  tool_ctx 角色同步移除。顺带修复 affected 收集漏掉卡真实 source 的 bug。
- 测试：`delete_character_command_clears_active_pointer_and_conversation_cache`
  （JSON 后端真实命令：活跃指针清空 + 会话缓存清空 + tool_ctx 移除）。

**P2 世界书读取错误仍被静默吞掉（属实）**：`rebuild_world_info_in_tool_ctx` 的
`let Ok(book)` 忽略读取错误、回退角色库；且新级联事务只有成功测试、无
fault-injection 回滚测试。
- 修复：
  - `rebuild_world_info_in_tool_ctx` 改为显式 match——读取成功且非空 → 注入本局
    世界书；读取失败 → 告警日志后回退角色库（不静默）。
  - `delete_character_full_cascade` 新增 `DeleteCascadeFault::MidCascade` 注入点
    （事务内、commit 前）+ `fail_delete_cascade_for_test` setter。
- 测试：`sqlite_character_lifecycle.rs` 8c 段——注入 MidCascade 失败 → 角色/卡/
  campaign 全部保留（整体回滚），错误断言含注入消息。

**验证证据（全部通过）**：`cargo fmt --check` / `cargo check --workspace` /
`clippy --workspace --all-targets -D warnings` / `cargo test --workspace`（全绿）/
8 个 sqlite 集成测试 + 2 个新命令级测试全过 / `backend-baseline.mjs`
（`unsupported: []`、`applicationMethodFlagReferences=0`）/ 前端契约 8/8 /
`npm test` 476/476 / `npm run build` / `git diff --check`。

### 30.12 Gate 4 五审修复（2026-07-31）

> 四审判定 INCOMPLETE（1 P1 + 2 P2），本段逐项记录修复与证据。独立提交，
> 未 amend，未 push。

**P1 `set_active_campaign` 失败后仍会留下新指针（属实）**：旧实现先
`set_active_campaign_in_state` 提交活跃指针，之后才读世界书/卡/角色模板——
任意读取、解析或写入失败，命令返回错误但指针已改变（违反“命令失败不改变
状态”）。
- 修复：世界书读取 / 模板解析 / 惰性种子（`ensure_world_info_from_book`）全部
  **前移到指针提交之前**，结果暂存于 `prepared_book`；世界书就绪后才调用
  `set_active_campaign_in_state`（锁内重新校验存在性，删除无法使选择失效）；
  指针提交成功后才 `apply_campaign_world_info_to_tool_ctx`。任意前置步骤失败
  → 命令返回错误，指针保持原值、tool_ctx 不被改写。
- 测试：新独立二进制 `tests/sqlite_command_lifecycle.rs`——对目标 Campaign 的
  `campaign_world_info` 行注入非法 JSON payload（经 `sqlite_runtime::with_db_
  raw_write` 测试写钩子），`set_active_campaign` 必须失败，且活跃指针保持原值、
  tool_ctx 世界书不被改写。

**P2 所谓命令级测试实际是 JSON，不是 SQLite（属实）**：既有两个命令级测试
（`set_active_campaign_command_routes_world_info_through_facade` /
`delete_character_command_clears_active_pointer_and_conversation_cache`）用
`AppState::new_for_test()`，它固定 `StorageBackend::Json`；激活测试也未断言
`tool_ctx.world_info`，不能证明 SQLite 命令闭环成立。
- 修复：新增**独立 SQLite 命令测试二进制** `tests/sqlite_command_lifecycle.rs`
  （与既有 sqlite_* 集成测试同模式：`sqlite_runtime::activate` 进程全局 →
  单测试函数）：真实 SQLite AppState（`AppState::new_with_backend` + SQLite
  facade，`validate_runtime_authority` 校验路径一致）+ 真实命令
  （`storyforge_lib::set_active_campaign` / `delete_character`），覆盖：
  - `set_active_campaign` 成功：活跃指针置位 + `tool_ctx.world_info` 注入模板
    + 世界书已落库（V006 `campaign_world_info`）；
  - 失败原子性（见上 P1）：命令失败 → 指针不变 + tool_ctx 不改写；
  - `delete_character`：真实级联删除后活跃指针清空 + 会话缓存失效 +
    tool_ctx 角色移除 + 库内行删除。
- 可见性最小改动：`set_active_campaign` / `delete_character` 由 `pub(crate)`
  提升为 `pub`（签名不变），lib.rs `pub use` 导出；`AppState::new_with_backend`
  与 `storage()` 提升为 `pub`；命令级源码测试字符串锚定同步更新
  （`pub fn delete_character(`）。Gate 1 契约仍通过：lib.rs 不含任何 tauri
  command 属性字面量。

**P2 部分错误仍被吞掉（属实）**：`merge_global_entries_into_book_facade` 用
`list_characters().unwrap_or_default()`——读取失败静默丢失全局条目；`delete_
character` 收集受影响 Campaign/会话时用 `if let Ok` / `.ok()` 忽略错误——收集
失败后数据库仍删除，活跃指针与会话缓存可能漏清。
- 修复：
  - `merge_global_entries_into_book_facade` 改为返回 `Result`，读取失败传播
    （调用方 `resolve_character_world_info_template` 的 SQLite 分支同步改）。
  - `delete_character` 收集 `affected_campaign_ids` / `affected_conv_ids` 全部
    改为 `?` 传播——反查卡、列 Campaign、读 Campaign 任一步失败，命令返回错误
    而非“删除成功却漏清状态”。

**验证证据（全部通过）**：`cargo fmt --check` / `cargo check --workspace` /
`clippy --workspace --all-targets -D warnings`（0 警告）/ `cargo test --workspace`
（全绿，405 lib + 104 等）/ 9 个 sqlite 集成测试（含新
`sqlite_command_lifecycle`）全过 / `backend-baseline.mjs`
（`commandAttributes=175`、`registered=175`、`unsupported: []`、
`applicationMethodFlagReferences=0`）/ 前端契约 8/8 / `npm test` 476/476 /
`npm run build` / `git diff --check`。

### 30.13 Gate 4 六审修复（2026-07-31）

> 五审判定 INCOMPLETE（2 P1 + 1 P2），本段逐项记录修复与证据。独立提交，
> 未 amend，未 push。

**P1 SQLite 重启后不恢复角色到运行时（属实）**：`AppState` 启动恢复只走 JSON
`CharacterStore`（`character_store` 在 SQLite 下为 `None`），SQLite 的
`tool_ctx.characters` 恒为空数组，写作入口直接消费该数组——角色虽在 V007
角色库、重启后写作运行时看不到。
- 修复：启动恢复改为 backend-neutral——JSON 走 `CharacterStore::list()`，
  SQLite 走 facade `list_characters()`（读 V007 `characters` 表）。两种后端
  共用同一恢复逻辑（`stored_info_to_character` / `collect_world_info_for_active`）。
  SQLite 角色库读取失败显式告警、留空，不 panic。
- 测试：`sqlite_command_lifecycle.rs` 改为**先种子角色再构造 AppState**，断言
  构造后 `tool_ctx.characters` 含该角色（启动恢复回归）。

**P1 激活 Campaign 与删除角色仍有并发竞态（属实）**：激活命令写完指针后释放
`active_campaign_update` 锁、随后才更新世界书；角色删除先删数据库、之后才取
同一把锁。并发时理论窗口：激活提交指针、删除清指针、激活再把旧 Campaign
世界书写回 tool_ctx。
- 修复：`set_active_campaign_in_state` 新增 `after_commit` 钩子，在
  `active_campaign_update` 锁内、指针提交后立即执行；`set_active_campaign` 把
  tool_ctx 世界书写入移入该钩子——指针提交与世界书注入成为原子单元，删除线程
  无法在两者之间插入清指针。`playthrough_lifecycle` 两处调用传空钩子。
  同时 `active_campaign_update` 字段由 `pub(crate)` 提升为 `pub`（锁句柄，
  供独立 SQLite 并发测试作可控屏障；无分派逻辑）。
- 测试：新增独立二进制 `tests/sqlite_command_concurrency.rs`——主线程持有
  `active_campaign_update` 锁，删除线程先删库（级联删 Campaign A）后阻塞在锁
  上，激活线程对已删 A 重新激活也阻塞；释放锁后串行完成。断言最终一致性：
  指针绝不指向已删除的 A、tool_ctx.world_info 绝不残留 A 的书、A 行已删。
  （诚实记录：五审代码中 `apply_campaign_world_info_to_tool_ctx` 的 active
  检查 + 删除后 `rebuild_world_info_in_tool_ctx` 覆盖已使该竞态难以确定性触发；
  锁内原子化消除理论窗口，测试作为最终一致性回归护栏。）

**P2 新测试两条假阳性断言（属实）**：测试在 AppState 创建后才写角色，删除前
`tool_ctx.characters` 本就为空，`all(...)` 自然通过；用角色名 "Elena" 查询，
但 SQLite 角色库只按存储 id / source id 查询，删除前该查询已是 None。
- 修复：种子移到 AppState 构造**之前**（启动恢复真正载入角色），删除后断言
  `tool_ctx.characters` 从含 Elena 变为不含；`get_character` 改用 `source_id`
  查询。

**验证证据（全部通过）**：`cargo fmt --check` / `cargo check --workspace` /
`clippy --workspace --all-targets -D warnings`（0 警告）/ `cargo test --workspace`
（全绿，405 lib + 81 个测试套件 ok）/ **10 个 sqlite 集成测试**（含新
`sqlite_command_concurrency`）全过 / `backend-baseline.mjs`
（`commandAttributes=175`、`registered=175`、`unsupported: []`、
`applicationMethodFlagReferences=0`）/ 前端契约 8/8 / `npm test` 476/476 /
`npm run build` / `git diff --check`。

Gate 5（迁移、等价与恢复）：对 Gate 4 新增的 world info / compress jobs / MVU 导出
回读路径做大数据与等价矩阵；随后 Gate 6 真实模型与平台证据。
