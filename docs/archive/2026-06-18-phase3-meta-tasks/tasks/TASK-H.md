# TASK-H：Campaign-aware Meta Tools（Phase 3 阶段 4 收尾）

> 第五轮单任务。从 main（HEAD `8494261`）拉 worktree + 分支 `codex/meta-campaign-tools`。
> 改动文件互不重叠，全在单一 worktree。

## 目标

让 Meta Agent 在多轮对话中能 inspect active Campaign 的 instances / variables / knowledge / tasks，并能提议 typed patch 走已有 preview/accept 闭环。做完后 ROADMAP Phase 3 关闭。

## 背景：现有架构（审计确认）

- `MetaSession`（`meta_conversation.rs:41`）当前字段：`character` / `world_info` / `patches`（PatchStore，自由文本 patch）/ `explainer`。**没有 Campaign 数据源**。
- `register_meta_runtime_tools`（`meta_conversation.rs:265`）注册 4 个工具：`meta_inspect_world_info` / `meta_inspect_character` / `meta_propose_patch` / `inspect_generation`。
- `chat()` 返回 `MetaTurn { agent_message, new_patch: Option<Patch> }`（`meta_conversation.rs:142`）。
- tauri-app `sync_meta_session_from_tool_ctx`（`lib.rs:2976`）只同步 character + world_info，**不同步 campaign_runtime**。
- tauri-app `meta_chat`（`lib.rs:3050`）把 `turn.new_patch` 同步进 `app.meta_patches`。
- `state.typed_patches`（`lib.rs:3248`）是 typed patch 统一存储；`meta_propose_campaign_repairs` 把 health-issue-driven 的 patch 追加进去；4 个命令 `meta_list_typed_patches` / `meta_preview_typed_patch` / `meta_accept_typed_patch` / `meta_dismiss_typed_patch` 基于它。
- `TypedPatchAction`（`typed_patch.rs:21`）现有 4 变体：`SyncInstanceVariables` / `PruneOrphanTaskReferences` / `DeleteOrphanKnowledge` / `RepointInstanceDefinition`，全由 `build_patch_for_issue`（health issue 驱动）构造。

## 改动 1：`crates/app-meta/src/meta_conversation.rs`

### 1.1 `MetaSession` 新增字段

```rust
pub struct MetaSession {
    pub character: Mutex<Option<Arc<Character>>>,
    pub world_info: Mutex<Option<Arc<WorldInfoBook>>>,
    pub patches: PatchStore,
    pub explainer: Option<Arc<dyn GenerationExplainer>>,
    // 新增：active Campaign 运行时快照（inspect_* 工具读它，None = 无 active campaign）
    pub campaign_runtime: Mutex<Option<Arc<CampaignRuntimeContext>>>,
    // 新增：Agent 通过 propose_campaign_patch 工具提议的 typed patch（chat 返回后 drain 到 AppState）
    pub typed_patches: Mutex<Vec<TypedPatch>>,
}
```

- `MetaSession::new()` / `default()` 初始化两个新字段为 None / 空 Vec。
- 新增 `set_campaign_runtime(&self, ctx: Arc<CampaignRuntimeContext>)`。
- `TypedPatch` 已在 crate 内导出（`typed_patch.rs`），直接 `use crate::typed_patch::TypedPatch`。
- `CampaignRuntimeContext` 来自 `storyforge_domain::campaign_runtime`，app-meta 已依赖 domain。

### 1.2 `MetaTurn` 新增字段

```rust
pub struct MetaTurn {
    pub agent_message: MetaMessage,
    pub new_patch: Option<Patch>,
    // 新增：本轮 Agent 通过 propose_campaign_patch 提议的 typed patch
    pub new_typed_patches: Vec<TypedPatch>,
}
```

`chat()` 末尾从 `session.typed_patches` drain 出本轮新增（记下进入时的长度，drain 该长度之后的所有项），填入 `new_typed_patches`。

### 1.3 `register_meta_runtime_tools` 注册 6 个新工具

所有新工具 handler 通过 `session.campaign_runtime.lock()` 取快照；None 时返回 `{"error": "当前没有 active Campaign"}`。

#### `inspect_campaign`（无参）
返回 campaign 概览：
```json
{
  "campaign_id": "...", "name": "...", "turn": 3,
  "instance_count": 4, "knowledge_count": 12, "task_count": 5,
  "pending_task_count": 2, "campaign_variables": [{"key":"story_clock","value":"..."}]
}
```

#### `inspect_instance`（参 `instance_id_or_name`）
用 `ctx.find_instance_by_id_or_name` 查；返回 instance id/name/definition_id/is_temporary/resolved_persona/resolved_behavior/variables。

#### `inspect_variables`（参 `scope`: "campaign"|"instance", 可选 `instance_id_or_name`）
- `scope=campaign`：返回 `campaign.variables`
- `scope=instance`：需 `instance_id_or_name`，返回该 instance 的 variables

