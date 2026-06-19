# 计划：Campaign 写作主线

> **归档说明**：本文件已于 2026-06-19 归档为历史执行记录。所有 6 个阶段（Phase 1/2）均已实现并合并到 main。后续执行入口见 `docs/ROADMAP.md` Phase 1/2。归档位置：`docs/archive/2026-06-19-completed-phases/PLAN-CAMPAIGN-MAINLINE.md`。

## 目标

把开 Campaign 后的写作链路从“扁平 Character 主导”改成“Campaign + CharacterInstance 主导”。

完成后：

- 未开 Campaign：继续兼容现有 SillyTavern 单卡写作。
- 已开 Campaign：Director、Subagent、Postprocess、Provenance 都优先使用 `CharacterInstance.id`。
- 变量、知识、任务、摘要能进入下一轮写作，而不是只在 Campaign 面板里展示。

## 非目标

- 不重写 JSON store。
- 不删除 `CharacterStore`。
- 不把 `CampaignStore` 直接依赖进 `app-agent` 或 `app-pipeline`。
- 不做通用插件系统。
- 不改前端大布局，前端入口改造放到 `PLAN-FRONTEND-WORKBENCH.md`。

## 当前事实

- `crates/tauri-app/src/lib.rs::start_writing` 用 `snapshot_tool_ctx()` 构造 `WritingContext.characters`。
- `crates/tauri-app/src/lib.rs::fill_campaign_context` 阶段 2 已扩展：加载 instances、definitions、knowledge，组装 `Arc<CampaignRuntimeContext>` 写入 `ctx.campaign_runtime` 并同步到 `tool_ctx`。开头先清空旧 runtime 防止 stale。
- `crates/app-pipeline/src/lib.rs::WritingContext` 阶段 2 已新增 `campaign_runtime: Option<Arc<CampaignRuntimeContext>>`。
- `crates/app-pipeline/src/lib.rs::build_director_tail` **阶段 3 已改造**：有 `campaign_runtime` 时从 instances 渲染（含 id/role_type/persona 摘要 + instance variables），campaign 全局变量注入 volatile tail，UTF-8 安全截断；无时退回旧逻辑。
- `crates/app-pipeline/src/lib.rs::has_available_characters` **阶段 3 新增**：兼容 Campaign（instances 非空）和旧路径（characters 非空），`start_writing` 和 `regenerate` 共用。
- `crates/app-pipeline/src/lib.rs::DIRECTOR_SYSTEM_PROMPT` **阶段 3 已更新**：character_id 规则区分 Campaign 实例（用 instance_id）和旧路径（用角色名）。
- `crates/app-agent/src/tools.rs::ToolContext` 阶段 2 已新增 `campaign_runtime: Option<Arc<CampaignRuntimeContext>>`。
- `crates/app-agent/src/tools.rs` 导演 `get_character` **阶段 3 已改造**：有 `campaign_runtime` 时优先查实例（返回 id/definition/persona/behavior/variables），查不到时 fallback 到旧扁平 Character。
- `crates/app-agent/src/runtime.rs::spawn_subagents` **阶段 4 已改造**：接收 `campaign_runtime: Option<Arc<CampaignRuntimeContext>>`，按 character_id 匹配 instance，使用 resolved persona/behavior 构造 system，注入该 instance 的 knowledge（信息隔离）和 variables，为每个子 Agent 构造独立 ToolContext（绑定 `current_character_instance_id`），注册子 Agent 工具。未匹配时 fallback 到旧 context_package。无 campaign_runtime 时走旧路径。
- `crates/app-agent/src/runtime.rs::build_campaign_subagent_system` **阶段 4 cleanup 新增**：纯函数，构造 Campaign 模式子 Agent 的 system prompt。测试可直接断言 prompt 内容。
- `crates/app-agent/src/runtime.rs::build_campaign_subagent_volatile` **阶段 4 cleanup 新增**：纯函数，构造 Campaign 模式子 Agent 的 volatile tail 文本。测试可直接断言 knowledge 隔离和 variables 注入。
- `crates/app-agent/src/tools.rs::register_subagent_tools` **阶段 4 已改造**：子 Agent get_character 有 `current_character_instance_id` 时只返回自己的 instance 数据，不泄露其他角色。无时退回旧扁平 Character。
- `crates/tauri-app/src/lib.rs::persist_postprocess_outcome` **阶段 5 已改造**：知识/变量写入会先把角色名或 id 解析成已持久化的 `CharacterInstance.id`，`present_chars` 真正用于校验知识和变量写入（不出场角色不写入），task 状态更新会校验 task 属于当前 campaign。阶段 6：临时 instance 在 postprocess 前已落盘，其知识/变量写回不再被跳过。
- `crates/domain/src/conversation.rs::SubagentSnapshot` **阶段 5 已扩展**：新增 `character_instance_id`、`display_name`、`fallback_reason` 字段（`Option`，serde 兼容旧数据）。
- `crates/app-conversation/src/lib.rs::build_provenance_with_campaign` **阶段 5 新增**：接收 `CampaignRuntimeContext`，从 Performance 中提取 instance 信息填充 SubagentSnapshot。
- `crates/tauri-app/src/lib.rs::CharacterInfo` **阶段 5 已扩展**：新导入卡会保存 `source_character_id: Option<String>`，启动恢复时用它保留 domain `Character.id`；旧数据没有该字段时 fallback 到 `StoredCharacter.id`。
- `crates/tauri-app/src/lib.rs::delete_character` **阶段 5 已修复**：级联删除时同时尝试 `StoredCharacter.id`、持久化 `source_character_id`、同会话 `tool_ctx` 中按角色名找到的 domain `Character.id`。
- `crates/domain/src/campaign.rs::CharacterInstance::temporary_with_overrides` **阶段 6 已完成**：创建临时 instance 时可传入 persona/behavior override。
- `crates/domain/src/campaign_runtime.rs::CampaignRuntimeContext::with_temporaries_for` **阶段 6 已完成**：为未匹配的 character_id 创建临时 `CharacterInstance`（支持 persona/behavior override），返回 `Vec<CharacterInstance>` 供调用者持久化；同一批次内重复 unmatched character 会去重。
- `crates/app-pipeline/src/lib.rs::PipelineOrchestrator::pending_temporary_instances` **阶段 6 已完成**：存储本轮创建的临时 instance，Tauri 层通过 getter 读取后落盘；`start_writing` / `regenerate` 开始时会清空旧 pending，避免失败或重试污染下一轮。
- `crates/app-pipeline/src/lib.rs::start_writing` / `regenerate` **阶段 6 已完成**：从 Director 的 `context_package.character_brief` 提取 persona 注入临时 instance，存储到 `pending_temporary_instances`。
- `crates/tauri-app/src/lib.rs::persist_temporary_instances_to` **阶段 6 已完成**：只在 pipeline 返回 `Ok` 后、postprocess 之前把临时 instance 写入 CampaignStore；会跳过同 campaign 已存在同名 instance、同批重复临时 instance，以及 `campaign_id` 不匹配的临时 instance。落盘后 postprocess 知识/变量写回不再被跳过。
- `request_ad_hoc_character` 工具**经评估不必实现**：当前 unmatched character_id 自动触发 + `context_package.character_brief` 作为 persona 注入已完整覆盖 Director 主动声明新角色的需求（见阶段 6 评估说明）。

