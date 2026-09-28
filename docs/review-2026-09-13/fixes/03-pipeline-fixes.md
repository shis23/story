# 修复记录：域3 写作流水线（task-10，W-01..W-32）

- 修复人：review-pipeline（域3）
- 任务：task-10「修复域3：写作流水线（W-01..W-32 全量修复）」
- 依据：`docs/review-2026-09-13/03-writing-pipeline.md`（域3 审查报告，P0=0 / P1=6 / P2=20 / P3=6）
- 起始 HEAD：`ab894c6`
- 工作区状态：多域并发修复中（同一工作树由 7 个域 + Lead 并行修改；跨域瞬时红属正常）

## 0 状态汇总

| 状态 | 数量 | 条目 |
| --- | --- | --- |
| 已修复 | 24 | W-01、W-02、W-03、W-04、W-05、W-06、W-07、W-09、W-10、W-15、W-16、W-19、W-20、W-21、W-22、W-23、W-24（R2 更正：删死熔断机制）、W-25、W-28、W-29、W-32 |
| 部分关闭（R2 复检新状态） | 4 | W-08（同名静默取首个仍有生产调用方）、W-12（写回过滤未成为契约字段）、W-14（无角色维度 + 注释已更正）、W-30（③ 合法空仍被当失败） |
| 已修复（降级方案） | 2 | W-11（上游已修 + 下游 tauri-app 写入门禁见「遗留」）、W-27（文档/多出现扫描已修；`owner_hint` 判定非问题） |
| 部分修复 + 判定非问题 | 1 | W-17（第 1 半已修；第 2 半为设计意图，附证据） |
| 暂缓（产品决策） | 2 | W-13（摘要受众隔离：需 `RoundSummary.audience` → 域1 schema + 存储迁移）、W-14 历史归档 `campaign_id` 回填（无标签记录无法归属，需产品决定是否丢弃/后台补标） |
| 移交他域 | 1 | W-26→task-f7（文档） |
| 判定非问题 | 2 | W-18（R9 收口：显式档位优先是书面契约，`None` 路径为 API 保留且有 6 条测试覆盖 → 保留不删）、`owner_hint`（归入 W-27） |
| 已由他域修复 | 1 | W-31（域5 于 R2 收尾修前端：`stores/writing.js:22` 从目录派生 + `:68` 拒绝未知档位；残留死分支 `adapter/useWritingScreenAdapter.js:89` 归前端任务） |
| 顺带修复（跨域请求） | 1 | D-04 接口约定（域1 已落地域侧语义，本域改为传真实 `story_clock`） |

**核实**：W-01..W-32 每条都在本文 §1/§2 有独立条目与状态；P0 = 0（原报告即无 P0）。

> **R2 复检更正入口**：本轮（task-34）对 W-24 的原条目做了**内容更正**（原描述写的是 W-07 的取消映射，不是 W-24 本身），
> 并把 W-08/W-12/W-14/W-30 从「已修复」降为「部分关闭」——全部记在 **§13 R2 复检更正**（含 file:line、反证与未关部分）。
> 原条目文本保留可见，不静默改写历史。

---

## 1 P1 逐条（ID → 修法 → 测试 → 状态）

### W-01 实例查找大小写/空白不一致 → 已修复（应用侧）

- **修法**：`crates/app-agent/src/runtime.rs` 新增
  - `pub(crate) fn instance_name_matches(candidate, needle) -> bool`（trim + `eq_ignore_ascii_case`）
  - `pub(crate) fn find_instance_normalized<'a>(runtime: &'a CampaignRuntimeContext, value: &str) -> Option<&'a CharacterInstance>`
    （顺序：id 精确 → id 忽略大小写 → name 归一化；不做模糊/前缀匹配）
  - `spawn_subagents` 改用 `find_instance_normalized`（原先直接调域侧精确匹配）。
- **测试**：`runtime::tests::test_find_instance_normalized_matches_case_and_whitespace_variants`、
  `runtime::tests::test_spawn_subagents_normalizes_case_and_whitespace_variant`。
- **边界声明**：域侧 `crates/domain/src/campaign_runtime.rs::find_instance_by_id_or_name`（精确、大小写敏感）
  **未改动**——它是域1 的文件与公共契约（域1 报告 D-01/D-02 有自己的裁决），本域只在应用侧收敛调用点，
  避免跨域争抢同一符号。→ 状态：**已修复（应用侧）**。

### W-02 `parse_plan_json` 畸形/重复 subagent_task → 已修复

- **修法**：`crates/app-pipeline/src/lib.rs::parse_plan_json`
  - `filter_map` + 跳过空白 `character_id` 与缺 `character_id` 的任务，逐条 `warn!`（`summarize_invalid_task` 截断 120 字原始 JSON，便于定位模型输出）；
  - 按 `character_id` 小写去重（保留首个），杜绝同一角色重复开戏；
  - 不再为缺失 id 生成 "unknown" 幽灵实例。
- **测试**：`tests::parse_plan_json_skips_blank_character_id_and_dedups`（5 条输入 → 2 条有效）。
- **状态**：已修复。

### W-03 非 Campaign 整卷重 roll 丢失 `generation_mode` → 已修复

- **修法**：新增纯函数 `effective_reroll_mode(requested: Option<&GenerationMode>, previous: Option<&GenerationMode>)`
  = `requested.or(previous).cloned()`；`regenerate` 用它算 `effective_mode`（前端非 Campaign 会话整卷重 roll 会传
  `None`，旧实现直接落回 legacy `big_scene` 全流程，产物与来源模式漂移）。
- **测试**：`tests::effective_reroll_mode_inherits_previous_and_prefers_request`
  （None+Continuation→Continuation；Some(Duet) 覆盖 previous；None+BigScene→**保持 legacy BigScene**；None+None→None）。
- **状态**：已修复。

### W-04 质量门禁误杀正常中文对白 → 已修复（精度优先）

- **修法**：`crates/app-pipeline/src/quality_gate.rs`
  - `STRICT_PATTERNS`（"作为AI"/"作为 AI"/"作为人工智能"/"以下是为您创作"/"以下是故事"/"现在开始创作"/
    "根据你的要求"/"按照你的要求"/"我将为你"/"我来为你"）**始终 Error**，不看上下文；
  - `AMBIGUOUS_PATTERNS`（"让我来"/"好的，我"/"没问题，我"/"我来写"）仅当
    `!inside_dialogue(text, idx)` **且** 匹配后 16 字符内出现元提示词
    （`META_CUES` = 为你/为您/以下/创作/写作/要求/正文/内容）时才 Error；
  - **删除 `at_line_start` 单独判 Error 的规则**（行首本身不足以判定元描述，会误杀对白）。
- **测试**（Lead 复检 R2 两组输入都已覆盖）：
  - `quality_gate::tests::test_dialogue_meta_phrases_are_not_errors`（对白内元短语不报）
  - `quality_gate::tests::test_bare_dialogue_phrases_are_not_errors_but_strict_meta_still_is`
    （"让我来帮你"/"好的，我这就去"/"没问题，我马上到" → 无 Error；"作为AI，我来帮你润色这一段。"/"以下是故事正文。" → Error）
  - `quality_gate::tests::test_assistant_preamble_still_errors`
- **状态**：已修复。**取向声明**：门禁阻断 accept，故取精度优先（宁可漏报也不误杀），
  真元描述由 STRICT 集兜住。

### W-05 后处理单条畸形条目整批丢弃 → 已修复

- **修法**：`crates/app-agent/src/postprocess.rs`
  - `PostProcessDto` 三个数组改为 `Vec<serde_json::Value>`；
  - `parse_entries<T>(entries, kind)` 逐条 `serde_json::from_value`，失败 `warn!` + 跳过，其余条目保留；
  - `postprocess_dto_from_value`：对象含 ≥1 已知键即算命中；已知键存在但**非数组** → `warn!` + 该类别空（仍算解析成功）。
- **测试**：`postprocess::tests::malformed_single_entry_does_not_drop_other_classes`、
  `postprocess::tests::non_array_field_is_empty_and_still_parses`。
- **状态**：已修复。

### W-06 归档水位线非连续 → 已修复（fail-closed 连续前缀）

