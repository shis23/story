# StoryForge 架构审计

> 状态：2026-06-16 调研版
> 范围：只评估现有架构与后续重构顺序，不包含代码改动。

## 结论

现有代码不需要推倒重写，但需要一次中等规模的主链路重构。

最大问题不是模块拆分错误，而是“运行时权威数据源”分裂：`CampaignStore` 已经有 Campaign、CharacterInstance、变量、知识、任务、摘要、MVU 分析结果，但写作入口和 Agent 流水线仍主要读取扁平 `CharacterStore` / `ToolContext.characters`。继续在外围堆 Meta、插件、Android UI，会让这些功能都围绕旧链路打补丁，后面返工会更大。

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
       fill_profile_context()
       fill_campaign_context()
          只填 campaign_id / story_clock / turn / pending_tasks
       PipelineOrchestrator::start_writing()
          Director 看 ctx.characters + world_info + tasks
          spawn_subagents(plan.subagent_tasks)
          Editor 合并
          run_postprocess()
       persist_postprocess_outcome()
          写 summary / knowledge / variables / tasks
```

关键断点：

- `crates/tauri-app/src/lib.rs::fill_campaign_context` 只加载 Campaign 标量字段和任务，没有加载 instances、definitions、knowledge、变量快照。
- `crates/app-pipeline/src/lib.rs::WritingContext` 没有 Campaign runtime 快照字段。
- `crates/app-pipeline/src/lib.rs::build_director_tail` 仍从 `ctx.characters` 渲染可用角色。
- `crates/app-agent/src/tools.rs::ToolContext` 只有 `characters/world_info/vector_store/archived_summaries`。
- `crates/app-agent/src/runtime.rs::spawn_subagents` 不接收 Campaign 上下文，子 Agent persona 依赖 Director 生成的 `context_package`。
- `crates/tauri-app/src/lib.rs::persist_postprocess_outcome` 会写 CampaignStore，但变量/知识 ID 归一仍偏弱，`present_chars` 未真正使用。

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

