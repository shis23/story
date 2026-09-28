# 域 3 全量审查：写作流水线（产品核心）

- **审查对象**：`crates/app-agent/**`、`crates/app-pipeline/**`、`crates/app-conversation/**`、`crates/app-memory/**`
- **基线**：git HEAD `ab894c6`（工作树干净，2026-09-13）
- **方式**：只读审查。作者亲自打开全部核心文件（`app-pipeline/src/lib.rs` 8919 行、`app-agent/src/runtime.rs` 2829 行、`tools.rs` 1939 行、`postprocess.rs` 980 行、`sequential_crew.rs` 661 行、`quality_gate.rs` 840 行、`app-conversation/src/lib.rs` 1631 行、`app-memory/src/archiver.rs` 447 行等）；外围文件（`character_extractor.rs`、`chronicle_compressor.rs`、`llm_parse.rs`、`tool_center.rs`、`turn_dossier.rs`、`draft_revision.rs`、`pipeline_postprocess.rs`、`prompts/*`）由 4 个并行子审查覆盖后，**所有 P1/P2 结论均回到源码逐行复核**。未运行 cargo/npm/构建/测试。未修改除本报告外的任何文件。
- 每条发现的「证据」均为实读代码摘录；跨 crate 的调用方（`crates/tauri-app/**`、`crates/domain/**`、`frontend/**`）仅作为**可达性证据**引用，不构成本域新增缺陷的归属。

## 1. 范围与覆盖率

| 范围 | 文件数 / 行数 | 覆盖方式 | 覆盖率 |
| --- | --- | --- | --- |
| `crates/app-pipeline/src/lib.rs` | 8919 | 逐段通读：模式分派、continuation/duet/big_scene、regenerate 三路径、director/subagent/editor stage、postprocess 接线、prompt 组装、配置构造、Plan 解析 | 高（除纯测试段逐条抽读） |
| `crates/app-pipeline/src/{quality_gate,sequential_crew,turn_dossier,draft_revision}.rs` | 840+661+480+87 | 子审查 + 作者复核关键分支 | 高 |
| `crates/app-agent/src/runtime.rs` | 2829 | 工具循环核心、spawn_subagents、campaign system/volatile 组装、hint 注入 + 作者复核隔离路径 | 高 |
| `crates/app-agent/src/{tools,postprocess,pipeline_postprocess,summarizer,llm_parse,character_extractor,chronicle_compressor,tool_center}.rs` | 1939+980+403+120+265+427+479+331 | 子审查 + 作者复核 P1/P2 全部证据行 | 高 |
| `crates/app-agent/src/prompts/**` | 520+236+120+84+26+40 | 子审查 | 中高 |
| `crates/app-conversation/src/lib.rs` | 1631 | 子审查 + 作者复核 validate/replace/epoch/committed 保护 | 高 |
| `crates/app-memory/src/**` | 447+403+18 | 子审查 + 作者复核水位推进链路（含调用方） | 中高 |

**未覆盖**：`crates/app-pipeline/src/lib.rs` 的 5.5k~8.9k 行测试代码仅抽读；`harness-real-llm` 的真实 LLM 行为未验证；前端仅作为可达性证据阅读，未做前端审查。

## 2. 结论摘要

- **P0：0 条。P1：6 条（W-01 ~ W-06）。P2：21 条。P3：8 条。**
- 三句判断：
  1. **主线骨架是真的**：Director→Subagent×N→Editor、四种生成模式、QualityGate + 有界 1× Editor auto-fix、三件套后处理、Summarizer、provenance、`AgentProfileConfig` 的 `model_override`/`max_tool_rounds`/`tool_whitelist`/`enable_*` 均在真实代码路径上被消费（非文档虚标），CLAUDE.md 的绝大多数「Current Code Facts」与代码一致。
  2. **最危险的不是"没实现"，而是"识别身份的归一不一致 + LLM 输出畸形时的静默降级"**：`with_temporaries_for`（trim+大小写去重）与 `find_instance_by_id_or_name`（精确、大小写敏感、不 trim）语义不一致，使**大小写/空白变体**的子 Agent 失去 instance 绑定并退化为扁平角色查询（W-01）；Plan 缺 `character_id` 时被静默改写成 `"unknown"` 并在 Accept 落库成角色（W-02）；后处理/归档在畸形输出或部分失败时静默丢数据（W-05/W-06）。
  3. **产品模式的 reroll 契约有真实缺口**：非 Campaign 会话首写走显式 `continuation`，但整卷 reroll 传 `generation_mode=null`，被管道当作旧 `big_scene` 全流程重写且无任何模式校验（W-03）；质量门禁把「好的，我」「让我来」这类常见对白判为 Error，触发多余的 Editor auto-fix 并默认拦截 Accept（W-04）。

## 3. 发现清单（按严重度降序）

### W-01 [P1] 角色名归一不一致：大小写/空白变体让子 Agent 失去 instance 绑定，信息隔离降级

- **类别**：B 逻辑正确性 / D 信息安全隔离 / 身份解析
- **位置**：`crates/app-agent/src/runtime.rs:656-679`；`crates/app-pipeline/src/lib.rs:2554-2565`；`crates/domain/src/campaign_runtime.rs:50-57,103-120`；`crates/app-agent/src/tools.rs:936-987`
- **证据（三处语义不一致）**：

```rust
// domain/src/campaign_runtime.rs:50-57 —— 名称精确、大小写敏感、不 trim
pub fn find_instance_by_id_or_name(&self, value: &str) -> Option<&CharacterInstance> {
    if let Some(inst) = self.instances.iter().find(|i| i.id.as_str() == value) {
        return Some(inst);
    }
    // Name match (fallback)
    self.instances.iter().find(|i| i.name == value)
}
```

```rust
// domain/src/campaign_runtime.rs:111-120 —— 却按 trim + 小写去重，命中即跳过创建临时实例
for (cid, persona, behavior) in character_specs {
    let cid = cid.trim();
    if cid.is_empty() { continue; }
    if !seen.insert(cid.to_lowercase()) { continue; }   // " Seraphina " / "SERAPHINA" 都命中已存在的 "Seraphina"
```

```rust
// app-agent/src/runtime.rs:656-658 —— 用原始 character_id 查，且未 trim
let matched_instance = campaign_runtime
    .as_ref()
    .and_then(|cr| cr.find_instance_by_id_or_name(&task.character_id));
```

- **影响**：Director 输出 `"seraphina"` / `" Seraphina "` 这类变体时：①`with_temporaries_for` 因小写去重**不**创建临时实例；②`find_instance_by_id_or_name` 因大小写/空白**匹配失败** → `matched_instance = None`；③走 `else` 分支：system prompt 用 `context_package.character_brief` 替代 resolved persona/behavior，**不注入该实例的 knowledge/variables**；④`instance_id_for_ctx = None`（runtime.rs:678），子 Agent 的 `get_character` 落进扁平分支 `ctx.characters`（tools.rs:983-987），可读到**全部导入角色卡**的 description/personality —— 隔离目标失效且角色状态被割裂。
- **建议**：把归一收口成一个函数（`trim` + `eq_ignore_ascii_case`、ID 同样处理），`find_instance_by_id_or_name` 与 `with_temporaries_for` 共用；匹配失败时不要静默退化，发事件/错误提示（可复用 `fallback_reason`）。
- **置信度**：高（机制已核实）；**触发频率中**（取决于 LLM 是否归一大小写，属需要真实探测的项）。

### W-02 [P1] Plan 缺 `character_id` 被静默改写成 `"unknown"`，并在 Accept 时作为临时角色落库

- **类别**：B 逻辑正确性 / 数据污染
- **位置**：`crates/app-pipeline/src/lib.rs:3921-3925`；`crates/app-agent/src/runtime.rs`（`with_temporaries_for` 调用点 `app-pipeline/src/lib.rs:2564-2577`）；`crates/domain/src/campaign_runtime.rs:111-129`；`crates/tauri-app/src/turn_lifecycle.rs:104-108`
- **证据**：