- **修法**：`crates/app-memory/src/archiver.rs`
  并发结果按 `batch_idx` 收 `slots: Vec<Option<_>>`，返回**连续成功前缀**，遇首个缺口 `break` + `warn!`
  （宁可下一轮重摘一次摘要，也不产生水位线空洞让摘要永久丢失）；embed/upsert 失败同样截断并只对
  已成功 upsert 的摘要回填 `summary.vector`。
- **测试**：`archiver::tests::archive_prefix_stops_at_first_failed_batch`、
  `archiver::tests::archive_prefix_drops_summaries_when_embed_fails`、
  `archiver::tests::archive_prefix_clamps_zero_concurrency`。
- **调用方**：`crates/tauri-app/src/commands/conversations.rs` 以 `max(source_range.1)+1` 推进水位线，
  前缀语义下不需要改动（已确认）。
- **状态**：已修复。

### 补充 D-04（跨域接口约定）：story_clock 传入真实时钟

- **根因修复（本域职责，已落地）**：`compile_turn_dossier(intent, runtime, pending_tasks, story_clock: &str, max_full_actors)`
  新增参数；两处调用点（duet / unified）由旧硬编码 `""` 改为 `&ctx.story_clock`。
- **最终语义（一句话）**：StoryTime 触发器用**真实**故事时钟判定——匹配 → `Satisfied`（注入「已满足条件的任务/伏笔」组）；
  不匹配 → `NotSatisfied`（不注入）；**时钟缺失 → `NeedsAgentJudgment`，注入「待判断的任务/伏笔」组**，
  由 Agent 判断是否适用（既不是"无条件注入"，也不是"静默判否"）。渲染分两组标题 +
  "不适用则忽略，不要提前揭示后续剧情"指令由**域1** `render_tasks_for_injection` 落地（2026-09-13 已入库，
  域1 自测 388 passed）；本域负责**根因修复**（两处调用点改传 `&ctx.story_clock`）与分类语义不被改写。
- **测试**：
  - `turn_dossier::tests::story_time_task_injected_only_with_matching_real_clock`
    （匹配 → 落在「已满足」组且不混入「待判断」；不匹配 → 不注入）
  - `turn_dossier::tests::story_time_task_without_clock_goes_to_pending_judgment_group`
    （空时钟 → `check_trigger == NeedsAgentJudgment`，落在「待判断」组、**不在**「已满足」组，
    且该组带"不要提前揭示"指令）
- **状态**：已修复（根因）；域侧渲染分组为兜底，不替代传参修复。

---

## 2 P2 / P3 逐条

### W-07 顺序班组失败被吞（P2）→ 已修复

- `crates/app-pipeline/src/sequential_crew.rs` 新增
  `pub(crate) enum SequentialActorOutcome { Cancelled, SceneClosed, Failed(String) }` +
  `into_agent_error()` / `impl From<AgentError>`；`run_sequential_crew` / `_suffix` 返回
  `Vec<Result<Performance, SequentialActorOutcome>>`，区分「取消」「场景已关闭（正常跳过）」「真失败」。
- 无法新增 `AgentError` 变体（`crates/tauri-app/src/error.rs:90-108` 对其穷举匹配，属域2/域4 文件），
  故 `Cancelled`/`SceneClosed` 映射为 `AgentError::Cancelled` 供下游；pipeline 内日志文本区分 skip 与 failure。
- `crates/app-pipeline/src/lib.rs` 三处调用点（duet / suffix / unified）处理三类结果：
  `SceneClosed` → `info!` 跳过；其余 → `error!` + `into_agent_error()`。
- 测试：`sequential_crew::tests::cancel_during_attempt_is_reported_as_cancelled_not_subagent_failed`、
  `sequential_crew::tests::scene_close_marks_remaining_actors_as_skipped`。

### W-08 `get_character` 参数与歧义（P2）→ 部分关闭（R2 复检）

- `crates/app-agent/src/tools.rs`：query trim + 空值 `ToolError::BadArgs`；id 精确 → id 忽略大小写 → name 归一化；
  命中 >1 → `ToolError::BadArgs(format_ambiguous_instance(...))`（不再静默取首个）。
- 测试：`tools::tests::test_get_character_rejects_ambiguous_duplicate_name`、
  `tools::tests::test_get_character_normalizes_case_and_whitespace_variant`。
- ⚠ **R2 复检：仍有一半未关**——`get_character` 工具这一侧收紧了，但**同名多实例静默取首个**的底层函数
  `CampaignRuntimeContext::find_instance_by_id_or_name` 仍是「精确 id → 第一个 name 命中」，且仍有生产调用方
  （`crates/app-conversation/src/lib.rs:951`、`crates/tauri-app/src/commands/meta_conversation.rs:546/606/655`）。
  详见 §13.2。

### W-09 空结果触发第二次完整 LLM 调用（P2）→ 已修复（行为变更）

- `run_postprocess` 的第二次 direct-JSON 调用改为**仅在 `!parse_succeeded`** 时发生：
  合法空结果 = "本轮无更新"，是成功。旧测试 `test_run_postprocess_retries_..._returns_empty_json`
  已被新契约取代：`postprocess::tests::test_run_postprocess_empty_json_is_success_without_second_call`
  （断言空结果解析成功、三类更新为空、**fallback 调用次数 = 0**）。
- 真解析失败仍走 fallback：`test_run_postprocess_retries_direct_json_when_tool_path_drifts` 保持通过。
- 取消语义测试保持：`..._respects_cancel_before_direct_json_fallback` / `..._during_direct_json_fallback`。

### W-10 broadcast 解析（P2）→ 已修复

- `parse_broadcast(raw)`：trim + 小写，`all|全体|所有人|全部` → `All`；其余 trim 后 → `Group(name)`；空白 → `None`。
- 测试：`postprocess::tests::broadcast_normalizes_case_and_whitespace`（"  ALL "→All、"  守卫  "→Group("守卫")、"   "→None）。

### W-11 名字直塞 `Id` + 传播门禁 fail-open（P2）→ 已修复（降级方案，下游移交）

- **已修（本域）**：`normalize_postprocess_identities(&mut result, runtime)` 把知识/变量/任务里的名字
  归一为**唯一命中**的 `CharacterInstance.id`；未命中或同名多实例保留原值（不猜），由下游解析兜底。
- **测试**：`postprocess::tests::normalize_identities_resolves_unique_name_to_instance_id`。
- **未修（移交域2）**：`crates/tauri-app/src/production_postprocess.rs:1149-1157,1218` 的持久化写入
  仍对"来源缺失"fail-open——该文件不在本任务写入范围。→ 状态：**已修复（降级方案）**。

### W-12 MVU JS fallback 越权/未归一（P2）→ 部分关闭（R2 复检）

- `crates/app-pipeline/src/lib.rs` 新增 `push_mvu_js_variable_updates(updates, pp, campaign_runtime) -> (usize, usize)`
  （返回 `(applied, skipped)`）：
  - `is_reserved_mvu_key(key)`：trim + 小写后以 `__storyforge` 开头 → **跳过并回填 skipped 计数**
    （`mvu_js_fallback_drops_reserved_namespace_keys`，M-04 独立回归锁）；
  - key 经 `storyforge_domain::variables::normalize_mvu_key` 归一；
  - `scoped_variable_key(instance_id, key)` = `instance.<id>.<key>`，`parse_scoped_variable_key` 反解为
    `VariableUpdate.instance_id`（未知实例 id 保持 campaign 级字面键，不猜）。
- 测试：`tests::mvu_js_fallback_drops_reserved_namespace_keys`、`tests::mvu_js_scoped_key_targets_instance`。
- **M-04 归属**：域6（`review-meta-plugin`）报告 M-04 时该写回点在 `crates/app-pipeline/src/lib.rs`（本域文件），
  故由本域实现、域6 只做验收；域6 的 `06-meta-plugin-fixes.md` §10 已注明"不据为自有计数"。
  **独立理由**：M-04 的判据（保留位键必须过滤）由域6 的 MVU 侧发现，本域的 W-12 恰好覆盖同一函数面，
  合并实现避免同一处改两遍；前端镜像（`mvuExecuteResult.js` / `mvuStatTree.js` 的 `__storyforge` 排除）由域6 负责，
  两侧判据一致（大小写不敏感前缀）；若前缀规则变更需两侧同步。
