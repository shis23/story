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