```rust
// app-pipeline/src/lib.rs:3921-3925
let character_id = t
    .get("character_id")
    .and_then(|v| v.as_str())
    .unwrap_or("unknown")     // 畸形 task 不报错，静默变成角色 "unknown"
    .to_string();
```

```rust
// domain/src/campaign_runtime.rs:118-129 —— 只要非空就建临时实例
if !seen.insert(cid.to_lowercase()) { continue; }
let temp = CharacterInstance::temporary_with_overrides(
    self.campaign.id.clone(), cid, persona.clone(), behavior.clone());
```

```rust
// tauri-app/src/turn_lifecycle.rs:104-108 —— accept 时前置 UpsertInstance，临时角色落 Campaign
if !instance_prelude.iter().any(|m| matches!(m, Mutation::UpsertInstance(inst) if inst.id == temp.id)) {
    instance_prelude.push(Mutation::UpsertInstance(Box::new(temp.clone())));
```

- **影响**：LLM 少写一个字段（回归测试外很常见）会：以 `"unknown"` 为名创建一个临时 `CharacterInstance`、给它跑一个子 Agent（多一次 LLM 调用）、把它挂到 `pending_temporary_instances`，Accept 时经 `UpsertInstance` **写入 Campaign**（后续 `list_characters`、变量、知识归属都能看到这个幽灵角色）。`parse_plan_json` 对 `subagent_tasks` 既不去重也不与 roster 交叉校验（同 `character_id` 两次会出现两个子 Agent，`rerun_subagents` 只命中第一个）。
- **建议**：`character_id` 缺失/空白 → 该 task 记 warning 并丢弃（或整体 `PlanParse` 失败重试一次）；对重复 `character_id` 去重；`unknown` 之类的保留字禁止落库。
- **置信度**：高（机制已核实；`unwrap_or("unknown")` 与 accept 落库两侧都读过）。

### W-03 [P1] 非 Campaign 会话整卷 reroll 丢失 generation_mode，续写稿被 `big_scene` 全流程静默重写

- **类别**：A 目标完成度 / B 逻辑正确性 / reroll 契约
- **位置**：`crates/app-pipeline/src/lib.rs:1672-1684,1690-1715,1812-1876`；调用方 `crates/tauri-app/src/commands/writing.rs:89-96`；前端 `frontend/src/composables/useWriting.js:139-141`、`frontend/src/composables/useMessageVariants.js:167`、`frontend/src/stores/writing.js:57-62`
- **证据**：

```rust
// app-pipeline/src/lib.rs:1672-1684 —— 只校验「局部重跑 + 非 BigScene 来源」
let requests_legacy_partial = !req.targets.is_empty()
    && matches!(req.generation_mode, None | Some(GenerationMode::BigScene));
...
if let Some(generation_mode) = req
    .generation_mode
    .filter(|mode| *mode != GenerationMode::BigScene)
{
    if generation_mode == GenerationMode::SequentialCrew && !req.targets.is_empty() { ... }
    // 否则整卷重写：start_writing_with_mode_at(..., generation_mode, ...)
```

```rust
// app-pipeline/src/lib.rs:808 / 1841 / 1859 —— 无 mode（None）时落到旧 BigScene
pub async fn start_writing(...) {
    self.start_writing_with_mode(intent, ctx, GenerationMode::BigScene, event_tx, cancel)
...
GenerationMode::BigScene,        // 路径 A：整卷重跑（1808-1876 段）
```

```javascript
// useMessageVariants.js:167 —— 非 campaign 一律 null；useWriting.js:141 首写却传显式 mode
generationMode: campaignStore.activeCampaign ? writingStore.generationMode : null,
// writing.js:60 —— 无 campaign 时 generationMode 计算值= 'continuation'，即首写是显式 continuation
if (!campaignId) return 'continuation'
```

- **影响**：非 Campaign 会话（legacy 卡模式，CLAUDE.md 明令"不得破坏"）首轮由 `continuation` 单笔者写成；用户点「整体重 roll / 重 roll 导演」时 `targets=[]`、`generation_mode=null` → 一路走到 `start_writing_with_mode_at(..., GenerationMode::BigScene, ...)`：**Director + N 子 Agent + Editor** 全流程重写，provenance 的 `generation_mode` 变成 `BigScene`，费用从「1 次正文」跳到「4+N 次编排」，成文风格/角色分配随之改变，且没有任何告警。
- **建议**：`RegenerateRequest.generation_mode=None` 时，从旧 provenance 继承模式（`provenance_old.generation_mode`），继承不到再退回 BigScene 并显式告知；前端非 campaign 也应传当前 `generationMode`（与首写一致）。
- **置信度**：高（两侧代码 + 前端传参均已核实）；产品意图（是否刻意保留 legacy 重 roll=BigScene）需 Lead 确认。

### W-04 [P1] 质量门禁子串匹配 + Error：常见对白触发 1× Editor auto-fix 并默认拦截 Accept

- **类别**：E 逻辑正确性 / 误报
- **位置**：`crates/app-pipeline/src/quality_gate.rs:91-119`；消费点 `crates/tauri-app/src/production_postprocess.rs:168-174,216-236`；拦截点 `crates/domain/src/turn.rs:510-518`、`crates/tauri-app/src/turn_lifecycle.rs:421`、`crates/tauri-app/src/turn_lifecycle.rs:228`
- **证据**：

```rust
// quality_gate.rs:91-119
const PATTERNS: &[&str] = &["作为AI", "作为 AI", "作为人工智能", "我来写", "以下是为您创作",
    "以下是故事", "让我来", "现在开始创作", "好的，我", "没问题，我", "根据你的要求",
    "按照你的要求", "我将为你", "我来为你"];
for pat in PATTERNS {
    if text.contains(pat) {
        return Some(QualityWarning { ... severity: QualitySeverity::Error });
```

```rust
// production_postprocess.rs:172-174 / 222 —— 任何 warning 就进 autofix；autofix 后仍有 Error 才回滚
fn autofix_result_can_replace_original(report: &QualityReport) -> bool { !report.has_errors() }
fn quality_report_needs_editor_autofix(report: &QualityReport) -> bool { !report.passed() }
```

```rust
// domain/src/turn.rs:515-518
/// Accept 策略：Error 默认拦截；`force=true` 时允许强制接受（→ Degraded）。
pub fn blocks_accept(&self, force: bool) -> bool { self.has_errors() && !force }
```

- **影响**：`让我来`（角色台词"让我来"）、`好的，我`（"好的，我这就去"）是中文小说高频句式，一旦出现即判 **Error** → ①白跑一次整篇 Editor auto-fix（`revise_draft`，一次完整 LLM 调用）；②默认 `block accept`（用户必须 force → Turn 落 `Degraded`）。同时该检查对"元描述"的语义判断完全依赖子串。
- **建议**：改为行首/段落级锚定 + 必须伴随创作元语言（如同时出现"用户/创作/故事"），或降级为 Warning；`但我来`/`让我来` 移除或加边界断言。
- **置信度**：高（机制与拦截链路均已核实）；误报频率取决于题材（判断项）。

### W-05 [P1] 后处理单条畸形 → 三件套全丢 + 多打一次 LLM（`PostProcessDto` 无逐条容错）

- **类别**：B 逻辑正确性 / D 静默失败
- **位置**：`crates/app-agent/src/postprocess.rs:189-205,347-398,85-92`；`crates/app-agent/src/pipeline_postprocess.rs:188-194`
- **证据**：

```rust
// postprocess.rs:189-205 —— character_id / knowledge_text 必填且无 serde default
struct KnowledgeUpdateDto {
    character_id: String,
    knowledge_text: String,
    #[serde(default = "default_source")] source: String,
    ...
```

```rust
// postprocess.rs:347-372 —— 5 层兜底全部反序列化同一个 PostProcessDto
for tc in &resp.tool_calls {
    if tc.function.name == "emit_postprocess"
        && let Ok(args) = serde_json::from_str::<serde_json::Value>(&tc.function.arguments)
        && let Ok(dto) = serde_json::from_value::<PostProcessDto>(args) { return dto_to_result(dto); }
```