- ⚠ **R2 复检：另有一半未关**——`present_chars` 仍只是 **prompt 输入**（`crates/app-agent/src/pipeline_postprocess.rs:58`
  `present_characters` 参数 → 提示词），`PostProcessOutcome`（同文件 `:29-38`）**没有**在场角色/过滤契约字段；
  真正的写回过滤只存在于 tauri-app 侧（`crates/tauri-app/src/commands/writing.rs:1370` 的 present_chars 校验、
  `:1618` 的"空集放行"逃生口）。任何绕开该 Tauri 分支的消费者（含测试夹具、将来的第二写回路径）都拿不到同一过滤契约。
  详见 §13.3。

### W-13 摘要注入每个子 Agent（P2）→ 暂缓（产品决策，R2 复检维持）

- **现状**：`crates/app-agent/src/runtime.rs:690-706` 把同一份近期摘要注入所有子 Agent，
  隔离只靠 prompt 软提示。
- **暂缓理由**：真正的按实例隔离需要摘要携带**受众（audience/instance 范围）**字段：
  `RoundSummary` 无该字段，改动面 = 域1 schema + SQLite/JSON 存储迁移 + 归档链路 + 前端展示，
  且历史摘要无受众信息只能默认全员可见（迁移期收益为 0）。这不是本任务（写作流水线）能安全收口的改动。
- **建议**：由域1 出 schema（`RoundSummary.audience: Option<Vec<Id>>`，None = 共享），
  存储层按 `None` = 旧语义回退；pipeline 注入时按当前 `CharacterInstance.id` 过滤。**暂缓**。

### W-14 远记忆检索 campaign 过滤 fail-open（P2）→ 部分关闭（R2 复检）

- `crates/app-memory/src/recall.rs::accepts_campaign`：带 `campaign_id` 过滤时，元数据**无 campaign 标签**
  的条目由"接受"改为"拒绝"（`debug!` 记录）；无过滤时行为不变。
- 测试：既有 `test_recall_archived_by_query_filtered_by_campaign` 已更新为
  camp-a 命中 1 条 / 无过滤命中 3 条。
- **迁移注意**：历史（打标签之前）归档不再出现在 campaign 作用域检索里——见 §6 遗留；
  R2 复检把历史 `campaign_id` 回填列为 **暂缓（产品决策）**（§13.6）。
- ⚠ **R2 复检：仍有一半未关**——（a）检索**没有角色维度**：`recall_archived_by_query_filtered`
  只过滤 `kind`/`campaign_id`/`min_score`（`recall.rs:188-210`），无法按 `CharacterInstance` 限定受众，
  因此"角色只应看到自己经历过的摘要"这一隔离在远记忆层并不成立（与 W-13 同源）。
  （b）`recall.rs:134-135` 原注释与实现相反（写着"或无 campaign 标签的旧记录"兼容回退，实现是 fail closed）
  ——**本轮已按实现修正注释**（`recall_archived_by_query` 无过滤 / `recall_archived_by_query_filtered` fail closed）。
  详见 §13.4。

### W-15 归档并发/批次溢出（P2）→ 已修复

- `batch_size.saturating_mul(trigger_count)`（防 usize 溢出回绕）；并发
  `.buffer_unordered(self.config.max_concurrency.max(1))`（防 0 并发死锁）。

### W-16 局部重 roll 目标重复（P2）→ 已修复

- `crates/app-conversation/src/lib.rs::validate_partial_roll`：目标按 `director`/`editor`/`subagent:{id}` 规格化后
  `HashSet` 去重，重复 → `PartialRollViolation("重 roll 目标重复：…")`；
  且当 `provenance.plan` 存在时，拒绝不在 `plan.subagent_tasks[].character_id` 里的 `Subagent(id)`。
- 测试：`tests::test_partial_roll_validation_rejects_duplicate_and_unknown_targets`。

### W-17 `replace_active_variant` 中间节点/非 campaign 提交（P2）→ 部分修复 + 判定非问题

- **已修**：`replace_active_variant` 要求目标是**最后一条**节点且其 active variant 是 Assistant，
  否则 `PartialRollViolation("replace_active_variant 只能用于对话最后一条 AI 消息；中间节点请用 add_variant 开分支")`。
  测试：`tests::test_replace_active_variant_rejects_middle_node`。
- **判定非问题（附证据）**：非 campaign 的 accept 短路（`CRATES/tauri-app/src/lib_tests_turns.rs:320`
  `accept_variant_non_campaign_keeps_legacy_behavior`）证明 legacy 路径的 accept **有意**非终态
  （保留 reroll/swipe 能力）；改成终态会破坏现有非 Campaign 写作路径（CLAUDE.md 硬规则）。

### W-18 big_scene 自动路由不可达（P2）→ 判定非问题（R9 收口，附书面契约 + 测试名）

- 证据指向 `frontend/src/composables/useWriting.js:139-141`、`frontend/src/stores/writing.js:57-62`、
  `crates/tauri-app/src/commands/writing.rs:927-936,855-869`——**全部不在本任务写入范围**。
- 本域已确认 pipeline 侧对 `big_scene` 的 legacy 兼容路径保留（W-03 修复后不再被默认落回）。
  → **移交域5（前端生成模式目录）与域2（命令层成本确认）**；本域无可修项。
- ⚠ **R9 复检（task-32）新结论：判定非问题（保留，不删）**——自动路由不是冗余死代码，而是
  **有书面契约的 API 级能力**：`docs/workstreams/WRITING-PIPELINE-V2-IMPLEMENTATION-2026-07-27.md:9/:66`
  明写"显式选择优先""**未传模式的 API 调用才进入自动路由**"；命令参数为 `Option<GenerationMode>`
  （`commands/writing.rs:877`），`None` 路径真实可达且被测试执行
  （`lib_tests_writing::start_writing_command_prompt_hook_messages_reach_mock_llm` 传 `None`），
  路由/守卫另有 4 条命令层测试（`automatic_route_promotes_large_roster_to_sequential_crew` 等）
  与 11 条域层测试。UI 层"永远不触发自动升档"是**设计取舍**（前端恒显式传档 + 展示预计调用量），
  不是接线遗漏；若要让 UI 用上，需前端功能开发（区分"未选档"与"选了 continuation" + 成本确认对话框）。
  详见 `round2/R9-unrecorded-pipeline.md` §2 与本文 §14.1。

### W-19 路径 C 子 Agent 配置硬编码（P2）→ 已修复

- 新增 `make_single_subagent_config(target_id, director_config, agent_profile_config)`：
  `max_tool_rounds` / `model` 取自 `AgentProfileConfig::run_config_for(&AgentRole::Subagent(id))`
  （含 Subagent 通配回退），无配置时保持历史默认（10 轮 + 导演模型）；`regenerate` 路径 C 使用它。
- 测试：`tests::effective_reroll_mode_inherits_previous_and_prefers_request` 同区新增的
  `tests::single_subagent_config_follows_profile_then_default`。

### W-20 Editor 修订稿空/严重缩水仍落盘（P2）→ 已修复

- `crates/app-pipeline/src/draft_revision.rs`：`MIN_REVISION_RATIO_DEN = 4`、`MIN_REVISION_COMPARE_CHARS = 200`；
  `revision_text_is_acceptable(original, revised)`：空稿直接拒绝；原文 ≥200 字且修订 <25% 拒绝。
  `revise_draft` 返回 `PipelineError::InvalidState("Editor 修订稿为空或相对原文严重缩水")` + `warn!`。
- 测试：`draft_revision::tests::{empty_revision_is_rejected, truncated_revision_is_rejected, short_draft_ratio_is_not_enforced, normal_revision_is_accepted}`。

### W-21 否定后肯定/私有泄漏（P2）→ 已修复

- `check_negation_then_affirmation` 扫描**所有** `"不是"` 出现位置（旧实现只看第一处）；
  `is_attributed_private_leak` 的 `has_other` 判定仅在 `!has_owner` 分支生效。

### W-22 `tool_center.rs` 死模块与幻影工具名（P2）→ 已修复

