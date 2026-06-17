# 文档与代码对齐审计

> 状态：2026-06-17
> 范围：核对 README、ROADMAP、HANDOFF、ARCHITECTURE-AUDIT、PLAN-* 与当前源码的一致性。
> 本次只审计和修正文档，不改业务代码。

## 结论

当前文档的大方向来自代码现状，核心架构判断成立；但 Campaign 主链路已经完成到 Phase 5，不能再按早期“写作流水线仍主要消费扁平 Character”的状态执行：

- Campaign 数据模型已经存在；开 Campaign 时写作流水线已通过 `CampaignRuntimeContext` 消费 instances / definitions / knowledge，未开 Campaign 时继续 fallback 到扁平 `Character`。
- `CampaignStore` 位于 `tauri-app`，下层 `app-agent` / `app-pipeline` 不应直接依赖它。
- 通过纯 domain DTO `CampaignRuntimeContext` 下传 Campaign 运行态，是符合当前 crate 分层的改造路径。
- Meta、MVU、Android、前端计划多数是基于已有雏形的后续计划，不是当前已完成能力。
- 临场角色后端已完成落盘闭环：临时 instance 会在成功写作结果的 postprocess 前写入 CampaignStore，并可被下一轮读取；前端展示和“升格为常驻”UI 仍是未来工作。

文档可以继续作为后续执行依据，但执行前应注意本文列出的“规划性内容”和“缺口”。

## 已核对为代码事实

### Workspace 和命令数量

- `Cargo.toml` 当前 workspace members 为 14 个 crate。
- `crates/tauri-app/src/lib.rs` 的 `tauri::generate_handler!` 当前注册 87 个 Tauri command。
- README 中“Rust workspace，14 个 crate”和“87 个命令”与当前代码一致。

### Campaign 写作主链路

已核对文件：

- `frontend/src/App.vue`
- `frontend/src/tauri-api.js`
- `crates/tauri-app/src/lib.rs`
- `crates/app-pipeline/src/lib.rs`
- `crates/app-agent/src/tools.rs`
- `crates/app-agent/src/runtime.rs`

代码事实：