```rust
// postprocess.rs:85-92 —— 解析失败（含"合法空结果"）触发第二次完整 LLM 调用
let result = if result.parse_succeeded && !result.is_empty() { result }
else { run_direct_json_fallback(...).await...unwrap_or(result) };
```

```rust
// pipeline_postprocess.rs:188-194 —— 最终 parse_succeeded=false → None → 事件按"失败"处理
Ok(r) if !r.parse_succeeded => { warn!(...); None }
```

- **影响**：LLM 只要在 `knowledge_updates` 的**任一条**里漏 `knowledge_text`/`character_id`，**整包**反序列化失败 → 同一响应里合法的 `variable_updates`/`task_updates` 一并丢弃，再补一次完整 LLM 调用（无工具直出 JSON），仍失败则本轮知识/变量/任务全部丢失，只留 `post_process=None`（best-effort，不阻断成文，用户不可见）。
- **建议**：把三个数组先反序列化为 `Vec<serde_json::Value>`，逐条解析、逐条 warn 丢弃；`parse_succeeded` 与 `is_empty()` 解耦（空结果是成功）；把"降级/丢弃条数"透出到 outcome 或事件。
- **置信度**：高（已核实：DTO 字段定义 + 三条消费路径）。

### W-06 [P1] 归档批次失败/嵌入失败仍推进水位 → 远记忆段永久空洞且永不重试

- **类别**：B 逻辑正确性 / 数据丢失（app-memory）
- **位置**：`crates/app-memory/src/archiver.rs:151-185,187-226`；调用方 `crates/tauri-app/src/commands/conversations.rs:145-163`
- **证据**：

```rust
// archiver.rs:179-185 —— 批次失败只 warn，成功批次照常返回
let results: Vec<Result<ArchivedSummary, MemoryError>> = stream.collect().await;
for result in results {
    match result {
        Ok(summary) => summaries.push(summary),
        Err(e) => warn!(target: "app-memory", "归档批次失败: {e}"),
    }
}
```

```rust
// archiver.rs:209-220 —— upsert 失败也只 warn，但 vector 照样置 Some、日志称"已入库"
if let Err(e) = self.vector_store.upsert(VectorRecord { ... }) {
    warn!(target: "app-memory", "总结 {} 入向量库失败: {e}", summary.id);
}
summary.vector = Some(vector);
info!(target: "app-memory", "总结 {} 已嵌入并入库", summary.id);
```

```rust
// tauri-app/src/commands/conversations.rs:154-159 —— 水位 = 成功批次的 max(end_idx)+1
let advanced = summaries.iter().map(|s| s.source_range.1.saturating_add(1)).max().unwrap_or(0);
```

- **影响**：批次按升序切分、`buffer_unordered` 乱序返回；若第 0 批 LLM 失败而第 1 批成功，水位直接越过失败批 —— 这些消息既不在 `pending`（水位之后不再归档）也不在向量池，**永久丢失且无重试**。嵌入失败同理：内容没有任何其他持久化（调用方只推进水位，不落盘 summary）。归档是 best-effort 后台任务，用户侧无任何信号。
- **建议**：`archive_prefix` 返回"连续成功前缀长度"，调用方只推进到缺口前；upsert 成功后才置 `vector=Some`；失败批次记入 job 状态/事件。
- **置信度**：高（两侧均已核实）。

### W-07 [P2] 顺序剧组把"用户取消"报成 `SubagentFailed`，且 `scene_close` 复用 `Cancelled`

- **类别**：B 逻辑正确性 / 错误语义
- **位置**：`crates/app-pipeline/src/sequential_crew.rs:229-232,239-243,320-329`
- **证据**：

```rust
// 226-232
let mut scene_closed = false;
for (index, original_task) in tasks.into_iter().enumerate() {
    if *cancel.borrow() || scene_closed {
        results.push(Err(AgentError::Cancelled));   // 剧本正常收尾 = Cancelled
        continue;
    }
// 239-243
    for attempt in 1..=MAX_ATTEMPTS_PER_ACTOR {
        if *cancel.borrow() { last_error = "流水线已取消".into(); break; }
// 320-329
    None => results.push(Err(AgentError::SubagentFailed(format!(
        "顺序剧组演员 {actor_id} 在 {MAX_ATTEMPTS_PER_ACTOR} 次尝试后失败: {last_error}")))),
```

- **影响**：取消发生在重试期间时该演员被标为 `SubagentFailed`（错误文案："N 次尝试后失败: 流水线已取消"），若它是最后一位则整批结果中不含 `Cancelled`；`scene_close`（叙事收尾，正常语义）反而发 `SubagentCancelled` 事件。调用方（`lib.rs:1317-1339` duet、`2342-2367` suffix）只按"是否全失败"判断，事件/日志会误导用户。（后续 Editor stage 仍会因 cancel 中止，故不是流程失控。）
- **建议**：`break` 后按 `cancel.borrow()` 返回 `Cancelled`；`scene_close` 用独立原因（如 `SceneClosed`）表达。
- **置信度**：高（已核实）。

### W-08 [P2] 同名 instance 静默绑定第一个，歧义无检测

- **类别**：B 逻辑正确性 / 身份解析
- **位置**：`crates/domain/src/campaign_runtime.rs:50-57`；`crates/app-agent/src/tools.rs:385-407`；`crates/app-agent/src/runtime.rs:656-658`
- **证据**：

```rust
// domain/src/campaign_runtime.rs:55-56
// Name match (fallback)
self.instances.iter().find(|i| i.name == value)     // 同名多实例 → 静默取第一个
```

```rust
// tools.rs:385-407（Director get_character）—— 同样静默取第一个
if let Some(runtime) = &ctx.campaign_runtime
    && let Some(inst) = runtime.find_instance_by_id_or_name(name) { ... }
```

- **影响**：同名（如两个「码头工人」）时 Director 用名字发出的 `SubagentTask`/`get_character` 会绑定到**第一个**实例：错的人格/知识/变量；后续知识写回（Tauri 侧有 `name_collisions` 收紧）仍可能落错对象。测试 `tools.rs:1331-1357` 名为 `..._duplicate_name_candidates`，实际查询的是**不存在**的名字（"工人甲"），**歧义分支零覆盖**。
- **建议**：同名且未给 ID 时返回可恢复错误（列出候选 instance_id）；补歧义用例。
- **置信度**：高（代码事实）。

### W-09 [P2] 合法空结果触发第二次完整 LLM 调用（成本翻倍）

- **类别**：B 逻辑正确性 / 成本
- **位置**：`crates/app-agent/src/postprocess.rs:85-92,103-112`
- **证据**：

```rust
let result = if result.parse_succeeded && !result.is_empty() { result } else {
    run_direct_json_fallback(runtime, &config, &fallback_user_msg, fallback_cancel).await...
};
// run_direct_json_fallback 开头：
warn!(target: "postprocess", "emit_postprocess/JSON parse missed or returned empty; retrying postprocess once without tools");
```

- **影响**：`is_empty()`（`knowledge/variable/task` 三项皆空）是"LLM 正常返回、本轮无更新"的常见情形，代码仍走 fallback：**多一次完整 LLM 调用**，日志还把正常情况写成 "parse missed"。测试 `postprocess.rs:807-838` 把这个行为固化成了预期。
- **建议**：只用 `parse_succeeded` 判是否需要 fallback；空结果直接返回 `Some(empty)`。
- **置信度**：高。

### W-10 [P2] `broadcast` 不归一大小写/空白 → 广播知识静默丢弃

- **类别**：B 逻辑正确性 / 契约脆弱
- **位置**：`crates/app-agent/src/postprocess.rs:290-294`；下游 `crates/tauri-app/src/production_postprocess.rs:1204-1218`
- **证据**：

```rust
// postprocess.rs:290-294
broadcast: k.broadcast.and_then(|s| match s.as_str() {
    "all" => Some(BroadcastTarget::All),
    "" => None,
    group => Some(BroadcastTarget::Group(group.to_string())),   // "All" / " all" 都落到这里
}),
```