- 删除 `crates/app-agent/src/tool_center.rs` 及 `lib.rs` 的 `pub mod tool_center;`
  与 `pub use tool_center::{ToolCenter, ToolScope, ToolSummary, role_matches};`。
- 测试：`tools::tests::registered_tool_names_are_flat_and_free_of_tool_center_phantoms`
  （Director/Subagent 注册表都含 `get_character`，且**无点记法**幻影名）。
- **文档同步**：`docs/ROADMAP.md:77` 变得过期 → §5。

### W-23 数组/对象边界扫描（P2）→ 已修复

- `crates/app-agent/src/character_extractor.rs`：`try_extract_bracket_array` 改用新的
  `find_matching_bracket(body)`（字符串/转义感知），只取**第一个配平数组**；未配平时退化为全文（保持兼容）。
- `crates/app-agent/src/chronicle_compressor.rs`：`extract_json_array` 重写为"首个配平 `[...]`"
  （删除 `rfind` 造成的尾对象吞并）。
- 测试：`character_extractor::tests::test_parse_layer5_ignores_objects_after_closing_bracket`、
  `chronicle_compressor::tests::parse_compress_json_array_stops_at_first_balanced_bracket`。

### W-24 取消被报成 SubagentFailed（P2）→ 已修复（R2 更正：删死熔断机制）

> ⚠ **R2 复检更正**：本条**原描述是错误的**——"attempt 循环的取消信号置 `cancelled_by_signal`、最终映射
> `SequentialActorOutcome::Cancelled`"描述的是 **W-07**（见 §2 W-07 与其测试
> `sequential_crew::tests::cancel_during_attempt_is_reported_as_cancelled_not_subagent_failed`），与 W-24 无关。
> W-24 的真实内容是**连续失败熔断字段是死代码**：`SequentialStageRecord.failures` 只写不读。
> 原状态「已修复」是**错挂**（记录不诚实，已在 §13.1 记明）。

- **真实位置（更正前）**：`crates/app-pipeline/src/sequential_crew.rs` 的 `SequentialStageRecord`：
  生产路径调用 `record_failure(...)` 累加 `failures: HashMap<String, u8>`，但 `failure_count` / `should_stop_actor`
  只在 `#[cfg(test)]` 下编译、主循环从不读取该计数 —— 字段自身的注释即自认
  「生产路径只 record 不读……保留为带观测语义的死字段」。
- **本轮处置（删除，而非接线）**：删除 `failures` 字段、`record_failure` 方法、两个 `#[cfg(test)]` 访问器
  与 3 处错误路径调用点（`missing_result`/`parse`/`agent`），并删除只验证这两个访问器的两条测试
  （`failed_actor_does_not_erase_existing_public_stage_record`、`actor_is_stopped_after_two_failures`）。
  **行为为零变更**（被删字段此前从不被读；`last_error` 仍带全部错误文本进入最终 `Failed` 结果）。
- **为什么不接线**：actor 已受 `MAX_ATTEMPTS_PER_ACTOR = 2` 的有界重试约束；把"失败 2 次即熔断"接进主循环
  会把尝试次数从上限 2 降为 2 次触发即停（等于改产品重试策略、影响成文质量），属**未定策略**，
  不做无 spec 的行为变更。若将来要真熔断：在 `:335-359` 的失败分支后读 `should_stop_actor`，
  把该 actor 直接判 `SequentialActorOutcome::Failed("熔断")`，并补端到端测试。
- 门禁：`cargo test -p storyforge-app-pipeline` → 139 passed / 0 failed（删 2 条死测试后由 138 → 136，
  R5-01 新增 3 条 → 139）。

### W-25 变量键跨实例串味（P2）→ 已修复

- `build_current_variables` 在 flat 键之外追加 `instance.<id>.<key>` 别名（同一变量两种取法）；
  跨实例同名键冲突 `debug!` 记录；写回侧 `parse_scoped_variable_key` 负责反解到 `instance_id`。
- 测试：`tests::mvu_js_scoped_key_targets_instance` + 既有 `test_build_current_variables_with_instances` 更新为 4 条
  （2 flat + 2 别名）。

### W-26 文档漂移 `PipelineState` / 变体清单（P3）→ 移交（task-f7）

- 位置：`docs/AGENT_INTERFACES.md:262-284`。本任务硬性写入范围**禁止改 `docs/**`**（本文除外）→ 归 task-f7 文档同步。

### W-27 质量门禁死代码/文档（P3）→ 已修复 + 判定非问题

- 已修：删除 `at_line_start` 死规则并更新注释（与 W-04 同一处改动）；多出现点扫描（与 W-21 同）。
- **判定非问题**：`owner_hint` 字段并非可证死代码——它由 `quality_gate.rs` 的调用方向门禁传递并参与
  报告文本组装，无证据表明"永不读取"；删除会改变公开报告结构。附证据：字段构建与消费点均在
  `crates/app-pipeline/src/quality_gate.rs` 内可见。

### W-28 取消 → `PostProcessFailed`（P2）→ 已修复

- `crates/app-pipeline/src/lib.rs::run_postprocess`：保留 `cancel_probe = cancel.clone()`，
  `outcome.post_process == None` 时先判 `*cancel_probe.borrow()`：
  已取消 → 发 `PipelineEvent::PostProcessSkipped { reason: "流水线已取消…" }`；
  未取消且开关打开 → 才发 `PostProcessFailed`（reason 去掉"或被取消"）；
  明确关闭 → `PostProcessSkipped`（原行为不变）。
- 测试：`tests::test_postprocess_cancelled_emits_skipped_not_failed`（调用前取消 → 无
  `PostProcessFailed`、有 `PostProcessSkipped`）。

### W-29 `llm_parse` 字节边界 panic / 静默丢参（P2）→ 已修复

- `match_braces`：`content.get(pos..)?` 取代直接切片（非 char 边界不再 panic）。
- `from_tool_call`：arguments 非法 JSON → `tracing::warn!(target: "llm_parse", …)`（raw 截断 200 字）
  并**继续扫描后续 tool_call**，不再整段放弃。
- 测试：`llm_parse::tests::match_braces_non_char_boundary_returns_none`、
  `llm_parse::tests::from_tool_call_skips_malformed_arguments_and_keeps_scanning`。

### W-30 解析失败与"合法空"混淆（P2）→ 部分关闭（R2 复检）

- `parse_character_definitions_from_response` 改为三态：
  工具调用 `characters: []` / content `[]` → `explicit_empty` → `Ok(vec![])`；
  5 层全 miss → `Err("5 层兜底全miss；content 前 200 字: …")`。
- 另：progress 排水任务 `drain.await` 的 `Err` 由静默丢弃改为 `warn!`（不影响抽取结果）。
- `crates/app-agent/src/chronicle_compressor.rs`：解析失败走确定性兜底时，`CompressRunOutcome`
  新增 `degraded: bool` + `degraded_reason: Option<String>`（旧实现把"降级产出"伪装成正常产出，
  调用方无法区分）。
- 测试：`character_extractor::tests::test_empty_array_is_parsed_not_reported_as_miss`、
  `chronicle_compressor::tests::deterministic_fallback_is_marked_degraded`。
- ⚠ **R2 复检：③ 仍有未关的一半**——解析层已能区分"显式空"与"5 层全 miss"，但 `extract_characters`
  在**合法空**上仍返回 `Err`：`crates/app-agent/src/character_extractor.rs:75-78`
  （`if defs.is_empty() { return Err(ExtractError::Parse("识别结果为空")) }`），
  于是调用方 `crates/tauri-app/src/commands/campaigns.rs:165-169` 走进 `Err` 分支 →
  `tracing::warn!("角色识别失败…")` + `fallback_character_extraction(...)`，把"这张卡确实没有可抽取角色"
  报成"识别失败 + 降级为源卡单角色卡"（`extraction_status` 也是失败态）。详见 §13.5。
  修复需动 `CharacterExtractionStatus`（领域枚举，序列化给前端）或多个命令层分支 → 超出本轮写入范围，留待产品/接口决策。

### W-31 前端 `validGenerationModes` 含 `big_scene`（P3）→ 移交（域5）

- 位置：`frontend/src/stores/writing.js:16-21,57-62`、`frontend/src/utils/generationModes.js:1-21`
  → **前端文件，本任务禁止改**。→ 移交域5。

