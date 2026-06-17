# StoryForge 架构审计

> 状态：2026-06-17 更新版
> 范围：评估现有架构与后续重构顺序，并记录 Campaign 主线 Phase 1-6 已落地后的当前限制。

## 结论

现有代码不需要推倒重写，但需要一次中等规模的主链路重构。

最大问题不是模块拆分错误，而是“运行时权威数据源”分裂。这个问题已经通过 `CampaignRuntimeContext` 在写作主链路上修到 Phase 6：开 Campaign 时 Director、Subagent、Postprocess、Provenance 已优先消费 Campaign instances；临场角色会在成功写作结果的 postprocess 前落盘；未开 Campaign 时继续 fallback 到扁平 `CharacterStore` / `ToolContext.characters`。

剩余的大缺口是产品闭环而不是 crate 分层：前端仍围绕 active character，临场角色后端已可落盘和跨轮读取，但前端尚未展示临时角色，也没有接入“升格为常驻”操作；Meta/插件/Android 还不应继续绕旧链路扩展。

推荐方向：

1. 先把 Campaign 变成写作主线。
2. 再做 Meta Agent 的诊断、解释和修复能力。
3. 前端围绕 Campaign 工作流重排。
4. Android 做专项验证和少量适配。
5. 插件/MVU 先收敛到“变量 schema + 状态栏 + 兜底执行”，不要先做通用插件平台。

## 当前架构地图

### 后端 crate 分工

| 层 | 现状 | 判断 |
| --- | --- | --- |
| `crates/domain` | 领域模型较完整，含 `Campaign`、`CharacterInstance`、变量、任务、知识、MVU translation。 | 可以继续承载共享 DTO。 |
| `crates/app-agent` | Agent runtime、工具循环、提示词、后处理。`ToolContext` 仍以扁平角色卡为主。 | 应消费只读 Campaign 快照，不应依赖 Tauri store。 |
| `crates/app-pipeline` | Director -> Subagents -> Editor -> Postprocess 主流水线。`WritingContext` 仍以 `characters: Vec<Character>` 为主。 | 是 Campaign 主线重构核心。 |
| `crates/app-meta` | Meta 对话、PatchStore、MVU 分析已经存在。 | 当前偏“配置诊断”，还不是 Campaign 维护层。 |
| `crates/infra-plugin-host` | 插件 manifest/registry 已有；MVU runtime 是 stub。 | 不应先扩大通用插件能力。 |
| `crates/tauri-app` | AppState、store、Tauri commands、前后端桥接。`CampaignStore` 定义在这里。 | 适合装配 Campaign 快照，但不适合让下层反向依赖。 |
| `frontend` | Vue 单页应用，主输入仍围绕 active character。 | 需要改成 Campaign-first 工作台。 |

### 当前写作调用链

```text
frontend/App.vue startWriting()
  -> tauri-api.js startWriting(intent, activeChar?.id, conversationId)
  -> tauri-app::start_writing
       snapshot_tool_ctx()
       WritingContext { characters, world_info, campaign_id: None, ... }
       fill_agent_profile_context()
       fill_campaign_context()
          阶段 2 已扩展：清空旧 runtime → 加载 campaign/instances/definitions/knowledge → 组装 CampaignRuntimeContext 快照 → 写入 ctx.campaign_runtime + tool_ctx
       PipelineOrchestrator::start_writing()
          has_available_characters() 校验（兼容 Campaign instances 和旧 characters）
          Director 看 ctx.characters + world_info + tasks（阶段 3 已改造：Director tail 消费 campaign_runtime，含 id/role/persona/variables）
          spawn_subagents(plan.subagent_tasks)
          Editor 合并
          run_postprocess()
       阶段 6：pipeline Ok 后、postprocess 持久化前，Tauri 持久化本轮 pending temporary instances
       persist_postprocess_outcome()
          写 summary / knowledge / variables / tasks
          阶段 5 已改造：knowledge/variables 解析到 persisted instance id，present_chars 校验，task campaign 校验
```