```rust
// production_postprocess.rs:1210-1217 —— Group 走精确组名匹配，无匹配则 targets 为空
Some(BroadcastTarget::Group(group)) => instances.iter()
    .filter(|instance| instance_matches_group(runtime, instance, group))
    .cloned().collect(),
...
for target in targets { ... }   // 空 targets = 静默丢弃，无 warn
```

- **影响**：LLM 输出 `"All"`/`" all"`/`"全体"` 时，广播知识变成"组广播"，无任何 instance 命中 → 该条知识**静默丢弃**（与 W-05 的静默风格一致）。
- **建议**：`s.trim().to_ascii_lowercase()` 后匹配 `all|全体|所有人`；Group 无命中时 warn。
- **置信度**：高（机制已核实）。

### W-11 [P2] 后处理把「名字」直接塞进 `Id` 字段，且来源缺失时传播门禁 fail-open

- **类别**：B 逻辑正确性 / D 隔离门禁
- **位置**：`crates/app-agent/src/postprocess.rs:284-288,303-313,322`；下游 `crates/tauri-app/src/production_postprocess.rs:1149-1157,1218`
- **证据**：

```rust
// postprocess.rs:284-288 —— 名字不解析，直接 Id::from_str
.map(|k| CharacterKnowledgeUpdate {
    character_id: Id::from_str(&k.character_id),
    ...
    source_character_id: k.source_character_id.map(|s| Id::from_str(&s)),
```

```rust
// production_postprocess.rs:1144-1157 —— 来源解析不出 → 直接不拦截（fail-open）
let propagating = update.broadcast.is_some() || matches!(update.source, KnowledgeSource::ToldByOther);
if !propagating { return false; }
let Some(source_raw) = update.source_character_id.as_ref() else { return false; };
let Some(source) = resolve(source_raw) else { return false; };
```

- **影响**：app-agent 层不做名字→instance 归一（域文档在 `domain/agent.rs:605-607`、`character_knowledge.rs:194` 声明这两个字段是实例 ID），归一完全依赖 Tauri 侧 `resolve`：解析失败即静默丢弃（`production_postprocess.rs:1218` 的 `resolve(...)` 返回空 targets）。更值得复核的是 `source_character_id` 缺失/不可解析时 `source_propagation_blocks` 返回 false（**不拦截**），此时带 `broadcast` 的知识会照常外播，源条目的 `private/sealed` 策略失效。
- **建议**：app-agent 输出层做名字→ID 归一（或在 DTO 里改名为 `character_name` 并显式标注契约）；来源不可解析时 fail-closed（跳过该条广播）并 warn。
- **置信度**：机制 高（两侧已核实）；实际泄漏面取决于 `resolve` 与同名收紧逻辑，**跨域需 Lead 在域 1/2 复核**。

### W-12 [P2] `present_chars` 门禁只存在于提示词，app-agent 侧无校验；MVU JS 产出 `instance_id=None` 直通

- **类别**：B 逻辑正确性 / 门禁分层
- **位置**：`crates/app-agent/src/prompts/postprocess.rs:244-251`；`crates/app-pipeline/src/lib.rs:1529-1541`
- **证据**：

```rust
// app-pipeline/src/lib.rs:1529-1541 —— MVU JS fallback 直接追加 variable_updates（无 instance 作用域）
if let Some(updates) = exec_result.variable_updates { ... }
outcome.post_process.variable_updates.push(VariableUpdate { instance_id: None, key, value })
```

- **影响**：`present_characters` 只以文本形式进提示词，app-agent 不做任何过滤（真正的门禁在 `persist_postprocess_outcome`/`build_knowledge_mutations`）。新增的 MVU JS 变量写回走同一 outcome 通道但**不带实例作用域**，最终写到 campaign 级变量，绕过了"实例变量"的作用域模型。任何新的 outcome 消费者都必须自己重实现门禁，属于易漏的契约设计。
- **建议**：在 `PostProcessOutcome` 上加"产出已按 present 过滤/带作用域"的显式契约字段，或把过滤下沉到 app-agent 的统一出口。
- **置信度**：中高（代码事实已核实）。

### W-13 [P2] 摘要/远记忆按整轮生成却注入**每个**子 Agent，隔离靠一句软提示

- **类别**：D 信息隔离
- **位置**：`crates/app-agent/src/runtime.rs:690-706`；`crates/app-pipeline/src/lib.rs:2589-2603,2001-2011`；`crates/app-agent/src/prompts/summarizer.rs:84-93`
- **证据**：

```rust
// runtime.rs:690-698 —— 同一份摘要块注入所有子 Agent
// ContextCompiler 最小版：子 Agent 也看到近期摘要（共享事实，不破信息隔离）
if let Some(block) = recent_summary_block.map(str::trim).filter(|s| !s.is_empty()) {
    volatile_text.push_str("\n\n");
    volatile_text.push_str(block);
    volatile_text.push_str("\n（以上为近期剧情摘要，仅供保持连续性；勿泄露你角色不该知道的信息。）");
}
```

- **影响**：摘要由**整轮成文**（含所有角色的内心/秘密）生成，没有任何按角色过滤；远记忆（`far_memory_hits`）同理是 campaign 级召回。每个子 Agent 都能读到"B 的内心秘密"，隔离只剩提示词软约束。这与 `docs/AGENT_INTERFACES.md` 的"按角色隔离表演"目标存在张力。
- **建议**：摘要块按 instance 过滤（或拆成"公开事件"与"角色私有"两段），至少对 private/sealed 知识做遮蔽；把注入口径写进 MEMORY-CONTEXT-COMPILER-SPEC。
- **置信度**：机制 高 / 泄露面 中（需产品确认摘要是否被视为公共事实）。

### W-14 [P2] 召回无角色维度；无 campaign 标签的旧归档被任意 Campaign 接受（app-memory）

- **类别**：D 信息隔离
- **位置**：`crates/app-memory/src/recall.rs:166-174,177-206`；`crates/app-memory/src/archiver.rs:191-208`
- **证据**：

```rust
// recall.rs:166-173
fn accepts_campaign(hit: &VectorRecord, campaign_id: Option<&str>) -> bool {
    let Some(cid) = campaign_id else { return true; };
    match hit.metadata.get("campaign_id").and_then(|v| v.as_str()) {
        Some(value) => value == cid,
        None => true,        // 无标签的旧归档对所有 campaign 开放
    }
}
```

- **影响**：归档写入时 `campaign_id` 为 None 就只写 `source`（archiver.rs:204-207），这类历史记录对任何 campaign 的检索都通过；`filter_archived_hits` 只按 kind/campaign/score/id 过滤，**没有任何 character/instance 过滤**，而召回结果会被注入子 Agent 提示（W-13）→ 跨故事/跨角色的私密记忆可被检索注入。
- **建议**：归档记录写入参与角色 id 集合、召回时按 `current_character_instance_id` 过滤；无标签旧归档在 campaign 过滤下 fail-closed。
- **置信度**：机制 高 / 实际泄露面 中。

### W-15 [P2] `archive_prefix` 的 `max_concurrency` 未钳制 0 → `buffer_unordered(0)` 永久挂起

- **类别**：B 边界 / app-memory
- **位置**：`crates/app-memory/src/archiver.rs:132-134,177`
- **证据**：

```rust
let batch_size = self.config.archive_batch_size.max(1);
let trigger_count = self.config.archive_trigger_count.max(1);
let total_to_archive = (batch_size * trigger_count).min(messages.len());
...
.buffer_unordered(self.config.max_concurrency);   // 0 时 futures 流永不 poll 内层
```

- **影响**：`max_concurrency=0` 时 `collect().await` 永不返回（挂起后台归档任务）。当前仓库调用方只用 `ArchiveConfig::default()`（`conversations.rs:56/249`），可达性属**疑似**，但这是"配置零值"类已知坑，与 `effective_max_concurrent_subagents()` 的钳制哲学（CLAUDE.md 明列）不一致。
- **建议**：`.max(1)`；顺带 `batch_size * trigger_count` 改 `saturating_mul`。
- **置信度**：代码事实 高 / 可达性 中。