### W-32 `format_subagent_context_*` 重复实现（P3）→ 已修复

- `crates/app-agent/src/runtime.rs` 的 `format_context_stable/volatile` 提升为 `pub` 并经 `lib.rs` 导出；
  `crates/app-pipeline/src/lib.rs` 删除两份逐字节重复实现，改为 `#[inline]` 委托调用（单一实现）。
- 回归：**无独立单元测试**（去重不改变输出，两实现去重前已逐字节一致）；委托后路径 C 由
  `cargo test -p storyforge-app-pipeline --lib` 全量覆盖。若需更强保证，可补"同一 `ContextPackage` 输出快照"测试。
- ✅ **R9 收口（task-32）已补该测试**：`tests::path_c_context_formatters_delegate_byte_for_byte_and_match_golden`
  —— ① 委托等价（薄壳 vs app-agent 唯一实现，逐字节）；② **golden 字节锁**（分区标题/空行/`keys.join(", ")` 分隔符，
  直接影响 system/tail 分段与 LLM 缓存键）；③ 空包边界。失败可控已实测（改 `app-agent` 的 `"## 你的角色设定"`
  即红）。详见 `round2/R9-unrecorded-pipeline.md` §3 与本文 §14.2。

### 补记：D-04 之外本域无 P0

原报告 P0 = 0，本域未发现新 P0。

---

## 3 门禁（命令 + 退出码 + 通过数）

> 环境：Windows / PowerShell；`cargo` 多域并发编译，结果取自本任务完成时的最终一轮。

| # | 命令 | 退出码 | 结果 |
| --- | --- | --- | --- |
| 1 | `cargo check -p storyforge-app-agent -p storyforge-app-pipeline -p storyforge-app-memory -p storyforge-app-conversation --all-targets` | 0 | `Finished`，0 error（含测试目标） |
| 2 | `cargo check -p storyforge-app-pipeline --all-targets` | 0 | `Finished` |
| 3 | `cargo test -p storyforge-app-agent --lib` | 0 | 130 passed / 0 failed |
| 4 | `cargo test -p storyforge-app-pipeline --lib` | 0 | 138 passed / 0 failed |
| 5 | `cargo test -p storyforge-app-pipeline --lib quality_gate` | 0 | 24 passed / 0 failed（W-04 子集） |
| 6 | `cargo test -p storyforge-app-memory --lib` | 0 | 12 passed / 0 failed |
| 7 | `cargo test -p storyforge-app-conversation --lib` | 0 | 23 passed / 0 failed |

- **未运行（Lead 收口）**：`cargo test --workspace`（Lead 拥有）；前端门禁（`npm test` / `vitest` / build）
  在成员侧会 `spawn EPERM`，由 Lead 统一运行。本域改动**不含任何 `frontend/**` 文件**，
  故不存在"未经前端测试运行器验证"的前端改动。
- **编译红说明**：多域并发期间出现过跨域瞬时红（例：`crates/domain` 的 `is_known_rule_code` 签名、
  `story_task.rs:305 borrow of moved value: satisfied`、`ToolSpec` 字段名、`ToolCall.kind`），
  均为其他域在飞行中的改动或我本人同一轮的中间态；上表是各文件（含域1 落盘）之后的复跑结果。
  其中 `satisfied` 一条已由域1 修复（`for t in &satisfied`），修复后本表全部复跑通过。

---

## 4 变更文件清单

**app-agent**

- `crates/app-agent/src/runtime.rs` —— W-01（`instance_name_matches` / `find_instance_normalized`）、
  W-32（`format_context_stable/volatile` 提为 pub）
- `crates/app-agent/src/lib.rs` —— W-22（删除 tool_center 导出）、W-32（导出上述两函数）
- `crates/app-agent/src/tool_center.rs` —— **删除**（W-22）
- `crates/app-agent/src/tools.rs` —— W-08、W-22 回归测试
- `crates/app-agent/src/postprocess.rs` —— W-05、W-09、W-10、W-11
- `crates/app-agent/src/llm_parse.rs` —— W-29
- `crates/app-agent/src/character_extractor.rs` —— W-23、W-30
- `crates/app-agent/src/chronicle_compressor.rs` —— W-23 + `degraded`/`degraded_reason` 降级标记

**app-pipeline**

- `crates/app-pipeline/src/lib.rs` —— W-02、W-03、W-07、W-12、W-19、W-25、W-28、W-32、
  D-04 调用点、`make_single_subagent_config`、`effective_reroll_mode`
- `crates/app-pipeline/src/quality_gate.rs` —— W-04、W-21、W-27
- `crates/app-pipeline/src/sequential_crew.rs` —— W-07、W-24
- `crates/app-pipeline/src/draft_revision.rs` —— W-20
- `crates/app-pipeline/src/turn_dossier.rs` —— D-04（新增 `story_clock` 参数）

**app-conversation**

- `crates/app-conversation/src/lib.rs` —— W-16、W-17（第 1 半）

**app-memory**

- `crates/app-memory/src/archiver.rs` —— W-06、W-15
- `crates/app-memory/src/recall.rs` —— W-14

**记录**

- `docs/review-2026-09-13/fixes/03-pipeline-fixes.md` —— 本文

**未改动（有意）**

- `crates/domain/**`（域1 文件；W-01 域侧、W-13 schema、`check_trigger` 语义均由域1 负责）
- `crates/tauri-app/**`（除 `turn_dossier.rs` 无需）——W-11 下游、W-18 命令层移交域2
- `frontend/**`（域5）——W-18、W-31
- `docs/**`（除本文）——W-26 归 task-f7

---

## 5 文档同步项（交给 task-f7 / 文档 owner）

1. `docs/ROADMAP.md:77` —— `tool_center.rs` 已删除（W-22），相关表述需更新。
2. `docs/AGENT_INTERFACES.md:262-284` —— `PipelineState` 列表与"完整变体"清单缺项（W-26）；
   同时补 `PostProcessSkipped` 的取消语义（W-28）与 `SequentialActorOutcome` 的 skip/failure 区分（W-07）。
3. `docs/AGENT_INTERFACES.md:12` 与前端 `validGenerationModes`（W-31，域5 修完后一并更新）。
4. `compile_turn_dossier` 新增 `story_clock` 参数（D-04）——接口文档若列出该函数签名需同步。

---

## 6 遗留 / 移交 / 风险

| 项 | 类型 | 处理 |
| --- | --- | --- |
| W-11 下游 `production_postprocess.rs` 来源缺失 fail-open | 移交 | 域2（tauri-app）；本域已保证"上游尽量给实例 id" |
| W-13 摘要按实例隔离 | 暂缓 | 需域1 出 `RoundSummary` 受众 schema + 存储迁移；建议列为独立任务 |
| W-14 历史无标签归档不再进 campaign 检索 | 风险提示 | 需要产品决策：是否回填 `campaign_id` 标签；未回填则旧归档只在不带 campaign 过滤时可见 |
| W-18 `big_scene` 自动路由 | **判定非问题（R9 收口）** | 有书面 API 契约（`WRITING-PIPELINE-V2…md:9/:66`）；`None` 路径可达且有 6 条测试覆盖 → **保留不删**；UI 层不触发属设计取舍，若要落地需前端立项（见 §14.1） |
| W-26 文档漂移 | 移交 | task-f7 |
| W-31 前端模式校验 | 移交 | 域5 |
| W-01 域侧精确匹配语义保留 | 接口约定 | 已向 Lead 声明；如需域侧也归一，须域1 决策 |
| W-09 行为变更（空结果不再二次调用） | 需知会 | 已更新原测试为新契约；若产品希望"空结果也重试"，需回滚此条并说明成本 |
| D-04 两组渲染标题文本 | 跨域耦合 | 本域测试断言了域1 的标题文案（「已满足条件的任务/伏笔」/「待判断的任务/伏笔」）；域1 若改名需同步本域两条测试 |

- **未验证项（诚实声明）**：本域所有修复均通过上表 Rust 门禁；`cargo test --workspace` 与前端门禁由 Lead 收口。
- **无 P0 遗留**。

---

## 13 R2 复检更正（task-34，2026-09-13）