#### `inspect_knowledge`（可选 `instance_id_or_name`）
- 无参：返回全部 `ctx.knowledge`（摘要：id/character_id/knowledge_text 前 80 字/source）
- 有参：`ctx.knowledge_for_instance(inst)` 过滤

#### `inspect_tasks`（可选 `status`: "pending"|"all"，默认 pending）
- pending：`task` 里 status 为 pending/active 的
- all：全部 task

#### `propose_campaign_patch`（参 `description`: String, `action`: TypedPatchAction JSON）
- 调 `crate::typed_patch::build_patch_from_action(description, action, &input)` 构造 TypedPatch
- `input` 从当前 campaign_runtime 快照构建 PreviewInput（instances/definitions/knowledge/tasks）
- 成功：把 patch push 进 `session.typed_patches`，返回 `{"patch_id": ..., "description": ..., "status": "已提议，等待用户预览/接受"}`
- 失败（target 不存在）：返回 `{"error": "..."}`，不存

**注意**：`build_patch_from_action` 需要 definitions 切片，而 `CampaignRuntimeContext.definitions_by_id` 是 HashMap。在 handler 里把 HashMap 的 values 收集成 Vec 传给 PreviewInput。

### 1.4 `ToolResultDisplay`

**不新增变体**。6 个新工具的返回 JSON 直接给 Agent，Agent 用自然语言转述。`chat()` 的 tool_result 解析循环对新工具名走 `_ => {}`（已有默认分支）。

## 改动 2：`crates/app-meta/src/typed_patch.rs`

### 2.1 新增 4 个 `TypedPatchAction` 变体

```rust
pub enum TypedPatchAction {
    // 现有 4 个不变 ...
    SyncInstanceVariables { ... },
    PruneOrphanTaskReferences { ... },
    DeleteOrphanKnowledge { ... },
    RepointInstanceDefinition { ... },
    // 新增：
    /// 改 Campaign 级变量（如 story_clock）
    UpdateCampaignVariable { key: String, value: serde_json::Value },
    /// 改某 instance 的变量（如 hp）
    UpdateInstanceVariable { instance_id: Id, key: String, value: serde_json::Value },
    /// 给 instance 加一条知识
    AddKnowledge { character_id: Id, knowledge_text: String, source: KnowledgeSource },
    /// 改任务状态
    UpdateTaskStatus { task_id: Id, new_status: TaskStatus },
}
```

- `KnowledgeSource` / `TaskStatus` 来自 domain（`character_knowledge.rs` / `story_task.rs`），确认其 Serialize/Deserialize 后直接复用。若 enum 形态不便 LLM 构造，定义一个窄 DTO 在 typed_patch.rs 内，build 时转换。
- `apply_action` 为每个新变体加分支：
  - `UpdateCampaignVariable`：找不到 campaign → TargetMissing（注：PreviewInput 当前无 campaign 字段，见 2.3）
  - `UpdateInstanceVariable`：找不到 instance → TargetMissing；找到则 set_variable
  - `AddKnowledge`：生成新 `CharacterKnowledgeEntry`，push 进 knowledge
  - `UpdateTaskStatus`：找不到 task → TargetMissing；找到则改 status
- `is_patch_stale` 为每个新变体加分支（target 是否仍存在）。

### 2.2 新增纯函数 `build_patch_from_action`

```rust
pub fn build_patch_from_action(
    description: String,
    action: TypedPatchAction,
    input: &PreviewInput,
) -> Result<TypedPatch, TypedPatchError>
```

- 校验 action 的 target 在 input 中存在（不存在 → TargetMissing）
- 构造 diff（before/after）
- 构造 TypedPatch（id 新生成，source_issue_category = "agent_proposed"，status = Pending）
- `source_issue_category` 用新值 `"agent_proposed"`（区别于 health-issue-driven 的 4 类）

### 2.3 `PreviewInput` 扩展

当前 `PreviewInput` 无 campaign 字段，`UpdateCampaignVariable` 的 apply/stale 检查需要。新增：

```rust
pub struct PreviewInput<'a> {
    pub instances: &'a [CharacterInstance],
    pub definitions: &'a [CharacterDefinition],
    pub knowledge: &'a [CharacterKnowledgeEntry],
    pub tasks: &'a [StoryTask],
    pub campaign: Option<&'a Campaign>,  // 新增：UpdateCampaignVariable 用
}
```

- 现有 `meta_propose_campaign_repairs`（tauri-app）构建 PreviewInput 时补 `campaign: Some(&campaign)`。
- `build_patch_for_issue` / `is_patch_stale` 现有逻辑不受影响（campaign 字段仅新变体读）。

## 改动 3：`crates/app-meta/src/prompts/meta_agent.rs`

### 3.1 `META_AGENT_SYSTEM_PROMPT` 增补

在「四大能力」后加「Campaign 诊断」能力块：