### W-16 [P2] `validate_partial_roll` 不校验 target 集合本身（重复/未知/空）

- **类别**：B 契约 / E 测试缺口
- **位置**：`crates/app-conversation/src/lib.rs:803-817`；消费 `crates/app-pipeline/src/lib.rs:1788-1807,1947-2108`
- **证据**：

```rust
// app-conversation/src/lib.rs:804-817
let rerun_director = targets.iter().any(|t| matches!(t, PartialRollTarget::Director));
let keep_subagents = !targets.iter().any(|t| matches!(t, PartialRollTarget::Subagent(_)));
if rerun_director && keep_subagents && !provenance.subagent_results.is_empty() {
    return Err(ConversationError::PartialRollViolation(...));
}
Ok(())      // 重复 Subagent / 未知 Subagent / 空 targets 全部放行
```

- **影响**：`targets=[Subagent("A"),Subagent("A")]` → 校验通过 → 路径 C 顺序重跑 A **两次**（两次 LLM 付费 + 重复 `SubagentStarted/Done` 事件），随后 `HashMap` 只保留最后一版，第一次结果静默丢弃；未知 id 直到 `lib.rs:1949-1958` 才以另一种错误类型失败。测试 `app-conversation/src/lib.rs:1487-1535` 只覆盖 Editor/Director。
- **建议**：validate 内去重 + 与 `provenance.plan.subagent_tasks` 交叉校验 + 空集合语义显式文档化。
- **置信度**：高。

### W-17 [P2] `replace_active_variant` 无"最后一条"断言；非 Campaign 会话的已采纳历史无保护

- **类别**：B 数据完整性 / 调用契约
- **位置**：`crates/app-conversation/src/lib.rs:60-66,524-562`
- **证据**：

```rust
// 60-66
fn validate_committed_prefix(original: &Conversation, updated: &Conversation) -> Result<(), ConversationError> {
    if original.campaign_id.is_none() {
        return Ok(());        // legacy 会话：已采纳前缀保护直接短路
    }
```

```rust
// 530-545
pub fn replace_active_variant(&self, conv_id: &Id, node_id: &Id, content: String, provenance: Option<Provenance>) -> ... {
    self.with_conversation_mut(conv_id, |conv| {
        let node = conv.find_node_mut(node_id)...?;
        if let Some(old) = node.variants.get_mut(node.active_variant) { old.status = VariantStatus::Discarded; }
```

- **影响**：方法自身不校验"必须是最后一条 AI 节点"（该不变量只由 `app-pipeline/src/lib.rs:2881-2900` 的调用方通过 `is_last_assistant_node` 维持）。任何绕过 pipeline 的调用方可以对中间节点执行"降级 active + 改写正文"，直接改写后续所有上下文；legacy 会话因 64-66 短路连 committed 前缀保护都没有。当前实测调用方均在 pipeline 内（可控），故定 P2。
- **建议**：把 `is_last` 判定与 replace/add 合并进同一个 `with_conversation_mut`（顺便消除 `is_last_assistant_node` 与落地之间的 TOCTOU），并让 committed 保护不区分 campaign。
- **置信度**：高（代码）。

### W-18 [P2] big_scene 自动路由与成本确认在生产不可达（冗余计算）

- **类别**：C 冗余/死代码
- **位置**：`frontend/src/composables/useWriting.js:139-141`、`frontend/src/stores/writing.js:57-62`；`crates/tauri-app/src/commands/writing.rs:927-936,855-869`、`crates/domain/src/generation.rs:61-106`
- **证据**：

```javascript
// useWriting.js:141 —— 唯一真正的 start_writing 调用，mode 永远非空
const result = await startWritingApi(hookedIntent, charIdForWriting, ..., writingStore.generationMode)
// stores/writing.js:60 —— 没有 campaign 时也返回 'continuation'
if (!campaignId) return 'continuation'
```

```rust
// domain/src/generation.rs:62-67 —— explicit_mode 一旦存在，路由引擎全部短路
if let Some(mode) = signals.explicit_mode {
    return GenerationRouteDecision { mode, reason: GenerationRouteReason::ExplicitChoice, requires_cost_confirmation: false };
```

- **影响**：`generation_route_signals*`（`writing.rs:695-853`，约 160 行：提及角色识别、私密知识分歧、对立议程、成本确认文案）与 `route_generation_mode` 的 `ActorCount / LargeSceneIntent / MultipleImminentTasks / DuetSignals` 分支在前端路径上**永不生效**；`enforce_generation_cost_confirmation` 的 fail-closed 分支同样不可达（显式选择时 `requires_cost_confirmation=false`）。用户永远拿不到"建议升级到顺序剧组"的自动提示。
- **建议**：要么前端在"用户未手动改过档位"时传 `null` 让路由生效，要么明确降级为不调用的 legacy 能力并删测试/文案。
- **置信度**：高（前端唯一调用点已核实）。

### W-19 [P2] `regenerate` 单子 Agent 路径硬编码 rounds=10 且忽略 Subagent profile 覆盖

- **类别**：A 目标完成度 / C 冗余
- **位置**：`crates/app-pipeline/src/lib.rs:2027-2034`（`ToolRegistry::new()` 后直接构造 `AgentConfig`）
- **证据**：

```rust
let config = AgentConfig {
    role: AgentRole::Subagent(target_id.clone()),
    system_prompt,
    max_tool_rounds: 10,                              // 硬编码，未读 profile
    model: director_config.model.clone(),             // 用导演模型，未读 Subagent:*
    tools: vec![], terminal_tools: vec![],
};
```

- **影响**：与 CLAUDE.md 的「Subagent model/rounds are overridden per-profile」在**局部重跑路径**上不一致：用户在配置面板给 `Subagent:*` 设的 `model_override`/`max_tool_rounds` 在 big_scene 局部重 roll 时被忽略。`spawn_subagents`（runtime.rs:644-653）与顺序剧组路径消费正常，属"路径遗漏"。
- **建议**：复用 `run_subagent_stage` 或把 profile 读取抽成公共函数。
- **置信度**：高。

### W-20 [P2] auto-fix 结果无长度/非空守卫，被截断的短稿可替换整篇

- **类别**：B 边界
- **位置**：`crates/app-pipeline/src/draft_revision.rs:78-85`；`crates/tauri-app/src/production_postprocess.rs:172-174,250-279`
- **证据**：

```rust
// draft_revision.rs:78-85 —— 无长度/非空/差异校验
let text = apply_editor_output_regex(&response.content, &ctx.regex_scripts)?;
let mut provenance = request.provenance.cloned();
if let Some(p) = provenance.as_mut() { p.editor_reasoning = response.reasoning_content; ... }
Ok((text, provenance))
```

- **影响**：判定能否替换原文的判据是 `!report.has_errors()`，而 `TooShort`（<50 字）只是 **Warning**（quality_gate.rs:123-135）。Editor 返回一段被 regex 截断的短文本（但 ≥1 字且通过视图/元描述检查）时会**无声替换**整篇草稿（空内容会被 `MaxRoundsExceeded` 挡下，故风险集中在"过短/被截断"）。
- **建议**：`revise_draft` 加最小长度与"不得显著短于原文"守卫，失败回退原文。
- **置信度**：中高（机制已核实）。

### W-21 [P2] 质量门禁 `has_other` 优先于 `has_owner`：双角色同场易误判越权

- **类别**：B 逻辑正确性 / 误报
- **位置**：`crates/app-pipeline/src/quality_gate.rs:380-418`
- **证据**：

```rust
let has_owner = owner_labels.iter().any(|l| !l.is_empty() && window.contains(l));
let has_other = other_labels.iter().any(|l| !l.is_empty() && window.contains(l));
if has_other {
    return true;        // 若 A 合法回忆自己的秘密、而 B 的名字落在 ±48 字窗口内 → 直接判越权
}
if !has_owner { ... 2× 宽窗口复核 ... }
```