- **触发**：`docs/review-2026-09-13/round2/R2-pipeline-recheck.md` 指出本文若干状态与代码不符（N-R2-02/03/04/05/06/09/10/14）。
- **本轮处置**：逐条复核 → 更正记录 + 修可低成本修的项；其余给**未关部分 + file:line + 反证**。
- **记录诚实性声明**：本节的更正**不修改历史叙事**（原条目文本保留，只是在 §0/条目头标注新状态并在此写明更正）。
  原 W-24 条目的内容错挂与 W-31 的虚假归属属**记录不诚实**（非代码缺陷），本轮一并记明。

### 13.1 W-24：原条目内容错挂（记录不诚实）→ 已更正 + 删除死熔断机制

| 项 | 内容 |
| --- | --- |
| 原状态 | 「已修复」（本文 §2 W-24） |
| 复核结论 | **内容错挂**：原条目描述的是 W-07 的取消映射（`cancelled_by_signal` → `SequentialActorOutcome::Cancelled`），与 W-24 无关；W-24 的真实内容是**连续失败熔断字段是死代码** |
| 证据 | `crates/app-pipeline/src/sequential_crew.rs`（原 `:103-107` 注释自认"生产路径只 record 不读"；原 `:149-157` 两个访问器在 `#[cfg(test)]` 下）；`record_failure` 有 3 处生产调用（原 `:337/352/357`）但 `should_stop_actor` 无生产读者 |
| 本轮处置 | **删除**死机制（字段 + 方法 + 2 个 `#[cfg(test)]` 访问器 + 3 处调用点 + 只测这两个访问器的 2 条测试）。行为零变更（字段从不被读，错误文本仍经 `last_error` 进入最终 `Failed`） |
| 为何不接线 | actor 已有 `MAX_ATTEMPTS_PER_ACTOR = 2` 的有界重试；把"失败 2 次即熔断"接进主循环等于改重试策略/成文质量，属**未定产品策略**。若将来要接：失败分支后读阈值 → 直接判 `Failed("熔断")` + 端到端测试 |
| 新状态 | **已修复（删除死代码；R2 定的"未修/暂缓"已解除）** |

### 13.2 W-08 → 部分关闭

- **已关**：`get_character` 工具侧（query trim、空值 BadArgs、id 精确→忽略大小写→name 归一、命中 >1 → `BadArgs(format_ambiguous_instance)`）。
- **未关（域侧 helper 仍静默取首个）**：`crates/domain/src/campaign_runtime.rs:75-83` 的
  `find_instance_by_id_or_name` = 「精确 id → **第一个** name 命中」，既无归一也无歧义判定。
  生产调用方：`crates/app-conversation/src/lib.rs:951`（`SubagentSnapshot.character_instance_id` 绑定）、
  `crates/tauri-app/src/commands/meta_conversation.rs:546/606/655`（Meta 工具按名字解析实例）。
- **反证（后果）**：同名两实例（两个 "Guard"）时域侧返回列表第一个；`app-conversation` 用它填 `character_instance_id`
  → 演出可能绑定到**错误的实例**（有 `fallback_reason` 记录，但绑定已错），Meta 工具也会操作错对象。
- **未修原因**：改该函数会同时改变 4 处生产语义（3 处在 `tauri-app`，本轮写入范围只允许 `runtime_support.rs` 一处例外）。
- **建议**：新增 `find_instance_unique(value) -> Result<&CharacterInstance, AmbiguityError>`，调用方歧义时不绑定并写
  `fallback_reason`；或把 `find_instance_by_id_or_name` 标记为"调用方须自行查重"。**状态：部分关闭**。

### 13.3 W-12 → 部分关闭

- **已关**：MVU JS fallback 的保留位键过滤 + 键归一 + instance 作用域键（含 M-04）。
- **未关（写回过滤未成为契约字段）**：`present_chars` 只是 **prompt 输入**
  （`crates/app-agent/src/pipeline_postprocess.rs:58` `present_characters` 参数）；
  `PostProcessOutcome`（同文件 `:29-38`）**没有**在场角色/过滤契约字段。
  真正的过滤只在 Tauri 侧：`crates/tauri-app/src/commands/writing.rs:1370`（present_chars 校验）、
  `:1618`（"present_chars 为空集 → 放行"的向后兼容逃生口）。
- **后果**：任何不经该 Tauri 分支的消费者（测试夹具、将来的第二写回路径、Meta 写回）都拿不到同一过滤契约，
  空集放行语义随调用方漂移。**状态：部分关闭**。

### 13.4 W-14 → 部分关闭 + 注释已按实现更正

- **已关**：`accepts_campaign` 对无 campaign 标签的记录 fail closed（带过滤时丢弃 + `debug!`）。
- **本轮修的**：`crates/app-memory/src/recall.rs:129-139` 原注释写"`campaign_id` 若提供，只返回匹配**或无 campaign 标签的旧记录**（兼容历史）"
  ——与实现**直接矛盾**（实现是 fail closed）。已改为按实现描述：
  `recall_archived_by_query` **不做** campaign 过滤（无参数）；
  需要隔离时用 `recall_archived_by_query_filtered(..., Some(cid))`，无标签记录会被丢弃。
- **未关 1（无角色维度）**：`filter_archived_hits`（`recall.rs:188-210`）只过滤 `kind`/`campaign_id`/`min_score`，
  没有 `CharacterInstance` 维度 → "角色只应看到自己经历过的摘要"在远记忆层不成立（与 W-13 同源）。
- **未关 2（历史回填）**：见 §13.6 暂缓。
- **状态：部分关闭**。

### 13.5 W-30 ③ → 部分关闭

- **已关**：解析层三态（显式空 → `Ok(vec![])`；5 层全 miss → `Err`）；`drain.await` 的 `Err` 由静默改 `warn!`；
  `chronicle_compressor` 降级标 `degraded`。
- **未关（合法空仍在调用层与失败等价）**：`crates/app-agent/src/character_extractor.rs:75-78`
  `if defs.is_empty() { return Err(ExtractError::Parse("识别结果为空")) }`
  → 调用方 `crates/tauri-app/src/commands/campaigns.rs:165-169` 走 `Err` 分支：
  `warn!("角色识别失败…")` + `fallback_character_extraction(...)`，把"卡里确实没有可抽取角色"
  呈现为"识别失败 + 降级单角色卡"（`extraction_status` 也是失败态）。
- **未修原因**：要区分需动 `CharacterExtractionStatus`（领域枚举，序列化给前端）或多个命令层分支
  → 跨域接口决策，超出本轮写入范围。**建议**：抽取层把"合法空"返回 `Ok(vec![])`，
  命令层映射为独立状态（如 `Empty`）并在前端提示"未识别到角色，已按源卡建卡"。**状态：部分关闭**。

### 13.6 暂缓（产品决策）：W-13 摘要受众隔离 + W-14 历史归档 `campaign_id` 回填

| 项 | 暂缓理由 | 解除条件 / 建议 |
| --- | --- | --- |
| W-13（`RoundSummary` 受众隔离） | 隔离字段不存在：`RoundSummary` 无 audience/instance 范围；改造面 = 域1 schema + SQLite/JSON 存储迁移 + 归档链路 + 前端展示；历史摘要无受众信息，迁移期收益为 0 | 域1 出 `RoundSummary.audience: Option<Vec<Id>>`（None = 旧语义全员可见），存储按 None 回退，注入侧按当前 `CharacterInstance.id` 过滤 |
| W-14 历史无标签归档 | 回填需要"这条历史归档属于哪个 campaign"的信息，而**标签缺失时该信息不可恢复**：按时间/会话猜测会制造错误归属（把 A 战役的记忆注入 B 战役，比丢弃更糟）；丢弃则只是少召回 | 产品决定：① 维持 fail closed（现状，旧归档只在无过滤召回可见）；② 或提供显式"历史归档重建/重新打标"后台工具（需用户确认归属）。**不接受**静默按时间窗口回填 |

> 两条都不是本域能安全收口的改动（跨域 schema/存储/前端），按 R2 口径记为**暂缓（产品决策）**并保留触发条件。

### 13.7 R11：`PostProcessSkipped` 双发（本轮引入的回归）→ 已修复（单一权威仲裁）