```
【Campaign 诊断（需 active Campaign）】
当用户问及当前 Campaign 的实例、变量、知识、任务时，调用：
- inspect_campaign：Campaign 概览（实例数/任务数/变量）
- inspect_instance：查某角色的实例详情（persona/behavior/变量）
- inspect_variables：查 Campaign 级或角色级变量
- inspect_knowledge：查角色可见知识
- inspect_tasks：查待办任务
- propose_campaign_patch：提议类型化修复（变量/知识/任务状态），用户预览后才写盘

无 active Campaign 时，明确告诉用户「当前没有 active Campaign，只能做角色卡/世界书层面诊断」。
propose_campaign_patch 的 action 必须包含正确 target id（先 inspect 拿到真实 id 再提议）。
```

- 同步更新 `register_meta_tools` 的占位 spec（与 `register_meta_runtime_tools` 对齐，保持 spec 一致性，避免两边漂移）。本轮 `register_meta_tools` 仍只注册占位 handler，真 handler 在 runtime 版。

## 改动 4：`crates/tauri-app/src/lib.rs`

### 4.1 `sync_meta_session_from_tool_ctx` 注入 campaign_runtime

```rust
fn sync_meta_session_from_tool_ctx(state: &tauri::State<'_, Arc<AppState>>) {
    let ctx = state.tool_ctx.read().unwrap_or_else(|p| p.into_inner());
    if let Some(card) = ctx.characters.last() {
        state.meta_session.set_character(card.clone());
    }
    if let Some(book) = &ctx.world_info {
        state.meta_session.set_world_info(book.clone());
    }
    // 新增：同步 campaign runtime 快照
    if let Some(rt) = &ctx.campaign_runtime {
        state.meta_session.set_campaign_runtime(rt.clone());
    } else {
        // 无 active campaign 时清空，避免读到过期快照
        *state.meta_session.campaign_runtime.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
}
```

### 4.2 `meta_chat` 同步 new_typed_patches

在现有 `if let Some(patch) = &turn.new_patch { ... }` 块之后加：

```rust
if !turn.new_typed_patches.is_empty() {
    let mut typed = app.typed_patches.write().unwrap_or_else(|p| p.into_inner());
    for tp in &turn.new_typed_patches {
        if !typed.iter().any(|p| p.id == tp.id) {
            typed.push(tp.clone());
        }
    }
}
```

返回 JSON 加 `"new_typed_patches": turn.new_typed_patches` 字段。

## 改动 5：`frontend/src/components/MetaPanel.vue`

`handleSend` 里 `metaChat` 返回后：

```js
if (result.new_typed_patches && result.new_typed_patches.length > 0) {
  // Agent 提议了 typed patch，刷新列表让用户看到
  await refreshTypedPatches()  // 新增：调 metaListTypedPatches
}
```

- 新增 `refreshTypedPatches` 函数（调 `metaListTypedPatches`，填 `typedPatches.value`，并对每条跑 preview 标记 stale——复用现有 `handleProposeRepairs` 里的 preview 逻辑）。
- `tauri-api.js` 确认 `metaListTypedPatches` 已导出（应已有，因 MetaPanel 已用 typed patch）。

## 测试

### `typed_patch.rs` 测试
- 每个新变体的 `build_patch_from_action` 成功 + target 缺失失败
- 每个新变体的 `apply_to_snapshot` 应用后字段正确
- 每个新变体的 `is_patch_stale`（target 存在 / 移除）
- `build_patch_from_action` 设置 `source_issue_category == "agent_proposed"`

### `meta_conversation.rs` 测试
- 6 个工具均注册（扩展现有 `test_inspect_generation_tool_registered` 模式）
- `inspect_campaign` handler：注入 mock CampaignRuntimeContext → 返回概览；None → 返回 error
- `inspect_instance` handler：按 id / 按 name / 找不到
- `propose_campaign_patch` handler：提议合法 action → session.typed_patches 增长 1；提议 target 缺失 → 不增长，返回 error
- `MetaTurn.new_typed_patches`：propose 后 chat 返回的 new_typed_patches 非空（用 mock LLM 触发工具调用，或直接单测 drain 逻辑）

### tauri-app 测试
- 扩展 `test_meta_session_explainer_is_injected`（`lib.rs:4603`）：断言 `meta_session.campaign_runtime` 在有 active campaign 时被注入（None 时为 None）

## 验证命令

```bash
cargo test -p storyforge-app-meta
cargo test -p storyforge --lib
cargo check --workspace
```

## 禁止

- 不让 Meta Agent 直接写 store（propose_campaign_patch 只存 session.typed_patches，写盘走 meta_accept_typed_patch）。
- 不为 inspect 工具新增 ToolResultDisplay 变体（保持 MetaPanel 简单）。
- 不做 PatchCharacterDefinition / MergeDuplicateInstance（留后续）。
- 不破坏现有 4 个 TypedPatchAction 变体的语义和测试。