- **影响**：`other_labels` 含其他 private binding 的显示名，两个各有秘密的角色同场时，"A 回忆自己的秘密 + 邻句出现 B 的名字"会被判 Error → 触发 autofix + 拦截 Accept（同 W-04 链路）。既有测试 `quality_gate.rs:684-707` 的窗口内无他人标签，覆盖不到。
- **建议**：`has_other` 仅在 `!has_owner` 时判定（拥有者标签在场应优先），或对他角色只比对 instance_id 而非显示名。
- **置信度**：中高。

### W-22 [P2] `tool_center.rs` 为死代码且内置"幻影工具名"

- **类别**：C 冗余/死代码
- **位置**：`crates/app-agent/src/tool_center.rs:100-107`；导出 `crates/app-agent/src/lib.rs:9,37`
- **证据**：

```rust
// tool_center.rs:100-107
c.reg("subagent.get_character", "子 Agent 查自己 instance（信息隔离）", ToolScope::Roles(vec![AgentRole::Subagent("*".into())]));
...
"compose",       // 实际注册名：tools.rs:919 是 "get_character"；"compose" 全仓无注册点
```

- **影响**：`ToolCenter|role_matches|default_tool_names_for` 的全部引用只落在 `tool_center.rs` 自身与 `lib.rs:37` 的 re-export（tauri-app/app-pipeline/app-meta/harness 命中 0）。若将来有消费方按它生成白名单，未知名会被 warn+ignore（CLAUDE.md 语义）→ 工具静默失效。测试 `tool_center.rs:252-258` 还把错误名字固化成断言。
- **建议**：删除该模块与双导出，或改为从 `ToolRegistry` 反推元数据。
- **置信度**：高。

### W-23 [P2] 角色抽取的数组边界过宽；Chronicle 压缩失败静默降级为"未压缩正文"

- **类别**：B 逻辑正确性 / D 静默降级
- **位置**：`crates/app-agent/src/character_extractor.rs:224-249`；`crates/app-agent/src/chronicle_compressor.rs:133-141,273-291`
- **证据**：

```rust
// character_extractor.rs:226-245 —— 以第一个 '[' 为起点后一直扫描到文本结尾，不认数组的 ']'
let arr_start = content.find('[')?;
let after_bracket = &content[arr_start + 1..];
while search_from < after_bracket.len() { ... if let Ok(dto) = ... { defs.push(dto_to_definition(dto)); } ... }
```

```rust
// chronicle_compressor.rs:273-290 —— 解析失败 → 用原文拼接发布（headlines + s.content）
Err(e) => { warn!(... "LLM JSON 解析失败，降级确定性文案: {e}");
    return publish_with_deterministic_texts(...) }        // summary: bodies.join(" ")
```

- **影响**：①数组之后的任何 `{...}`（模型举例、尾注 JSON）都会被当成角色定义写进卡；②压缩失败时发布的 `summary` 是成员原始正文拼接（不受 80–200 字约束），调用方 `backend_workflows.rs:1927` 直接按成功发布，**用户侧无降级信号**；`extract_json_array`（133-141）用"第一个 `[` + 最后一个 `]`"切片，尾部方括号（如"注：以上 [1]"）会让解析失败并落进同一降级分支。
- **建议**：①以数组 `]` 为界、失败计数并上报；②给确定性降级文案设上限并把 degraded 透出到 outcome/job。
- **置信度**：高（已核实）。

### W-24 [P2] 顺序剧组熔断是"带观测语义的死字段"，测试自证

- **类别**：C 死代码 / E 测试质量
- **位置**：`crates/app-pipeline/src/sequential_crew.rs:103-107,144-157,596-603`
- **证据**：

```rust
// 103-107 注释自陈：生产路径只 record 不读
// L-12：失败计数。生产路径只 record 不读（record_failure 被调用，但
// failure_count/should_stop_actor 仅在 #[cfg(test)] 下编译）。
...
#[cfg(test)]
pub fn should_stop_actor(&self, actor_id: &str) -> bool { self.failure_count(actor_id) >= 2 }
```

- **影响**：`actor_is_stopped_after_two_failures` 验证的是仅测试编译的函数，给"有熔断"的假信心；生产里连续失败的演员只会重试 `MAX_ATTEMPTS_PER_ACTOR` 次（该部分真实存在）。注释已声明是待定稿策略，故列为低危但需在测试缺口清单里标注。
- **置信度**：高。

### W-25 [P2] MVU JS 变量快照把所有实例变量压平为一张表（key 冲突后者覆盖）

- **类别**：B 逻辑正确性 / 作用域
- **位置**：`crates/app-pipeline/src/lib.rs:3215-3227`（`build_current_variables`）
- **证据**：

```rust
fn build_current_variables(ctx: &WritingContext) -> ... {
    // campaign 变量 + 所有 CharacterInstance 的 variables 合并成一个 map（按 vv.key）
    // → key 相同（hp/love/…）时后遍历的实例覆盖前面的
```

- **影响**：MVU JS fallback 片段（`rt.execute_fragment(&frag.js_snippet, &current_variables)`，lib.rs:1517-1520）拿到的是"最后一个实例的值"，无法区分实例作用域；对多角色同名变量（`default_character_variables` 里 hp/love 等是模板键，天然重名）会读到错误的当前值。
- **建议**：按 `instance.<id>.<key>` 提供作用域视图（或让片段声明目标实例）。
- **置信度**：中高（代码事实）。

### W-26 [P3] 文档漂移：`PipelineState` 列表错误、"完整变体"缺两项

- **类别**：F 文档漂移
- **位置**：`docs/AGENT_INTERFACES.md:265`（`PipelineState`）、`docs/AGENT_INTERFACES.md:262-284`（变体清单）；代码 `crates/domain/src/agent.rs:384-399,450-461`
- **证据**：

```markdown
- `StateChanged { state }`（PipelineState：Generating / Editing / Review / Committed / Aborted）
```

```rust
// domain/src/agent.rs:384-398
pub enum PipelineState { Idle, Directing, Delegating, Editing, Review, Committed, Aborted }
```

- **影响**：文档写了一个不存在的 `Generating`，且漏了 `Idle/Directing/Delegating`；`QualityChecked`（agent.rs:450）与 `PromptHookRequest`（agent.rs:461）未出现在"完整变体"清单里。
- **建议**：同步清单（最好由测试断言 serde tag 集合）。
- **置信度**：高。

### W-27 [P3] 质量门禁注释与检查项清单过期；`owner_hint` 死代码；只查第一个「不是」

- **类别**：E/F 文档与死代码
- **位置**：`crates/app-pipeline/src/quality_gate.rs:4-12,250-262,286-301`
- **证据**：

```rust
/// Gate 失败不硬阻断——只标记警告，用户仍可手动 accept ...
if let Some(i) = text.find("不是") { ... }      // 只处理首个「不是」
let owner_id = binding.map(|b| b.owner_id.clone()).or(owner_hint).unwrap_or_default();  // owner_hint 恒不生效
```

- **影响**：注释与 `domain/src/turn.rs:515-518`（Error 默认拦截 accept）矛盾，头部检查项列表只有 6 项（实际 8 项 + contract 泄漏扫描）；"不是…而是…"只查首个命中（假阴性）；`owner_hint` 依集合包含关系永不生效。
- **置信度**：高。

### W-28 [P3] pp 事件契约：取消时先 `PostProcessFailed` 再 `PostProcessSkipped`

- **类别**：B 事件语义
- **位置**：`crates/app-pipeline/src/lib.rs:1596-1609`；`crates/tauri-app/src/runtime_support.rs:51-57`；前端 `frontend/src/composables/usePipeline.js:181-193`
- **影响**：用户取消后 `run_postprocess` 先把 `post_process=None` 当失败发 `PostProcessFailed`，调用方随后又因 `cancel` 发 `Skipped`；前端 `Skipped` 分支把状态重置为 `idle`（表现上被覆盖，但摘要块会留下 `error: 摘要未产出`）。两个互斥语义的事件同轮出现，契约不自洽。
- **置信度**：高。

