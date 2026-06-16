# 文档与代码对齐审计

> 状态：2026-06-16
> 范围：核对 README、ROADMAP、HANDOFF、ARCHITECTURE-AUDIT、PLAN-* 与当前源码的一致性。
> 本次只审计和修正文档，不改业务代码。

## 结论

当前文档的大方向来自代码现状，核心架构判断成立：

- Campaign 数据模型已经存在，但写作流水线仍主要消费扁平 `Character`。
- `CampaignStore` 位于 `tauri-app`，下层 `app-agent` / `app-pipeline` 不应直接依赖它。
- 通过纯 domain DTO `CampaignRuntimeContext` 下传 Campaign 运行态，是符合当前 crate 分层的改造路径。
- Meta、MVU、Android、前端计划多数是基于已有雏形的后续计划，不是当前已完成能力。

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
- `crates/tauri-app/src/lib.rs::fill_campaign_context` 只填充 `campaign_id`、`turn`、`pending_tasks`、`story_clock`。
- `crates/app-pipeline/src/lib.rs::WritingContext` 当前字段仍是 `characters/world_info/conversation_id/campaign_id/turn/pending_tasks/story_clock/profile/modules/recent_messages`，没有 `CampaignRuntimeContext`。
- `crates/app-pipeline/src/lib.rs::build_director_tail` 仍基于 `ctx.characters` 渲染可用角色，并注入 pending tasks。
- `crates/app-agent/src/tools.rs::ToolContext` 当前只有 `characters/world_info/vector_store/archived_summaries`，没有 Campaign runtime。
- `crates/app-agent/src/runtime.rs::spawn_subagents` 当前只接收 `Vec<SubagentTask>`、runtime、registry 等，不接收 Campaign 快照。
- `crates/tauri-app/src/lib.rs::persist_postprocess_outcome` 已写回 Campaign summary、knowledge、variables、tasks，并在知识/变量写入时使用 `store.list_instances(camp_id)` 做部分角色名到实例 ID 的匹配。

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
- `CharacterInstance::resolved_persona()` / `resolved_behavior()` 当前已存在，但只返回 override，不接收 `CharacterDefinition` fallback。
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

- `CampaignRuntimeContext`
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