## 阶段 0：基线保护

目标：执行前确认当前测试和关键路径状态，避免把已有坏状态误归因到本计划。

改动文件：无。

操作：

1. 运行 `cargo test -p storyforge-domain`。
2. 运行 `cargo test -p storyforge-app-agent`。
3. 运行 `cargo test -p storyforge-app-pipeline`。
4. 运行 `cargo test -p storyforge`。
5. 记录失败项，不修无关失败。

验收：

- 有一份测试基线记录。
- 不产生代码改动。

## 阶段 1：补齐 domain 层读取能力

目标：让 `CharacterInstance` 能从 `CharacterDefinition` fallback 出完整设定。

改动文件：

- `crates/domain/src/campaign.rs`
- 如需要，新增 `crates/domain/src/campaign_runtime.rs`
- `crates/domain/src/lib.rs`

任务：

1. 给 `CharacterInstance` 或 `CampaignRuntimeContext` 增加读取 helper。当前代码里 `CharacterInstance` 只有 `persona_override` / `behavior_override`，没有 backstory override；不要假装实例已经能承载所有 definition 字段：
   - `resolved_persona(definition: Option<&CharacterDefinition>)`
   - `resolved_behavior(definition: Option<&CharacterDefinition>)`
   - `resolved_backstory` 可作为 runtime/helper 函数直接读取 `CharacterDefinition.base_backstory`；只有显式新增 instance backstory override 字段后，才放到 `CharacterInstance` 方法上。
   - `resolved_variable_schema` 可作为 runtime/helper 函数直接读取 `CharacterDefinition.variable_schema`；实例只保存当前变量值。