### W-29 [P3] `match_braces` 为非字符边界 `pos` 会 panic（当前不可达）；`from_tool_call` 坏 arguments 无日志

- **类别**：D 健壮性
- **位置**：`crates/app-agent/src/llm_parse.rs:21-22,153-167`
- **证据**：

```rust
pub fn match_braces(content: &str, pos: usize) -> Option<usize> {
    let chars: Vec<char> = content[pos..].chars().collect();   // 非 char boundary → panic
```

- **影响**：全仓调用点均以 `find('{')`/`find('[')` 提供合法边界（`llm_parse.rs:82,102`、`character_extractor.rs:233-238`），当前**不可达**，但函数是 `pub` 且无前置条件文档；`from_tool_call` 对非法 `arguments` 静默返回 None（无 warn），工具调用坏掉时无法定位。
- **建议**：加 `is_char_boundary` 守卫；非法 args 记 warn。
- **置信度**：条件 高 / 可达性 低。

### W-30 [P3] 角色抽取杂项：drain 的 JoinError 被吞、双导出路、`[]` 与"解析失败"混淆

- **类别**：C/D
- **位置**：`crates/app-agent/src/character_extractor.rs:60-72,155-183`；`crates/app-agent/src/lib.rs:29-37`、`crates/app-agent/src/prompts/mod.rs:11-14`
- **影响**：①`let _ = drain.await;`（:67）丢弃 JoinError，注释声称让 panic 可观测但实际仍被吞；②同一批 item 有 `storyforge_app_agent::X` 与 `::prompts::X` 两条公开路径；③合法空数组 `[]` 在多层 `!parsed.is_empty()` 下被判为"5 层兜底全 miss"，调用方（campaigns.rs:166-169）降级成单角色卡 —— "空结果"与"解析失败"语义混淆。
- **置信度**：高。

### W-31 [P3] 前端 `validGenerationModes` 允许 `big_scene`，但模式目录里没有该项

- **类别**：F/C
- **位置**：`frontend/src/stores/writing.js:16-21,57-62`；`frontend/src/utils/generationModes.js:1-21`
- **影响**：旧 localStorage 里的 `big_scene` 会被接受并作为当前档位参与 `start_writing`/`allowPartialReroll` 判定，但 ComposerBar 只渲染 3 个档位 → 用户处于"看不到选中项"的昂贵模式，且与 `AGENT_INTERFACES.md:12`"仅保留后端兼容路径"的定位冲突。
- **置信度**：高。

### W-32 [P3] 重复实现：`format_subagent_context_*` 在 pipeline 与 app-agent 各一份

- **类别**：C 冗余
- **位置**：`crates/app-pipeline/src/lib.rs:4112-4153`（注释自陈"与 app-agent/runtime.rs 保持同步，独立实现避免跨 crate 耦合"）vs `crates/app-agent/src/runtime.rs:956-1002`
- **影响**：两份模板文本漂移时，**局部重 roll 的子 Agent prompt 与首轮不一致**（缓存段划分随之变化），且没有测试断言两者等价。
- **建议**：把格式化函数提到 `app-agent` 公共导出或加等价性测试。
- **置信度**：高。

## 4. 目标完成度核对表

| 声明能力（来源） | 代码证据 | 结论 |
| --- | --- | --- |
| Director→Subagent×N→Editor 主线（ARCHITECTURE/AGENT_INTERFACES） | `lib.rs:828-991`（big_scene）、`2396-2933`（stage 化）、`runtime.rs:612-830`（spawn_subagents） | ✅ 已落地 |
| continuation 单笔者（AGENT_INTERFACES:9，默认档） | `lib.rs:996-1190`（无 Director/Subagent/Editor，直接 Writer + dossier） | ✅ 已落地；⚠️ 整卷 reroll 会漂到 BigScene（W-03） |
| duet 对手戏 A-B-A | `lib.rs:1191-1432`、`sequential_crew.rs:164-195` | ✅ 已落地（`sequential_crew` 被 duet 复用做 beat 序列，EditorPromptFlavor::Duet） |
| sequential_crew 顺序剧组 | `lib.rs:2609-2684`、`sequential_crew.rs:201-333` | ✅ 已落地；公开 beat 隔离已核实无问题 |
| big_scene 旧并行兼容档 | `lib.rs:808,1841,1859`；前端目录不暴露 | ✅ 代码可达（非 campaign reroll 默认走到它）；⚠️ 与 W-03/W-18 相关 |
| QualityGate + 有界 1× Editor auto-fix | `quality_gate.rs:17-22`；`production_postprocess.rs:179-236`（`revise_draft` 只调一次，失败回滚原稿） | ✅ 已落地；⚠️ 误报与无长度守卫（W-04/W-20/W-21） |
| 后处理三件套（知识/变量/任务） | `postprocess.rs:180-344`、`pipeline_postprocess.rs:169-230` | ✅ 已落地；⚠️ W-05/W-09/W-10/W-11 |
| Summarizer | `summarizer.rs`、`prompts/summarizer.rs`；`pipeline_postprocess.rs:132-167` | ✅ 已落地；⚠️ 隔离口径 W-13 |
| Provenance（plan/subagent 快照/推理预算/`generation_mode`） | `build_provenance_with_campaign`（`app-conversation/src/lib.rs:886-915`）、`lib.rs:73-97` 预算校验、`2863` | ✅ 已落地（`SubagentSnapshot` 三字段与 CLAUDE.md 一致） |
| CharacterInstance 身份与信息隔离 | `runtime.rs:656-679,730-741`、`tools.rs:936-987`（绑定后硬失败防泄漏） | ⚠️ 主路径成立，但**归一不一致**造成降级（W-01）；同名歧义（W-08） |
| reroll 策略（产品模式整体重写 / big_scene 局部 / sequential 后缀重演） | `lib.rs:1672-1760`、`2162-2392`；`AGENT_INTERFACES.md:155` | ✅ 与文档一致（含 `generation_mode` 校验）；⚠️ 空 targets 的 None 模式缺口（W-03）、集合校验缺失（W-16） |
| `model_override`/`max_tool_rounds` 运行时消费 | Director `lib.rs:3543-3555`；Editor `3812+`；Writer `3592-3599`；Subagent `runtime.rs:644-651`；PostProcessor/Summarizer（app-agent） | ✅ 真实消费；⚠️ `AgentRole::Writer` 不在配置面板角色表（`AgentProfileManager.vue:34-38`）→ continuation 档的模型覆盖无法从 UI 设置；reroll 路径 C 硬编码（W-19） |
| `tool_whitelist` 运行时消费 | Director `lib.rs:2431-2437`；Subagent `runtime.rs:749-754`；PostProcessor `postprocess.rs:66-74` | ✅ 已核实（`None`=默认、`Some([])`=清空、未知名 warn+ignore；被删工具 dispatch 返回 `ToolError::NotFound`） |
| `enable_postprocess`/`enable_summarizer` | `lib.rs:1447-1467`、`pipeline_postprocess.rs:132-173,188-194`；both-off → `PostProcessSkipped` | ✅ 已落地；⚠️ 失败/取消事件语义（W-28） |
| `max_concurrent_subagents` 0 值钳制 | `lib.rs:2583-2588`、`runtime.rs:628-631`、`domain/agent_profile_config.rs:172-180` | ✅ 已落地 |
| 三水位（ContextEpoch/Chronicle/MemoryArchiver） | `lib.rs:348-502,1072-1090`；`chronicle_compressor.rs:59-68`；`archiver.rs:120-230` | ✅ 已落地；⚠️ archiver 水位缺口（W-06）、召回口径（W-14） |
| 失败传播与 best-effort 边界 | 后处理/摘要 best-effort（`pipeline_postprocess.rs`）；关键落盘失败向上返回（`writing.rs:284-326`） | ⚠️ 边界总体正确，但"静默降级"过多（W-05/W-06/W-23） |

## 5. 未发现问题与低风险观察