- **现象（R2 N-R2-05 报告，本轮独立复现）**：取消后处理时前端收到**两个** `PostProcessSkipped`，
  与 `runtime_support.rs` 文档写下的"恰好一个"契约矛盾。
  根因：app-pipeline 的 `run_postprocess` 在 `cancel_probe` 为真时自己发一个 Skipped
  （`crates/app-pipeline/src/lib.rs:1630`），随后 Tauri 侧 `run_shared_postprocess_background` 在
  `*cancel.borrow()` 分支又发一个（`crates/tauri-app/src/runtime_support.rs`，W-28 映射）。
- **失败可控复现（修复前事件序列，实测）**：临时关闭仲裁后运行新测试，得到
  `[PostProcessSkipped{"postprocess cancelled"}, PostProcessStarted, SummaryDone, PostProcessSkipped{"流水线已取消，后处理未完成（best-effort，不阻断成文）"}]`
  —— **2 个终态事件**（且 Tauri 那个还早于 `PostProcessStarted`）。
- **同类未报告面**：同一双发结构也让**成功路径发 2 个 `PostProcessDone`**（pipeline Done + Tauri Done），
  **持久化失败路径先 Done 后 Failed**（前端先置 done 再收到失败）。本轮一并收口。
- **修复（单一权威 = Tauri 持久化层）**：只有 Tauri 层知道 outcome 是否真落盘，故由它作为生产路径终态事件的唯一权威；
  pipeline 的终态事件被**截留为兜底**（当 Tauri 分支不派生终态事件时使用：非 Campaign 路径、双开关关闭的 Skipped、
  legacy `campaign_id: None` 的静默路径），非终态事件（Started/SummaryDone/…）仍**立即转发**（进度显示不变）。
  实现：`spawn_pipeline_event_arbiter`（代理 channel + 转发任务 + 终态槽）→ 主体返回后
  `drop(proxy_tx)` → `forwarder.await`（确定性排空，无竞态）→ `if !terminal.sent { 用被截留的终态兜底 }`。
- **为何不改 app-pipeline 侧**：`run_postprocess` 的直接调用方还有 `sqlite_endurance.rs`、`m5_cache_and_memory.rs` 等
  独立消费者与其自带测试断言（"取消即 Skipped"），改它会破坏 pipeline 自身契约；仲裁在共享调用点一处收口。
- **测试（失败可控）**：
  `runtime_support::tests::cancel_during_runner_emits_exactly_one_terminal_event`
  （mock LLM 首次调用即置取消位 → 确定性命中"runner 内取消"，断言恰好 1 个终态 + 必须是 Skipped +
  `PostProcessStarted` 确实转发过（防退化成"早取消"场景））、
  `runtime_support::tests::arbiter_forwards_non_terminal_and_holds_terminal_events`（单元级：非终态即时转发、终态截留、后写覆盖）、
  既有 `early_cancel_emits_single_skipped_event` 保持通过。

### 13.8 R5-01（引号内「我将为你」误杀）+ N-R2-14（无引号「让我来为你倒茶」误杀）→ 已修复（判据收窄，精度优先）

- **问题**：`quality_gate.rs` 的 `STRICT_PATTERNS` 含「我将为你」「我来为你」，判定是**无条件 `contains`**：
  `「将军，我将为你赴汤蹈火。」` → `MetaDescription` **Error** → `QualityReport::blocks_accept(false)` 拦截采纳。
  同源的 N-R2-14：歧义模式线索表含「为你」，`让我来为你倒茶。` 也判 Error。
  两者都是**正常对白**（宾语是剧情里的人），却被当成助手自述。
- **新判据（可判定、非"恒不触发"）**：
  - STRICT 无条件模式只留真正不可能出现在对白里的自述（`作为AI`/`作为 AI`/`作为人工智能`/`以下是为您创作`/`以下是故事`/`现在开始创作`/`根据你的要求`/`按照你的要求`）；
  - 「我将为你」「我来为你」移入**条件严格**类：**必须"非引号语境"且命中点后 16 字内出现写作任务线索**
    （`写/创作/续写/生成/润色/改稿/正文/章节/故事/内容/安排`）才判 Error；
  - 歧义模式（「让我来」「好的，我」…）的线索表由 `META_CUES`（含「为你/以下/要求」）**收窄为同一写作任务线索表**
    → 「让我来为你倒茶」「满足你的要求」不再误杀；「让我来为你安排这一章的节奏」仍 Error。
- **边界与取舍**：引号内（含带写作动词的引用，如「我来为你写这封信」）**不判 Error** —— 引用/转述语境优先按对白处理；
  兜底仍由无条件 STRICT 模式负责（如「以下是为您创作」在不在引号内都命中）。精度优先与 W-04 同口径。
- **测试**：`test_quoted_first_person_promises_are_not_meta_errors`（正例：引号 + 无引号剧情承诺，含 `blocks_accept(false)==false`）、
  `test_assistant_task_promises_still_error_and_block_accept`（反例：非引号 + 写作线索仍 Error 且拦截）、
  `test_quoted_writing_task_promise_is_treated_as_quotation`（边界 + 无条件 STRICT 不受引号影响）、
  `test_person_object_cue_phrases_are_not_meta_errors`（N-R2-14 正反例）。
  既有 W-04 四条测试全部保持通过（`test_dialogue_meta_phrases_are_not_errors`、`test_assistant_preamble_still_errors` 等）。
- **未做（诚实声明）**：`R2 §6-低危-观察` 里"句首/标点边界"约束未引入（本轮的线索词判据已覆盖其反例）；如仍要更强精度需另立判据。

### 13.9 N-R7-01（= R2 N-R2-10）身份归一语义统一 → 已修复（单一归一函数）

- **问题（两侧结论相反）**：
  - domain：`campaign_runtime.rs:108/118` 用 `to_lowercase()` 且**不 trim**；
  - app-agent：`runtime.rs:608` 用 `trim() + eq_ignore_ascii_case()`；
  - 结果：`"Ähre"/"ähre"` 在 domain 侧"已存在"（不建临时实例）、在 app-agent 侧 miss；`" Alice "/"Alice"` 反之。
    `runtime.rs:604` 的注释当时已宣称"必须保持同一套语义"——**注释与事实不符**。
- **裁定（单一语义）**：`trim` + **Unicode 小写**，落在 domain 侧的
  `storyforge_domain::campaign_runtime::normalize_instance_identity`，两侧共用。
  - 选 `trim`：名称外侧空白是导入/LLM 输出噪声，不是身份差异（两侧都忽略）。
  - 选 Unicode 小写而非 ASCII：非 ASCII 名字（拉丁扩展/希腊/西里尔）的大小写变体是同一个人；
    ASCII-only 会静默 miss，正是 W-01 想消灭的失败模式。代价：极少数"外表不同但 Unicode 小写相同"的字符
    （如 KELVIN SIGN `U+212A` 与 `k`）会被并入同一键——在角色名域可接受，且两侧行为一致。
  - **不误合并优先**：土耳其 `İ`（`U+0130`）`to_lowercase()` = `i` + `U+0307`，故 `"İ"/"i"/"I"` **不**合并；
    德语 `ß` 不展开为 `ss`，`"Straße"/"strasse"` **不**合并（保守方向）。
- **改动点**：`crates/domain/src/campaign_runtime.rs`（新增 `normalize_instance_identity` + 去重键改用它，含名称 trim）；
  `crates/app-agent/src/runtime.rs`（`instance_name_matches` 委托共享函数、`find_instance_normalized` 的 id 归一分支、注释按事实改写）；
  `crates/app-agent/src/tools.rs`（4 处 id/name 比较改用共享归一：`:395-401`、`:449-462`、`:1004-1006`、`:1040-1046`）。
- **跨 crate 共享语料测试（失败可控）**：`runtime::tests::test_normalization_shared_corpus_agrees_across_crates`
  （12 组语料：ASCII 大小写、前导/尾随空白、制表/换行、`Ähre/ähre`、`Élodie/élodie`、`İrem/irem` 双向、`Straße/strasse`、
  不同人、形近不同字符；每组同时断言 domain 去重结论、app-agent 匹配结论、以及**两者相等**）+
  `test_normalization_same_name_multi_instance_agrees`（同名多实例：不新建、解析返回第一个）。
  **失败可控证据（实测）**：
  - 只把 app-agent 侧换回旧的 ASCII 语义 → 测试在 `("Ähre","ähre")` 失败（`left: false, right: true`）；
  - 只把 domain 侧换回旧的"不 trim"语义 → 测试在 `(" Alice ","Alice")` 失败（`domain 去重结论与期望不符 … temps=1`）。