2. persona/behavior 走 override 优先，definition 兜底。
3. 不把 definition 字段复制进 instance，保持 instance 轻量。
4. 增加单元测试覆盖 override、fallback、none，以及 backstory/schema helper 不依赖实例复制字段。

验证：

```bash
cargo test -p storyforge-domain
```

验收：

- `CharacterInstance` 不再只能返回 override。
- 现有序列化格式不破。

## 阶段 2：新增 CampaignRuntimeContext 快照

目标：让 pipeline/agent 能读取 Campaign 运行态，但不依赖 Tauri store。

改动文件：

- `crates/domain/src/campaign_runtime.rs` 或 `crates/domain/src/campaign.rs`
- `crates/domain/src/lib.rs`
- `crates/app-pipeline/src/lib.rs`
- `crates/app-agent/src/tools.rs`
- 所有 `ToolContext { ... }` 构造点
- 所有 `WritingContext { ... }` 构造点

任务：

1. 新增纯 domain DTO：

```rust
pub struct CampaignRuntimeContext {
    pub campaign: Campaign,
    pub instances: Vec<CharacterInstance>,
    pub definitions_by_id: HashMap<Id, CharacterDefinition>,
    pub knowledge: Vec<CharacterKnowledgeEntry>,
    pub turn: u32,
}
```

2. 增加 helper：
   - `find_instance_by_id_or_name`
   - `definition_for_instance`
   - `display_name`
   - `knowledge_for_instance`
3. `WritingContext` 增加 `campaign_runtime: Option<Arc<CampaignRuntimeContext>>`。
4. `ToolContext` 增加同名字段。
5. legacy 构造和测试构造全部补 `None`。
6. 没有 active Campaign 时完全走旧路径。

验证：

```bash
cargo build --workspace
cargo test -p storyforge-domain -p storyforge-app-agent -p storyforge-app-pipeline
```

验收：

- 依赖方向仍是 `app-* -> domain`，没有 `app-agent -> tauri-app`。
- 所有旧测试构造点编译通过。

## 阶段 3：Tauri 装配 Campaign 快照

目标：让开 Campaign 后的 `WritingContext` 和 `ToolContext` 都拿到同一份只读快照。

改动文件：

- `crates/tauri-app/src/lib.rs`
- 如需测试辅助，`crates/tauri-app/src/campaign_store.rs`

任务：

1. 在 `fill_campaign_context` 中加载：
   - `campaign = store.get_campaign(active_id)`
   - `instances = store.list_instances(active_id)`
   - `card = store.get_card(campaign.card_id)`
   - `definitions_by_id`
   - `knowledge = store.list_knowledge(active_id)`
2. 计算 turn 继续使用 summaries 数量 + 1。
3. 组装 `Arc<CampaignRuntimeContext>`。
4. 写入：
   - `ctx.campaign_runtime = Some(runtime.clone())`
   - 当前写作使用的 tool snapshot 或 `state.tool_ctx` 快照字段。
5. 明确错误策略：
   - active campaign id 不存在：fallback 到旧路径并 warn。
   - campaign 有 0 个 instance：返回友好错误，不让 Director 瞎编。

验证：

```bash
cargo test -p storyforge
cargo test --workspace
```

验收：

- active Campaign 下 `ctx.campaign_runtime` 非空。
- 无 active Campaign 下旧路径不变。

## ~~阶段 4：Director 改用 CharacterInstance~~ **（已合并到阶段 3，全部完成）**

原阶段 4 的 Director 改造（build_director_tail / get_character / prompt 更新）已在阶段 3 中一并完成，不再单独列为阶段。

## 阶段 4：Subagent 接收 Campaign 快照并做信息隔离

目标：子 Agent persona 来自实例化数据，只看到自己该看到的知识和变量。

改动文件：

- `crates/app-agent/src/runtime.rs`
- `crates/app-pipeline/src/lib.rs`
- `crates/domain/src/agent.rs` 如需扩展 provenance DTO

任务：

1. `spawn_subagents` 增加参数 `campaign_runtime: Option<Arc<CampaignRuntimeContext>>`。
2. 所有调用点传 `ctx.campaign_runtime.clone()`。
3. 子 Agent 构造：
   - 有 runtime 且能匹配 instance：system 使用 resolved persona/behavior。
   - tail 注入 scene、该 instance 可见 knowledge、该 instance variables、任务 brief。
   - 查不到 instance：warn 并 fallback 到旧 `context_package`。