- `frontend/src/App.vue::startWriting` 仍调用 `apiStartWriting(intent, activeChar.value?.id, ..., currentConversationId.value)`。
- `frontend/src/tauri-api.js::startWriting` 调用 Tauri command `start_writing`。
- `crates/tauri-app/src/lib.rs::start_writing` 存在，并从 `snapshot_tool_ctx()` 构造写作上下文。
- `crates/tauri-app/src/lib.rs::fill_campaign_context` 填充 `campaign_id`、`turn`、`pending_tasks`、`story_clock`，**阶段 2 已扩展**：开头先清空旧 runtime 防 stale，然后从 CampaignStore 加载 instances、definitions、knowledge，组装 `Arc<CampaignRuntimeContext>` 写入 `ctx.campaign_runtime` 并同步到 `tool_ctx`。
- `crates/app-pipeline/src/lib.rs::WritingContext` **阶段 2 已新增** `campaign_runtime: Option<Arc<CampaignRuntimeContext>>`。字段列表为 `characters/world_info/conversation_id/campaign_id/turn/pending_tasks/story_clock/profile/modules/recent_messages/campaign_runtime`。
- `crates/app-pipeline/src/lib.rs::build_director_tail` **阶段 3 已改造**：有 `campaign_runtime` 时从 instances 渲染（含 id/role_type/persona 摘要 + instance variables），campaign 全局变量注入 volatile tail，UTF-8 安全截断（`truncate_chars` 按 char 而非 byte）；无时退回旧的扁平 Character 名称列表。pending tasks 注入不变。
- `crates/app-pipeline/src/lib.rs::has_available_characters` **阶段 3 新增**：兼容 Campaign（instances 非空）和旧路径（characters 非空），`start_writing` 和 `regenerate` 共用。错误文案兼容两条路径。
- `crates/app-pipeline/src/lib.rs::DIRECTOR_SYSTEM_PROMPT` **阶段 3 已更新**：character_id 规则区分 Campaign 实例（用 instance_id）和旧路径（用角色名）。
- `crates/app-agent/src/tools.rs::ToolContext` **阶段 2 已新增** `campaign_runtime: Option<Arc<CampaignRuntimeContext>>`。字段列表为 `characters/world_info/vector_store/archived_summaries/campaign_runtime`。
- `crates/app-agent/src/tools.rs` 导演 `get_character` **阶段 3 已改造**：有 `campaign_runtime` 时优先查实例（返回 id/instance_id/definition_id/role_type/persona/behavior/variables/is_temporary），查不到 fallback 到旧扁平 Character。
- `crates/app-agent/src/tools.rs` 子 Agent `get_character` **阶段 4 已改造**：有 `current_character_instance_id` 时只返回该 instance 的数据，不允许查其他角色（信息隔离）。无时退回旧扁平 Character。
- `crates/app-agent/src/tools.rs::ToolContext` **阶段 4 新增** `current_character_instance_id: Option<Id>`：子 Agent 绑定的 instance id，用于 get_character 信息隔离。导演/编剧/无 Campaign 时为 None。
- `crates/app-agent/src/runtime.rs::spawn_subagents` **阶段 4 已改造**：接收 `campaign_runtime: Option<Arc<CampaignRuntimeContext>>`，按 character_id 匹配 instance（id 优先，name 兜底），使用 resolved persona/behavior 构造 system，注入该 instance 的 knowledge（信息隔离）和 variables，为每个子 Agent 构造独立 ToolContext（绑定 `current_character_instance_id`）。未匹配时 fallback 到旧 context_package 并 warn。无 campaign_runtime 时走旧路径。
- `crates/tauri-app/src/lib.rs::persist_postprocess_outcome` **阶段 5 已改造**：知识/变量写入先解析到已持久化 `CharacterInstance.id`，`present_chars` 真正用于落盘校验；不出场角色的知识不写入，非在场 instance 的变量写入被跳过并 warn；task 状态更新会校验 task 属于当前 campaign。
- `crates/domain/src/conversation.rs::SubagentSnapshot` **阶段 5 已扩展**：新增 `character_instance_id: Option<String>`、`display_name: Option<String>`、`fallback_reason: Option<String>`（serde 兼容旧数据）。
- `crates/app-conversation/src/lib.rs::build_provenance_with_campaign` **阶段 5 新增**：接收 `CampaignRuntimeContext`，从 Performance 中提取 instance 信息填充 SubagentSnapshot。旧 `build_provenance` 保留向后兼容。
- `crates/tauri-app/src/lib.rs::CharacterInfo` **阶段 5 已扩展**：新导入卡保存 `source_character_id: Option<String>`，启动恢复时可保留 domain `Character.id`；旧数据 fallback 到 `StoredCharacter.id`。
- `crates/tauri-app/src/lib.rs::delete_character` **阶段 5 已修复**：级联删除时同时尝试 `StoredCharacter.id`、持久化 `source_character_id`、同会话 `tool_ctx` domain id，避免旧代码只用 StoredCharacter.id 查 card 导致静默失败。
- `crates/app-agent/src/runtime.rs::build_campaign_subagent_system` / `build_campaign_subagent_volatile` **阶段 4 cleanup 新增**：纯函数，测试可直接断言 prompt 内容（persona/behavior/knowledge 隔离/variables）。
- `crates/domain/src/campaign.rs::CharacterInstance::temporary_with_overrides` **阶段 6 已完成**：创建临时 instance 时可传入 persona/behavior override。
- `crates/domain/src/campaign_runtime.rs::CampaignRuntimeContext::with_temporaries_for` **阶段 6 已完成**：为未匹配的 character_id 创建临时 CharacterInstance（`is_temporary=true`，支持 persona/behavior override），返回更新后的 context 和临时 instance 列表供调用者持久化；同一批次内重复 unmatched character 会去重。
- `crates/app-pipeline/src/lib.rs::PipelineOrchestrator::pending_temporary_instances` **阶段 6 已完成**：存储本轮创建的临时 instance，Tauri 层通过 getter 读取后落盘；`start_writing` / `regenerate` 开始时会清空旧 pending，避免失败或重试污染下一轮。
- `crates/app-pipeline/src/lib.rs::start_writing` / `regenerate` **阶段 6 已完成**：从 Director 的 `context_package.character_brief` 提取 persona 注入临时 instance，存储到 `pending_temporary_instances`。
- `crates/tauri-app/src/lib.rs::persist_temporary_instances_to` **阶段 6 已完成**：只在 pipeline 返回 `Ok` 后、postprocess 之前把临时 instance 写入 CampaignStore；会跳过同 campaign 已存在同名 instance、同批重复临时 instance，以及 `campaign_id` 不匹配的临时 instance。落盘后 postprocess 知识/变量写回不再被跳过。
- `request_ad_hoc_character` 工具**未实现**：当前用 unmatched character_id 自动触发，Director 的 character_brief 作为 persona 注入。
- `frontend/src/components/CampaignPanel.vue` **临时 instance UI 已实现**：展示 `is_temporary` 标记，提供升格为常驻按钮（带确认对话框、loading 状态、detail 刷新），`promoteTemporaryInstance` 从 `tauri-api.js` 导入。