### 13.10 `08-docs-sync-fixes.md:123` 虚假归属 → 已更正（只改那一句）

- **原文（虚假归属）**：称 W-31"域5 记录称其已在前端侧修复、文档侧待 task-16"。
- **反证**：撰写当时 `fixes/05-frontend-fixes.md` 全文对 `W-31`/`validGenerationModes` **零命中**
  （R2 已记为"被源码与域5 记录双重反证"，`round2/R2-pipeline-recheck.md:111`）。
- **本轮更正（只改该句 + 标注更正来源）**：写明这是虚假归属；现状 = 域5 在 R2 收尾时已修前端部分
  （`frontend/src/stores/writing.js:22` 从 `utils/generationModes.js` 目录派生 `validGenerationModes`、`:68` 拒绝未知档位；
  `fixes/05-frontend-fixes.md:475-485`），残留死分支 `adapter/useWritingScreenAdapter.js:89` 与 W-18 归 **task-32/task-33**，
  文档侧 `AGENT_INTERFACES.md:12` 归文档任务。
- **额外澄清**：`frontend/src/stores/writing.js:18-20` 仍出现 `big_scene` 字样，但那是说明"该值不得成为合法档位"的注释，
  不能据此判缺陷仍在（R2 当时的 grep 命中即此处）。

### 13.11 本轮门禁（实际命令 + 真实通过数）

| # | 命令 | 退出码 | 结果 |
| --- | --- | --- | --- |
| 1 | `cargo test -p storyforge-app-pipeline` | 0 | 140 passed / 0 failed（W-24 删 2 条死测试、R5-01 新增 3 条、N-R2-14 新增 1 条）；R9 再 +1（W-32 golden）→ **141** |
| 2 | `cargo test -p storyforge-app-agent` | 0 | 132 passed / 0 failed（新增 2 条跨 crate 归一语料测试） |
| 3 | `cargo test -p storyforge-domain` | 0 | 388 passed / 0 failed |
| 4 | `cargo test -p storyforge-app-memory` | 0 | 12 passed / 0 failed |
| 5 | `cargo test -p storyforge --lib runtime_support::tests::` | 0 | 9 passed / 0 failed（含 2 条新增 R11 测试） |
| 6 | `cargo test -p storyforge --lib` | 0 | 477 passed / 0 failed / 3 ignored（tauri-app 全库，本轮收口后复跑；task-32 收尾时同命令 480/0/3，+3 来自并发写者新增测试） |
| 7 | `cargo clippy -p storyforge-app-pipeline -p storyforge-app-agent -p storyforge-domain -p storyforge-app-memory --all-targets -- -D warnings` | 0 | `Finished`，0 warning |
| 8 | `cargo clippy -p storyforge --lib -- -D warnings` | 0 | `Finished`，0 warning |

> 本域未运行 `cargo test --workspace`（Lead 收口项）；前端未跑（无 npm 权限，且本轮未改前端）。
> 本轮改动文件：`crates/app-pipeline/src/{sequential_crew.rs,quality_gate.rs}`、
> `crates/domain/src/campaign_runtime.rs`、`crates/app-agent/src/{runtime.rs,tools.rs}`、
> `crates/app-memory/src/recall.rs`（仅注释）、`crates/tauri-app/src/runtime_support.rs`（唯一授权跨域例外）、
> `docs/review-2026-09-13/fixes/{03-pipeline-fixes.md,08-docs-sync-fixes.md}`、`docs/review-2026-09-13/round2/R11-pipeline-regression-closure.md`。

---

## 14 R6 无记录项收口（task-32 / R9）

> 背景：`round2/R6-fix-completeness-audit.md` 列出 4 条"无人处置"项，其中 **W-18**（本域）与 **W-32**（本域）归本任务。

### 14.1 W-18（P2）→ 判定非问题（保留后端能力，不删）

| 项 | 内容 |
| --- | --- |
| 原状 | 本记录"移交域5+域2"，接收方零动作；R2 N-R2-01 判"生产不可达"成立 |
| 本轮复核结论 | **自动路由是有书面契约的 API 级能力**，不是死代码：`docs/workstreams/WRITING-PIPELINE-V2-IMPLEMENTATION-2026-07-27.md:9`「显式选择优先」+ `:66`「**未传模式的 API 调用才进入上述自动路由**……调用方确认后须显式以 `generation_mode=sequential_crew` 重试」 |
| 具体调用点 | `crates/tauri-app/src/commands/writing.rs:877`（`generation_mode: Option<…>`）→ `:927-930`（`route_generation_mode` + `enforce_generation_cost_confirmation`）；前端 wrapper `frontend/src/tauri-api.js:394-406`（`generationMode \|\| null`，具备传 null 能力） |
| 具体测试名 | `lib_tests_writing::start_writing_command_prompt_hook_messages_reach_mock_llm`（`generation_mode=None` 实跑该路径）、`automatic_route_promotes_large_roster_to_sequential_crew`、`automatic_route_detects_explicit_two_actor_interaction`、`automatic_expensive_route_requires_an_explicit_resubmission`、`cheap_or_explicit_route_needs_no_extra_confirmation`；域层 11 条（`crates/domain/src/generation.rs:120-236`） |
| 为何不删 | 选项 (b) 的前提"全仓无调用方（含测试）"不成立：两条测试直接调用；且删掉会让"省略档位"成为未定义行为（破坏跨进程 API 契约） |
| 未兑现的部分（诚实声明） | 真实 UI 恒传显式档位（`useWriting.js:141` + `stores/writing.js:58-63`，computed 永不为 null ⇒ 至少 `'continuation'`），故用户**永远不会**看到"自动升档 + 成本确认"。这是**设计取舍**（显式选择优先 + ComposerBar 已展示预计调用量），不是接线遗漏 |
| 若要落地到 UI | 需前端功能开发：① 区分"未显式选档"与"选了 continuation"（当前同一 `'continuation'` 兜底无法区分）；② 未选档时传 `null`；③ 为该 `Err` 做确认对话框（建议档位 + 预计调用量）并带 `generation_mode=<建议>` 重试。**本任务不改前端** → 建议 Lead 立项或明确"自动路由仅服务 API 调用方" |
| 残留（前端，非本域） | `frontend/src/adapter/useWritingScreenAdapter.js:89`（`generationMode === 'big_scene'` 死分支）、`frontend/src/utils/rerollPolicy.js:8` |

### 14.2 W-32（P3）→ 已补等价性/golden 测试（去重本身早已落地）

- 现状：`crates/app-agent/src/runtime.rs:1005/1029` 是**唯一实现**；`crates/app-pipeline/src/lib.rs:4348-4356`
  是 `#[inline]` 委托薄壳（路径 C 调用点 `lib.rs:2019/2023`）⇒ 已不存在"两份实现"可直接对比。
- 但 R2 §6 观察 7 指出的真实缺口是**没有测试防漂移**，本轮补：
  `crates/app-pipeline/src/lib.rs` tests 模块 `tests::path_c_context_formatters_delegate_byte_for_byte_and_match_golden`：
  ① 委托等价（薄壳 ↔ app-agent 唯一实现逐字节）；② **golden 字节锁**（分区标题、空行、`keys.join(", ")` 分隔符
  —— 直接影响 system/tail 分段与 LLM 缓存键）；③ 空包边界（全空 → 两边空串，`task` 不进分区）。
- **失败可控实测**：把 `app-agent/src/runtime.rs` 的 `"## 你的角色设定"` 临时改成 `"## 角色设定"` →
  测试红并打印 left/right（`stable 分段文本已变更（会影响 system 段缓存键）`）；改回后 1 passed / 0 failed。
- 门禁：`cargo test -p storyforge-app-pipeline` → **141 passed / 0 failed**（R11 后 140 → +1）。