4. ~~Provenance 记录~~ **（未完成，归入阶段 5）**：
   - subagent 使用的 `character_instance_id`
   - display name
   - fallback reason，如有。

验证：

```bash
cargo test -p storyforge-app-agent
cargo test -p storyforge-app-pipeline
cargo test --workspace
```

验收：

- A 角色私有知识不出现在 B 角色 subagent prompt。
- 同名角色通过 instance id 不混淆。
- reroll 仍能定位对应子 Agent。

## 阶段 5：Postprocess ID 归一、Provenance 收尾和写回收尾

目标：写回 CampaignStore 前，把名字、旧 character id、instance id 统一成 `CharacterInstance.id`。Provenance 记录 instance 级信息。

当前实现状态（2026-06-17）：

- 已实现：知识写入按 instance id/name 解析到已持久化 `CharacterInstance.id`；解析失败或不在 `present_chars` 时跳过。
- 已实现：变量写入按 instance id/name 查 persisted instance；不在 `present_chars` 时跳过。
- 已实现：task 状态更新会校验 `task.campaign_id == 当前 campaign_id`，避免跨档误写。
- 已实现：Provenance 记录 `character_instance_id`、`display_name`、`fallback_reason`。
- 已实现：`delete_character` 级联清理会使用持久化 `source_character_id`，并兼容旧数据和同会话 `tool_ctx`。
- 未实现：persona/behavior override 的后处理写回；当前 `PostProcessResult` 没有这类字段。

改动文件：

- `crates/tauri-app/src/lib.rs`
- `crates/domain/src/agent.rs` 如需细化 DTO
- `crates/domain/src/conversation.rs` SubagentSnapshot 扩展
- `crates/app-conversation/src/lib.rs` build_provenance 扩展
- `crates/app-agent/src/prompts/postprocess.rs` 如需调整输出约束

任务：

1. 提取 `resolve_postprocess_character_ref(campaign_runtime, input)`。
2. 变量更新：
   - instance 更新必须落到 instance id。
   - 不明确时记录 warn，不写错对象。
3. 知识更新：
   - 如果 knowledge 是角色可见，绑定 instance id。
   - 如果是全局知识，明确全局字段或保留 campaign 级条目。
4. 任务更新：
   - task status 更新必须校验 task 属于当前 campaign。
5. 真正使用 `present_chars`：
   - 至少用于 postprocess 提示和落盘校验。
6. Provenance 收尾：
   - SubagentSnapshot 扩展：`character_instance_id`、`display_name`、`fallback_reason`。
   - build_provenance 接收 CampaignRuntimeContext，从 Performance 中提取 instance 信息。

验证：

```bash
cargo test -p storyforge
cargo test --workspace
```

手动验证：

1. 导入单角色 ST 卡，未开 Campaign 写作正常。
2. 识别多角色卡，创建 Campaign，写一轮。
3. 检查 variables、knowledge、summary、tasks 均落到正确 campaign/instance。
4. 同名角色不串变量。

验收：

- 写作输出能改变下一轮 Campaign 上下文。
- postprocess 不再靠裸名字盲写。

## 阶段 6：临场角色（D34，单独阶段）（已完成落盘闭环，request_ad_hoc_character 经评估不必实现）

目标：用户在游玩中提到新角色时，导演能提出临场角色创建请求，由 Tauri 层落盘为 `CharacterInstance::temporary`，下一轮写作可见。

当前实现状态（2026-06-17 落盘闭环完成）：