因此 `ARCHITECTURE-AUDIT.md` 和 `PLAN-CAMPAIGN-MAINLINE.md` 的主线判断与代码一致。

### Domain 模型

已核对文件：

- `crates/domain/src/campaign.rs`
- `crates/domain/src/character.rs`
- `crates/domain/src/character_knowledge.rs`
- `crates/domain/src/variables.rs`
- `crates/domain/src/story_task.rs`
- `crates/domain/src/mvu_translation.rs`

代码事实：

- `Campaign`、`CharacterInstance`、`CharacterDefinition`、`CharacterKnowledgeEntry`、变量模型、任务模型、`MvuTranslation` 均存在。
- `CharacterInstance` 当前只有 `persona_override` 和 `behavior_override`，没有 `backstory_override` 或 `variable_schema` 字段。
- `CharacterInstance::resolved_persona(definition)` / `resolved_behavior(definition)` 接收 `Option<&CharacterDefinition>`，override 优先，无 override 时 fallback 到 definition（阶段 1 已实现）。
- `CharacterDefinition` 持有 `persona_prompt`、`behavior_rules`、`base_backstory`、`variable_schema`。

本次已修正 `PLAN-CAMPAIGN-MAINLINE.md`：`resolved_backstory` / `resolved_variable_schema` 不应被写成当前实例天然字段，建议作为 `CampaignRuntimeContext` helper，除非先显式新增 instance override 字段。

### Meta Agent 和 MVU

已核对文件：

- `crates/app-meta/src/lib.rs`
- `crates/app-meta/src/meta_conversation.rs`
- `crates/app-meta/src/mvu_import.rs`
- `crates/app-meta/src/prompts/meta_agent.rs`
- `crates/app-meta/src/prompts/mvu_analyzer.rs`
- `crates/tauri-app/src/lib.rs`
- `frontend/src/components/MetaPanel.vue`

代码事实：

- `MetaSession`、`PatchStore`、Meta runtime tools 存在。
- `meta_accept_patch`、`meta_analyze_mvu_card`、`meta_list_mvu_translations` 等 Tauri commands 存在。
- `mvu_import::analyze_mvu_card` 会产出 `MvuTranslation`，解析失败时走 `pure_data_fallback`。
- `frontend/src/components/MetaPanel.vue` 已展示 Meta 聊天、tool result、pending patches、MVU translations。

因此 `PLAN-META-AGENT.md` 和 `PLAN-PLUGIN-MVU.md` 的“当前事实”基本准确；其中 health check、generation explanation、typed patch preview、schema apply、runtime fallback 是后续计划，不是当前已完成能力。

### 前端工作台

已核对文件：

- `frontend/src/App.vue`
- `frontend/src/components/CampaignPanel.vue`
- `frontend/src/components/PipelinePanel.vue`
- `frontend/src/components/MetaPanel.vue`
- `frontend/src/components/MvuStatusBar.vue`
- `frontend/src/components/CharacterDetail.vue`
- `frontend/src/tauri-api.js`

代码事实：