关键断点：

- `crates/tauri-app/src/lib.rs::fill_campaign_context` **阶段 2 已修复**：加载 instances、definitions、knowledge，组装 `CampaignRuntimeContext` 快照。开头先清空旧 runtime 防 stale。
- `crates/app-pipeline/src/lib.rs::WritingContext` **阶段 2 已修复**：新增 `campaign_runtime: Option<Arc<CampaignRuntimeContext>>`。
- `crates/app-pipeline/src/lib.rs::build_director_tail` **阶段 3 已改造**：有 `campaign_runtime` 时从 instances 渲染（含 id/role_type/persona 摘要 + instance variables），campaign 全局变量注入 volatile tail，UTF-8 安全截断；无时退回旧逻辑。
- `crates/app-pipeline/src/lib.rs::has_available_characters` **阶段 3 新增**：兼容 Campaign（instances 非空）和旧路径（characters 非空），`start_writing` 和 `regenerate` 共用。
- `crates/app-agent/src/tools.rs::ToolContext` **阶段 2 已修复**：新增 `campaign_runtime: Option<Arc<CampaignRuntimeContext>>`。
- `crates/app-agent/src/tools.rs` 导演 `get_character` **阶段 3 已改造**：有 `campaign_runtime` 时优先查实例（返回 id/definition/persona/behavior/variables），查不到 fallback 到旧扁平 Character。
- `crates/app-agent/src/runtime.rs::spawn_subagents` **阶段 4 已改造**：接收 `campaign_runtime`，按 character_id 匹配 instance，用 resolved persona/behavior 构造 system，注入该 instance 的 knowledge（信息隔离）和 variables，每个子 Agent 有独立 ToolContext（绑定 `current_character_instance_id`）。未匹配时 fallback 到 context_package。
- `crates/tauri-app/src/lib.rs::persist_postprocess_outcome` **阶段 5 已改造**：知识/变量写入先解析到 persisted `CharacterInstance.id`，`present_chars` 真正用于校验；不出场角色的知识不写入，非在场 instance 的变量写入被跳过；task 状态更新校验 task 属于当前 campaign。
- `crates/domain/src/conversation.rs::SubagentSnapshot` **阶段 5 已扩展**：新增 `character_instance_id`、`display_name`、`fallback_reason`。
- `crates/tauri-app/src/lib.rs::CharacterInfo` **阶段 5 已扩展**：新导入卡保存 `source_character_id`，启动恢复时保留 domain `Character.id`。
- `crates/tauri-app/src/lib.rs::delete_character` **阶段 5 已修复**：级联删除时同时尝试 `StoredCharacter.id`、持久化 `source_character_id`、同会话 `tool_ctx` domain id。
- `crates/domain/src/campaign.rs::CharacterInstance::temporary_with_overrides` **阶段 6 已完成**：创建临时 instance 时可传入 persona/behavior override。
- `crates/domain/src/campaign_runtime.rs::CampaignRuntimeContext::with_temporaries_for` **阶段 6 已完成**：为未匹配角色创建临时 instance（支持 persona/behavior override），返回 instance 列表供调用者持久化；同批重复 unmatched character 会去重。
- `crates/app-pipeline/src/lib.rs::PipelineOrchestrator::pending_temporary_instances` **阶段 6 已完成**：存储本轮临时 instance，Tauri 层 getter 读取后落盘；`start_writing` / `regenerate` 开始时清空旧 pending，避免状态污染。
- `crates/tauri-app/src/lib.rs::persist_temporary_instances_to` **阶段 6 已完成**：只在 pipeline `Ok` 路径、postprocess 前把临时 instance 写入 CampaignStore；会跳过同名重复、同批重复和 `campaign_id` 不匹配的临时 instance，落盘后 postprocess 知识/变量写回不再被跳过。