**未发现问题（已核对范围）**：

- **`run_tool_loop_core` 的工具循环健壮性**（`runtime.rs:393-577` 抽读 + `execute_tool_call` 98-123）：畸形 tool-call JSON 不 dispatch、不 panic，只回 `{"error":...}` 给模型；`max_rounds` 退出路径与 `tool_calls.is_empty()` 提前返回已核实；三种入口共用同一核心（Gate 2 Batch 2.5 的去重没有留下双实现）。
- **`spawn_subagents` 并发与对齐**（`runtime.rs:612-830`）：`Semaphore` 排队不丢任务、结果按原始 index 对齐、`max(1)` 防御 0、取消 watch 联动、每个子 Agent 独立 `ToolContext`（`current_character_instance_id` 绑定）——隔离的正向路径成立。
- **子 Agent `get_character` 绑定后硬失败**（`tools.rs:968-979`）：绑定 id 在 runtime 中找不到时拒绝降级到扁平查询，防泄漏设计正确（与 CLAUDE.md 的 P0 修复一致）。
- **顺序剧组隔离**（`sequential_crew.rs:31-37,119-140,220-322`）：下一位演员只拿到 `narrative+dialogue` 组成的公开拍，`inner_thoughts`/reasoning 不进 stage；测试 `runner_passes_only_prior_public_performance_to_next_actor` 断言的是真实 prompt。
- **`current_character_instance_id` 与 `campaign_runtime` 组装**：`spawn_subagents` 用同一份 `effective_runtime`（含 temps）做匹配与 tool ctx，未发现"匹配用 A、工具用 B"的不一致。
- **`pending_temporary_instances` 生命周期**：`start_writing`(864)/continuation(1008)/duet(1203)/regenerate(1646) 四条路径均清理陈旧数据，与 CLAUDE.md 一致。
- **`validate_provenance_reasoning_budget`**（`lib.rs:73-97`）聚合三类推理字节并 fail-closed，未发现绕过。
- **UTF-8 安全截断**：`truncate_chars`(3485-3492)/`truncate_chars_pub`(337-343)/`overview_headline`/`chars().take()` 系列全部按 `chars()`；`llm_parse` 的配平与切片（`byte_offset += ch.len_utf8()`）落在字符边界。
- **`turn_dossier` 的 id 语义与确定性排序**（`turn_dossier.rs:158-197`）：roster/actor/`character_id` 全部来自 `instance.id`，duet 建 task 也用 `actor.id`（`lib.rs:1239-1253`）；排序 = (priority, original_index) 稳定。
- **`enable_*` 门禁本身**：两个开关都关时在 `PostProcessStarted` 之前返回 `PostProcessSkipped`，不会把"配置关闭"报成失败（`lib.rs:1454-1462`）。
- **`app-memory` 的关键词提取与合并去重**（`archiver.rs:290-361`、`recall.rs:106-127,209-235`）：同频稳定排序、bigram 逻辑正确。
- **`app-conversation` 的 `with_conversation_mut`**（`211-251,134-139,146-188`）：回滚、毒化锁、外部 authority fail-closed 设计正确。
- **`tool_whitelist` 过滤语义**（`tools.rs:266-286` + 三个注册点）：覆盖 `None/Some([])/Some(list)/未知名` 四种分支，测试充分。
- **big_scene 与 flat Character 路径不是死代码**：非 Campaign 的 legacy 写作与 reroll 仍在走它们（`useWriting.js:133-141`、`useMessageVariants.js:167`），CLAUDE.md"不得破坏非 Campaign 路径"的约束下**不能删**；但重复实现（W-32）与路由死代码（W-18）需要清理。

**低风险观察（不计入发现编号）**：

- `PipelineOrchestrator` 落盘后不发 `PipelineEvent::Committed`（只在 `lib.rs:2906-2909` 发 `StateChanged`；`Committed` 由 `tauri-app/src/commands/writing.rs:493` 在 accept 时发）——事件命名上容易误解，但与前端"committed=terminal accept"的语义一致（`plugin-bridge.js:338`）。
- `generate`/`regenerate` 的 `session.intent` 在 reroll 时取 `hint` 或 `plan.scene_brief`（`lib.rs:2916-2929`），与首轮用户意图不同，仅影响展示。
- `run_director_stage` 把 `input.hint` 用 `EDITOR_HINT_MARKER`（"【上次问题】"）拼进 Director tail（`lib.rs:2476-2478`），标记名与角色不匹配（应为 `SUBAGENT_HINT_MARKER` 语义或独立常量），当前只影响提示词措辞。
- `quality_gate` 的 `nearby_window` 每次命中都 `char_indices().collect()`（`quality_gate.rs:420-433`），长草稿多 probe 时 O(命中×n) 分配。
- 前端 `postprocess_failed`/`postprocess_skipped` 都会在摘要仍 running 时置 `summary=error('摘要未产出')`（`usePipeline.js:181-193`），语义上把"postprocess 关闭"也算摘要失败（此时摘要通常已 done，影响面小）。

## 6. 需要 Lead 重点复核的结论

1. **W-01 的触发频率需要真实 LLM 证据**：机制（trim/大小写两套归一）100% 确定，但"Director 是否会输出大小写/空白变体"属探测项。建议在 `harness-real-llm` 用真实模型跑多轮、统计 `matched_instance=None` 的 `fallback_reason` 计数（该字段已在 `SubagentSnapshot` 里，零改动即可取证）。
2. **W-03 是否为有意设计**：`generation_mode=None → BigScene` 可能是刻意保留的 legacy 兼容（前端非 campaign 不传 mode），但它与"continuation 为默认档"的产品语义冲突。请产品/Lead 判定后决定是"继承旧 provenance 模式"还是"补文档说明"。
3. **W-11 的跨域部分需要域 1/2 复核**：app-agent 输出"名字级 ID"的契约（域文档却写 ID 是 instance id）与 Tauri 侧 `resolve`/`source_propagation_blocks` 的 fail-open 行为叠加，才构成"来源不可解析时 private 知识可外播"。本报告只对 app-agent 侧的 DTO 与无校验下了结论，下游门禁的行为需在域 1/2 确认是否为已知取舍。
4. **W-06 水位语义**：`archive_prefix` 是否需要返回"连续成功前缀"是接口级决策（影响 JSON/SQLite 两后端与自动归档调用方）。当前实现下"批次失败"是不可自愈的数据丢失，建议按 P1 排期。
5. **W-13/W-14 的隔离口径**：摘要与召回是否被视为"公共事实"？若否，需要按 instance 过滤，属跨域（app-agent + app-memory + 域 1 的持久化）改造，建议先定规格（MEMORY-CONTEXT-COMPILER-SPEC）再动手。
6. **测试缺口（E 维度汇总）**，建议补在对应 crate：
   - `app-conversation/src/lib.rs:1487-1535`：无重复/未知/空 targets 用例；`1539-1579`：`replace_active_variant` 无中间节点/非 campaign 用例。
   - `app-agent/src/runtime.rs`：`test_spawn_subagents_campaign_fallback_to_name`（2264）覆盖了 name 兜底，但**没有大小写/空白变体**用例（W-01）；`sequential_crew.rs:431,514` 的 profile 参数恒为 `None`，per-actor 覆盖零覆盖。
   - `tools.rs:1331-1357`：同名歧义查询（"码头工人"）无断言（W-08）。
   - `postprocess.rs:468-583`：无"单条畸形不拖垮其它两类"用例（W-05）、无 broadcast 大小写用例（W-10）、无 `new_status` 缺失用例。
   - `pipeline_postprocess.rs:319-344`：只断言 outcome 不断言 LLM 调用次数（W-09 的"空结果仍重试"回归会通过）。
   - `app-memory/src/archiver.rs:408-446`：`maybe_archive`/`archive_prefix` 的阈值、批次失败、嵌入失败、水位推进语义零覆盖（W-06/W-15）。
   - `quality_gate.rs`：缺"owner+他人标签同窗"（W-21）与"非首个『不是』"（W-27）用例。