- 已实现：`CampaignRuntimeContext::with_temporaries_for` 会为 director plan 中未匹配的 character_id 创建临时 instance，支持 persona/behavior override；同一批次内重复 unmatched character 会去重。
- 已实现：`CharacterInstance::temporary_with_overrides` 构造器，可传入 persona/behavior override。
- 已实现：`PipelineOrchestrator.pending_temporary_instances` 字段 + getter，`start_writing` / `regenerate` 在创建临时 instance 后存储到此字段，并在每轮开始时清空旧 pending。
- 已实现：Tauri 层 `persist_temporary_instances_to` helper，只在 pipeline 返回 `Ok` 后、`start_writing` / `regenerate` 的 postprocess 之前调用，把临时 instance 写入 `CampaignStore`。
- 已实现：去重和防串档逻辑——同一 campaign 内已存在同名 instance 时跳过创建，同一批次同名临时 instance 只写一次，`campaign_id` 不匹配的临时 instance 不写入。
- 已实现：临时 instance 落盘后，`persist_postprocess_outcome` 的知识/变量写回能通过 `find_instance_by_name_or_id` 找到它们，不再被跳过。
- 已实现：Director 的 `context_package.character_brief` 自动作为临时 instance 的 `persona_override` 注入。
- **经评估不必实现**：导演工具 `request_ad_hoc_character`。现有机制已完整覆盖：Director 在 `emit_plan` 中使用任意 `character_id`，pipeline 的 `with_temporaries_for` 自动为未匹配 ID 创建临时 instance，`context_package.character_brief` 作为 `persona_override` 注入，Tauri 层在 pipeline 成功后落盘。单独做工具需 Director 先调用再 emit_plan，增加一轮 LLM 交互，且需在 Director tool loop 中处理跨工具产出依赖——当前架构不支持。见下方评估说明。
- 已实现：前端展示临场角色（`is_temporary` 标记 + 临时 badge）和升格为常驻的 UI 流程（`CampaignPanel.vue` 中 `handlePromoteTemporary` 调用 `promoteTemporaryInstance`，带确认对话框、loading 状态、detail 刷新）。

改动文件：

- `crates/domain/src/campaign.rs`：+`temporary_with_overrides` 方法
- `crates/domain/src/campaign_runtime.rs`：`with_temporaries_for` 签名扩展为 `&[(String, Option<String>, Option<String>)]`，返回 `Vec<CharacterInstance>`
- `crates/app-pipeline/src/lib.rs`：+`pending_temporary_instances` 字段/getter，`start_writing`/`regenerate` 构建 character specs 并存储 temps
- `crates/app-agent/src/runtime.rs`：测试适配新签名
- `crates/tauri-app/src/lib.rs`：+`persist_temporary_instances_to` helper，`start_writing`/`regenerate` 调用点集成

关于 `request_ad_hoc_character` 的评估结论（2026-06-17）：

**判定：不必实现。** 现有 unmatched character_id 兜底机制已完整覆盖该工具的设计目标。

代码证据（关键路径）：
1. `app-pipeline/src/lib.rs:348-358`：从 `plan.subagent_tasks` 提取 `(character_id, character_brief as persona, None)` 构建 `char_specs`。
2. `domain/campaign_runtime.rs:94-130`：`with_temporaries_for(&char_specs)` 为未匹配 ID 创建 `CharacterInstance::temporary_with_overrides`，persona/behavior override 来自 specs。
3. `app-agent/src/runtime.rs:530-538`：`spawn_subagents` 通过 `find_instance_by_id_or_name` 匹配临时 instance，进入 Campaign 子 Agent 路径。
4. `app-agent/src/runtime.rs:666-686`：`build_campaign_subagent_system` 使用 `cr.resolved_persona_for(inst)` 获取 persona（override 优先于 definition），构造 system prompt。
5. `tauri-app/src/lib.rs:1523`：pipeline 成功后通过 `pending_temporary_instances()` 读取临时 instance，调用 `persist_temporary_instances_to` 落盘。

等价性分析：
- `request_ad_hoc_character` 的设计目标是让 Director 主动声明新角色（带 persona）。
- 现有路径：Director 在 `emit_plan` 的 `subagent_tasks` 中使用任意 `character_id` + `context_package.character_brief`（persona）→ pipeline 自动创建临时 instance → subagent 获得 persona → Tauri 落盘。
- 两条路径功能等价，现有路径零额外 LLM 开销，且不需修改 Director 工具循环的跨工具产出依赖。

如果单独做 `request_ad_hoc_character` 工具，需要 Director 先调用此工具再 emit_plan，增加一轮 LLM 交互，且需要在 Director 的 tool loop 中特殊处理产出顺序——当前架构不支持"工具产出影响 emit_plan 的输入"这种跨工具依赖。用现有 unmatched character_id + persona 来自 context_package 的方式，零额外 LLM 开销，且不改 Director 工具循环。

## 回滚策略

- 所有新字段必须是 `Option`，旧路径可直接 fallback。
- 每阶段一个 commit。
- 如果阶段 4/5 失败，可保留阶段 1/2/3 的 domain DTO 和 Director 改造，不影响旧写作。