### AgentProfileConfig 运行时闭环（已完成）

AgentProfileConfig 已从"存储但未消费"推进到完整运行时闭环：

- **Domain DTO**：`crates/domain/src/agent_profile_config.rs` 定义 `AgentRunConfig`（`model_override` / `max_tool_rounds` / `tool_whitelist`，全 `Option`）和 `AgentProfileConfig`（`agent_configs: HashMap<AgentRole, AgentRunConfig>` / `max_concurrent_subagents` / `enable_postprocess` / `enable_summarizer` / `source` / `config_version`）。内置默认 `builtin-default-agent-v1` 始终可用、不可删除。`effective_max_concurrent_subagents()` 把 `0` clamp 到 `1` 避免 Semaphore 死锁。`run_config_for` 支持 Subagent 通配符回退（先精确 `Subagent(id)`，再 `Subagent("*")`）。
- **Tauri 层持久化**：`crates/tauri-app/src/module_store.rs::AgentProfileConfigStore` 持久化到 `agent_profile_configs.json` + `active_agent_profile_config.json`。6 个 Tauri command：`list_agent_profile_configs` / `get_agent_profile_config` / `get_active_agent_profile_config` / `save_agent_profile_config` / `delete_agent_profile_config` / `set_active_agent_profile_config`。`fill_agent_profile_context` 在 `start_writing` / `regenerate` 时加载活跃配置到 `WritingContext.agent_profile_config`。
- **运行时消费**：
  - Director/Editor `make_*_config` 从 profile 读取 `model_override` 和 `max_tool_rounds`；无 config 时保持当前硬编码默认值。
  - `spawn_subagents` 接收 `max_concurrent_subagents` 和 `agent_profile_config`；子 Agent 按 profile 覆盖 model 和 max_tool_rounds；每个子 Agent 的 registry 按 profile `tool_whitelist` 过滤。
  - `tool_whitelist` 通过 `ToolRegistry::retain` / `filter_registry_by_whitelist` 在 Director/Subagent/PostProcessor 注册工具后过滤（`None` = 默认工具集，`Some([])` = 禁用全部，`Some(list)` = 只保留列表中的；未知工具名记 warning 后忽略，不 panic）。被禁用的工具 dispatch 返回 `ToolError::NotFound`，whitelist 不可绕过。
  - `run_postprocess_pipeline` 接收 `enable_postprocess` / `enable_summarizer` 参数；某开关 `false` 时跳过对应 LLM 调用（返回 `None`）；两者都 `false` 时 `run_postprocess` 不发 `PostProcessStarted`，改发 `PipelineEvent::PostProcessSkipped { reason }`（区别于真失败的 `PostProcessFailed`）；单关 summarizer 时不发 `SummaryDone`；无 config 时全开（向后兼容）。
- **新 PipelineEvent 变体**：`crates/domain/src/agent.rs` 新增 `PostProcessSkipped { reason: String }`（serde 兼容），Tauri 序列化为 `postprocess_skipped`。
- **前端管理 UI**：`frontend/src/components/AgentProfileManager.vue`（power 模式下，位于 `AgentConfigCard` 之后）支持列/切活跃/复制/删除/编辑/保存。可编辑 name/description/max_concurrent_subagents/enable_postprocess/enable_summarizer 以及 Director/Editor/Subagent:*/Summarizer/PostProcessor 各自的 model_override/max_tool_rounds/tool_whitelist（逗号分隔）。内置默认只读不可删除。注意：`AgentConfigCard.vue` 是 PromptProfile 模块选择器（另一套体系），不是 AgentProfileConfig 编辑器。
- **前端 API**：`frontend/src/tauri-api.js` 已实现 6 个 wrapper（`listAgentProfileConfigs` / `getAgentProfileConfig` / `getActiveAgentProfileConfig` / `saveAgentProfileConfig` / `deleteAgentProfileConfig` / `setActiveAgentProfileConfig`）。

