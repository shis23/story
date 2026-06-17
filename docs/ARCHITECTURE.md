# StoryForge 架构说明

本文描述当前代码事实和目标架构边界。历史长文档已归档到 `docs/archive/2026-06-16-pre-rewrite/`。

## 架构原则

1. Campaign 是主线运行时。所有长期状态应落在 Campaign 域内，而不是散落在角色卡、对话文本或前端临时状态里。
2. ST 角色卡是兼容层。导入后保留原始 JSON，便于回写和兼容，但写作时不应长期依赖扁平 `Character`。
3. app 层不依赖 Tauri store。`CampaignStore` 目前位于 `crates/tauri-app/src/campaign_store.rs`，不能直接塞进 `app-agent` 或 `app-pipeline`。
4. Agent 契约优先结构化。LLM 可以用名字交流，但落盘、任务分配、变量更新和知识归属必须使用稳定 ID。
5. Meta Agent 是维护层。它负责解释、诊断、补丁建议和数据健康检查，不参与常规正文生成。

## 当前模块边界

```text
frontend
  -> tauri commands
crates/tauri-app
  -> local stores, command DTO, stream events
crates/app-pipeline
  -> writing orchestration
crates/app-agent
  -> agent runtime, tools, prompts, postprocess
crates/app-conversation
  -> conversation tree and variants
crates/app-meta
  -> meta conversations, MVU analysis, patches
crates/domain
  -> shared domain models
crates/infra-*
  -> LLM, import, vector, regex, plugin host
```

`crates/domain` 是纯领域模型层。`app-*` crate 可以依赖 domain 和 infra，但不应反向依赖 Tauri。Tauri 层负责把本地 store 组装成应用层需要的快照。

## 写作流水线

当前主流程位于 `crates/app-pipeline/src/lib.rs`：

1. `start_writing(intent, ctx)` 创建 session 和 seed。
2. Director 使用 `MessageLayout` 构造 system/history/tail，并调用工具生成 `Plan`。
3. `spawn_subagents` 按 `Plan.subagent_tasks` 并行运行子 Agent，当前并发上限为 4，超出排队。
4. Editor 合并子 Agent 表演，生成最终成文。
5. 成文写入 `ConversationStore`，并保存 `Provenance`。
6. 如果 `WritingContext.campaign_id` 存在，则后处理生成摘要、角色知识、变量更新和任务更新。

当前重要限制：

- `WritingContext.characters` 仍是 `Vec<Arc<Character>>`。
- Director 工具 `get_character` 从扁平 `Character` 查角色。
- `SubagentTask.character_id` 实际仍可能是角色名字符串。
- Campaign 后处理已接入，但 Campaign 还不是写作输入的真相源。
- **阶段 2 已完成**：`WritingContext`/`ToolContext` 已有 `campaign_runtime` 字段，`fill_campaign_context` 已组装快照，但 Director/Subagent 尚未消费它（阶段 3/4）。

## Campaign 目标流

目标架构应改为：

```text
Tauri CampaignStore
  -> 组装 CampaignRuntimeContext 快照
app-pipeline
  -> Director 使用 instances/variables/tasks/lore
app-agent
  -> tools 只读快照，不持有 store
Postprocess
  -> 输出 structured updates
Tauri
  -> 校验 ID，持久化到 CampaignStore
```

关键点：

- `CampaignRuntimeContext` 应是纯 domain/app DTO，而不是 `CampaignStore` 引用。
- 写作内部统一使用 `CharacterInstance.id`。
- 名字只用于 UI 展示和 LLM 输入输出，进入持久化前必须解析为 ID。
- 临场角色由请求显式创建 temporary instance，不应由后处理凭空造落盘角色。

当前执行计划见 `docs/PLAN-CAMPAIGN-MAINLINE.md`；角色统一历史计划已归档到 `docs/archive/2026-06-17-campaign-mainline-phase5/PLAN-CHARACTER-UNIFICATION.md`。

## Tauri 命令

当前 `crates/tauri-app/src/lib.rs` 暴露 87 个 Tauri 命令，覆盖：

- 角色卡导入、列表、删除。
- 世界书条目编辑。
- preset/module/profile 管理。
- 插件管理。
- 写作、取消、重 roll。
- 连接配置和模型列表。
- 对话树 variant 编辑。
- 日志查询、清理、导出。
- 向量 embedder 配置。
- Meta Agent 会话、补丁、MVU 分析。
- Campaign、instance、variables、knowledge、tasks、round summaries。

只有 `start_writing` 和 `regenerate` 通过 Tauri Channel 流式推送 PipelineEvent；其他命令基本是同步请求/响应。

## 前端结构

前端位于 `frontend/src`，核心组件包括：

- `App.vue`：应用主状态与页面组织。
- `tauri-api.js`：Tauri 命令包装。
- `CampaignPanel.vue`：Campaign、实例、变量、任务和知识展示。
- `PipelinePanel.vue`：流水线状态与 Agent 输出。
- `MetaPanel.vue`：Meta Agent 会话和 MVU 分析。
- `CharacterList.vue` / `CharacterDetail.vue`：角色卡导入和查看。
- `PresetPanel.vue` / `PluginPanel.vue`：预设、模块和插件入口。

前端目前已经有 Campaign 面板，但写作入口仍需要进一步围绕 active campaign 重构。

## 存储现状

本地存储由 Tauri 层管理，主要是 JSON 文件。关键 store：

- `CharacterStore`：扁平 ST 角色卡。
- `CampaignStore`：Campaign、CharacterCard、CharacterDefinition、CharacterInstance、变量、知识、任务、摘要。
- `ConversationStore`：对话树和 variants。
- `PresetStore` / `ModuleStore` / `ConnectionStore`。

架构方向不是删除 ST 兼容存储，而是把它降级为导入/导出和 fallback 来源。

## 当前最高优先级债务

1. Campaign 不是写作输入真相源。
2. ~~`CharacterInstance.resolved_persona()` 当前只返回 override，没有 fallback 到 `CharacterDefinition`。~~ **阶段 1 已修复**：`resolved_persona(definition)` / `resolved_behavior(definition)` 已支持 override → definition → None 回退。
3. Director/Subagent 仍围绕角色名和扁平 `Character` 运作。
4. 后处理输出需要更严格的 ID 归一化和校验。
5. Meta Agent 已有入口，但还没有成为 Campaign 数据健康和写作链路解释的核心维护层。
