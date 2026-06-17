# 技术设计

本文件保留为技术设计入口。历史长版设计已归档到 `docs/archive/2026-06-16-pre-rewrite/TECHNICAL_DESIGN.md`。

当前权威文档拆分如下：

- `docs/ARCHITECTURE.md`：代码架构、模块边界、写作流水线。
- `docs/DATA_MODEL.md`：角色、Campaign、变量、知识、任务模型。
- `docs/AGENT_INTERFACES.md`：Director、Subagent、Editor、Postprocess、Meta Agent 契约。
- `docs/PLAN-CAMPAIGN-MAINLINE.md`：Campaign 写作主线当前执行计划。
- `docs/archive/2026-06-17-campaign-mainline-phase5/PLAN-CHARACTER-UNIFICATION.md`：Campaign 角色统一历史实施记录（已归档）。
- `docs/ROADMAP.md`：后续分阶段路线图。

## 当前设计主线

StoryForge 的技术主线是把以下闭环打通：

```text
ST Card Import
  -> CharacterCard / CharacterDefinition
  -> Campaign / CharacterInstance
  -> CampaignRuntimeContext
  -> Director / Subagents / Editor
  -> Conversation + Provenance
  -> Postprocess
  -> Campaign knowledge / variables / tasks / summaries
  -> Next writing round
```

所有新增能力都应优先服务这个闭环。

## 非目标

短期不建议优先做：

- 新增大型外围面板。
- 让 Meta Agent 直接替代写作流水线。
- 在 app 层直接引用 Tauri store。
- 为单个 ST 卡格式做过度定制，破坏内部 Campaign 模型。