- `App.vue` 已有 `activeCampaign`，mounted 时调用 `getActiveCampaign()`。
- 主写作入口仍传 `activeChar.value?.id`。
- `CampaignPanel.vue` 已有 Campaign、instances、variables、knowledge、tasks、summaries 相关入口。
- `MvuStatusBar.vue` 已存在，并在 `CharacterDetail.vue` 中使用。

因此 `PLAN-FRONTEND-WORKBENCH.md` 的主要判断成立。

### Android/Tauri

已核对文件：

- `crates/tauri-app/tauri.conf.json`
- `crates/tauri-app/capabilities/default.json`
- `crates/tauri-app/gen/android/app/src/main/AndroidManifest.xml`
- `crates/tauri-app/gen/android/app/src/main/java/com/storyforge/app/MainActivity.kt`

代码事实：

- Tauri v2 配置文件存在。
- capability 当前包含 `core:default`、`fs:default`、`dialog:default`。
- Android Manifest 包含 `INTERNET`、`MainActivity`、`FileProvider`。
- `MainActivity.kt` 调用 `enableEdgeToEdge()`。

因此 `PLAN-ANDROID.md` 的当前事实准确。

## 已修正文档问题

1. `docs/PLAN-CAMPAIGN-MAINLINE.md`
   - 原文容易让执行者以为 `CharacterInstance` 已有或应该直接承载 backstory/schema。
   - 已改为：persona/behavior 可在 instance method 中做 override + definition fallback；backstory/schema 优先作为 runtime/helper 读取 `CharacterDefinition`，除非显式新增 instance override 字段。

2. `docs/ROADMAP.md`
   - 原文把 Phase 5 “ST 兼容和导入/导出”的详细计划指向 `PLAN-PLUGIN-MVU.md`，但该计划只覆盖 MVU/plugin 方向，不覆盖完整 ST 导入/导出。
   - 已改为：`PLAN-PLUGIN-MVU.md` 只覆盖 MVU 状态栏、schema preview、JS fallback；进入 Phase 5 前应补 `docs/PLAN-ST-IMPORT-EXPORT.md`。

## 仍需补齐的文档缺口

### 1. ST 导入/导出专项计划缺失

`ROADMAP.md` Phase 5 包含：

- ST V2/V3 导入保真范围。
- raw JSON 和 extensions 保留策略。
- StoryForge Campaign 导出格式。
- 是否支持导出回 ST 卡或 Lorebook。

当前没有对应 `PLAN-ST-IMPORT-EXPORT.md`。这是后续文档层面的最大缺口。

### 2. Release checklist 和 user guide 只是未来产物

`PLAN-POST-MAINLINE.md` 提到：

- `docs/RELEASE-CHECKLIST.md`
- `docs/USER-GUIDE.md`

这两个文件当前不存在，且在计划中标为新增/如需要新增。执行者不应把它们当成当前文档。

### 3. 架构计划中的新类型尚未实现

以下名称是推荐目标，不是当前代码事实：

- ~~`CampaignRuntimeContext`~~ **阶段 2 已完成**：`crates/domain/src/campaign_runtime.rs` 包含 DTO + helpers，已接入 `WritingContext`/`ToolContext`，`fill_campaign_context` 已组装快照。
- `meta_explain_generation`
- `meta_preview_mvu_schema`
- `propose_apply_mvu_schema`
- `startCampaignWriting`
- `PLAN-ST-IMPORT-EXPORT.md`

执行时应按计划新增或替换，不要在当前代码中搜索不到就判定任务失败。

## 对小模型执行的补充规则

- 先读本文，再读对应 `PLAN-*.md`。
- 把“当前事实”与“任务/目标”分开理解。
- 如果计划里提到的文件存在但符号不存在，先判断它是不是计划要求新增的符号。
- 如果计划要求修改 `CharacterInstance`，必须先看当前结构字段，不能凭文档臆造已有字段。
- Phase 5 ST 导入/导出开始前，先补 `PLAN-ST-IMPORT-EXPORT.md`，不要用 `PLAN-PLUGIN-MVU.md` 代替。
- 执行代码改动后要回写本审计报告或对应计划的状态，避免文档再次漂移。