## 是否要大修

需要大修“主链路”，不需要大修“整个项目”。

不建议做的事：

- 不要把 `CampaignStore` 直接塞进 `app-agent` 或 `app-pipeline`。这会让下层 crate 反向依赖 Tauri，后续 Android、测试、CLI 都会变差。
- 不要先重写所有 store。当前 JSON store 虽然不优雅，但不是眼前最大瓶颈。
- 不要先做通用插件市场、复杂 WebView sandbox、Meta 自动修复所有问题。它们都依赖 Campaign 主线稳定。
- 不要把 SillyTavern 卡兼容和 StoryForge 内部运行态混成同一个模型。ST 卡应是导入源，Campaign 才是运行时真相源。

建议做的事：

- 在 `domain` 增加纯数据 `CampaignRuntimeContext` 快照。
- Tauri 层从 `CampaignStore` 装配快照。
- `WritingContext` / `ToolContext` 消费该快照。
- Director、Subagent、Postprocess、Provenance 逐步从“角色名字符串”迁移到 `CharacterInstance.id`。
- 保留未开 Campaign 时的扁平 Character fallback，保证现有 ST 单卡写作不崩。

## 主要风险

### 1. 身份语义混乱

当前 `SubagentTask.character_id` 实际上经常是角色名。后续 Campaign 路径中它应优先承载 `CharacterInstance.id`，展示层再映射成名称。

风险：同名角色、临场角色、导入多角色卡后，变量和知识写串。

### 2. Agent 上下文由 LLM 补全

子 Agent 的 persona 目前依赖 Director 产出的 `context_package.character_brief`。这意味着角色设定可能被 LLM 改写或漏写。

风险：角色一致性不可控，Meta 也无法解释“为什么这个子 Agent 看到了这些信息”。

### 3. 后处理已经写 Campaign，但输入不完整

Postprocess 会写 summary、knowledge、variables、tasks，但写作阶段没真正注入 Campaign variables/knowledge，导致“写回闭环”不完整。

风险：状态变了，但下一轮不一定看见；或者看见的是名字猜测而非稳定 ID。

### 4. 前端入口仍是 active character

`frontend/src/App.vue` 已加载 `activeCampaign`，但主写作调用仍是 `apiStartWriting(intent, activeChar.value?.id, ...)`。

风险：用户以为在玩 Campaign，系统实际上按当前角色卡写。

### 5. 插件和 MVU 尚未形成闭环

`infra-plugin-host` 的 registry 已有，`mvu_runtime.rs` 仍是 `StubMvuRuntime`。`app-meta` 能分析 MVU，但分析结果还没有稳定进入 Campaign variable schema 和 UI 状态栏运行。

风险：继续做插件 UI 会变成“能装 manifest，但不能影响主体验”。

## 推荐重构顺序

1. `PLAN-CAMPAIGN-MAINLINE.md`
   - 先打通 CampaignRuntimeContext、实例身份、变量/知识注入、后处理 ID 归一。

2. `PLAN-FRONTEND-WORKBENCH.md`
   - 后端主线稳定后，前端首屏和写作入口改成 Campaign-first。

3. `PLAN-META-AGENT.md`
   - Meta 从“聊天式配置助手”升级为 Campaign 健康检查、解释、修复入口。

4. `PLAN-PLUGIN-MVU.md`
   - MVU 先接入变量 schema / 状态栏 / 兜底 runtime，再考虑通用插件。

5. `PLAN-ANDROID.md`
   - 以主流程验证为中心，处理文件导入、长文本流式、日志导出、权限和性能。

## 给小模型执行的规则

- 每次只执行一个计划文件里的一个阶段。
- 每阶段必须先读“当前事实”和“禁止改动”。
- 除非计划明确要求，不允许跨 crate 做顺手重构。
- 代码阶段必须跑计划指定测试；跑不了要写明原因。
- 改动完成后更新相关文档中的状态，不要改 archive。