## 禁止改动

- 禁止删除 `CharacterStore`。
- 禁止把 `CampaignStore` 移入 `domain`。
- 禁止让 `app-agent` 或 `app-pipeline` 依赖 `tauri-app`。
- 禁止一次性重写前端大布局。
- 禁止把 ST 卡导入格式改成 StoryForge 专有格式。

## 文件改动清单（按阶段）

| 阶段 | 文件 | 改动 |
|------|------|------|
| 1 | `domain/campaign.rs` | +resolved_persona/behavior/backstory/variable_schema 方法 + 测试 |
| 2 | `domain/campaign_runtime.rs` | +CampaignRuntimeContext 快照结构 + helper |
| 2 | `app-agent/tools.rs` | ToolContext + `campaign_runtime: Option<Arc<CampaignRuntimeContext>>` |
| 2 | `app-pipeline/lib.rs` | WritingContext + `campaign_runtime`，legacy() 补 None，空角色校验兼容 Campaign |
| 2 | `tauri-app/lib.rs` | fill_campaign_context 加载 campaign/instances/definitions/knowledge，组装快照并同步 tool_ctx |
| 3 | `app-pipeline/lib.rs` | build_director_tail 开档走实例 + 变量注入；DIRECTOR_SYSTEM_PROMPT 更新 |
| 3 | `app-agent/tools.rs` | get_character（导演）开档返回实例设定 |
| 4 | `app-agent/runtime.rs` | spawn_subagents 签名加 CampaignRuntimeContext，persona 走实例，信息隔离 |
| 4 | `app-pipeline/lib.rs` | spawn_subagents 调用点传 campaign_runtime |
| 4 | `app-agent/tools.rs` | 子 Agent get_character 限制为当前 instance |
| 5 | `tauri-app/lib.rs` | persist_postprocess_outcome 做 ID 归一化；present_chars 启用；task campaign 校验；CharacterInfo.source_character_id；delete_character id 修复 |
| 5 | `app-agent/prompts/postprocess.rs` | 明确输出可用角色名，但后端会解析成 instance id |
| 6 | `domain/campaign.rs` | +`temporary_with_overrides` 构造器（支持 persona/behavior override） |
| 6 | `domain/campaign_runtime.rs` | `with_temporaries_for` 签名扩展，返回 `Vec<CharacterInstance>`，支持 override 注入 |
| 6 | `app-pipeline/lib.rs` | +`pending_temporary_instances` 字段/getter，`start_writing`/`regenerate` 构建 specs 并存储 temps |
| 6 | `app-agent/runtime.rs` | 测试适配 `with_temporaries_for` 新签名 |
| 6 | `tauri-app/lib.rs` | +`persist_temporary_instances_to` helper，`start_writing`/`regenerate` 集成（postprocess 前落盘） |

## 工作量评估

| 阶段 | 工作量 | 风险 |
|------|--------|------|
| 1 domain 补全 | 小（1-2h） | 低 |
| 2 上下文扩展 | 中（3-4h） | 低（机械加字段） |
| 3 导演接通 | 中（3-4h） | 中（get_character 返回变了） |
| 4 子 Agent + 信息隔离 | 中-大（5-6h） | 中高（spawn_subagents 重构） |
| 5 后处理 ID 归一化 + 收尾 | 中（3-4h） | 中（身份解析要严格） |
| 6 临场角色 | 中（3-4h） | 中（工具协议 + 落盘事务） |

## 给执行 agent 的提示

1. **先读 §1.3 的断点清单**，对照行号确认现状（代码可能已演进，行号会漂移，以符号名为准）。
2. **每阶段做完先跑 `cargo test --workspace`** 再进下一阶段，不要攒着一起测。
3. **向后兼容是硬约束**：所有新字段必须 Option/默认空，每个消费点必须写「开档/未开档」分支。
4. **阶段 4 的信息隔离**是本计划的核心价值点，不要跳过。子 Agent 只看自己的 character_knowledge 是 StoryForge 区别于普通写作壳的关键。
5. **变量注入（阶段 3）**用现成的 `render_variables_for_injection()`，压在 tail 末尾，不要进 system。
6. **临场角色（阶段 6）**不要让 `ToolContext` 持有 `CampaignStore`；工具只产出请求，Tauri 层统一落盘。
7. **所有落盘前都做身份归一化**：LLM 可以说角色名，存储层必须写 instance id。
