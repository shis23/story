# StoryForge 交接说明

更新时间：2026-06-16

## 当前状态

StoryForge 已经具备：

- ST 角色卡导入和原始 JSON 保留。
- 多角色模型：`CharacterCard`、`CharacterDefinition`、`CharacterInstance`。
- Campaign store：Campaign、实例、变量、知识、任务、轮次摘要。
- 多 Agent 写作流水线：Director、Subagents、Editor。
- 对话树和 variant/re-roll。
- Postprocess 后处理雏形。
- Meta Agent 会话、补丁、MVU 分析入口。
- Vue 前端主要面板。

当前主线问题：

- 写作流水线仍以扁平 `Character` 为主要输入。
- Campaign 数据已经存在，但还没有成为写作运行时真相源。
- 角色名和角色 ID 边界不清，后续会影响知识、变量和同名角色。

## 已完成的文档重写

旧版文档已归档：

```text
docs/archive/2026-06-16-pre-rewrite/
```

新版核心文档：

- `README.md`
- `docs/DOCS-CODE-AUDIT.md`
- `docs/ARCHITECTURE-AUDIT.md`
- `docs/ARCHITECTURE.md`
- `docs/DATA_MODEL.md`
- `docs/AGENT_INTERFACES.md`
- `docs/ROADMAP.md`
- `docs/PLAN-CAMPAIGN-MAINLINE.md`
- `docs/PLAN-META-AGENT.md`
- `docs/PLAN-FRONTEND-WORKBENCH.md`
- `docs/PLAN-ANDROID.md`
- `docs/PLAN-PLUGIN-MVU.md`
- `docs/PLAN-POST-MAINLINE.md`
- `docs/PLAN-AGENT-PROFILE.md`
- `docs/PLAN-CHARACTER-EXTRACTION.md`
- `docs/HANDOFF.md`

## 下一步建议

先读 `docs/DOCS-CODE-AUDIT.md`，区分哪些文档内容是当前代码事实、哪些是未来计划；再读 `docs/ARCHITECTURE-AUDIT.md`，确认当前结论：项目不需要推倒重写，但需要一次围绕 Campaign 主线的中等规模重构。

执行顺序优先按 `docs/PLAN-CAMPAIGN-MAINLINE.md`，其中前置核心步骤是：

1. ~~修正 `CharacterInstance::resolved_persona()` / `resolved_behavior()` fallback。~~ **阶段 1 已完成**（2026-06-16）。
2. 新增 `CampaignRuntimeContext`。**阶段 2 已完成**（2026-06-16）：DTO + helpers 在 `domain/campaign_runtime.rs`，已接入 WritingContext/ToolContext，`fill_campaign_context` 已组装快照。
3. ~~Tauri 层组装 runtime snapshot。~~ **阶段 2 已完成**。
4. Director 的可用角色列表改为 instances。
5. `Plan` 内部使用 `CharacterInstance.id`。
6. Subagent 上下文改为 instance + definition + variables + visible knowledge。
7. Postprocess 落盘前做名字到 ID 归一化。

`docs/archive/2026-06-17-campaign-mainline-phase5/PLAN-CHARACTER-UNIFICATION.md` 已归档为角色体系历史计划；当前执行入口以 `docs/PLAN-CAMPAIGN-MAINLINE.md` 为准。后续再按 `PLAN-META-AGENT.md`、`PLAN-FRONTEND-WORKBENCH.md`、`PLAN-PLUGIN-MVU.md`、`PLAN-ANDROID.md` 推进外围能力。

`docs/PLAN-AGENT-PROFILE.md` 阶段 1-5 全部实现。运行时闭环：`tool_whitelist` 通过 `ToolRegistry::retain`/`filter_registry_by_whitelist` 在 Director/Subagent/PostProcessor 注册后过滤（None=默认全部，Some=[]=禁用全部，Some(list)=只允许列表工具；未知工具名记 warning 不 panic；被禁用的工具 dispatch 返回 `NotFound`，不可绕过）。`enable_postprocess`/`enable_summarizer` 由 `run_postprocess_pipeline` 消费，false 时跳过对应 LLM 调用；两者都关时流水线发 `PipelineEvent::PostProcessSkipped`（而非误导性的 `PostProcessFailed`）。`max_concurrent_subagents = 0` 会按 `1` 执行，避免子 Agent 调度挂死。前端管理 UI 在 `frontend/src/components/AgentProfileManager.vue`（power 模式下，`AgentConfigCard` 之后）。阶段 5 新增：`ProfileConfigError` + `validate()`（空名/越界 rounds/越界并发）+ `migrate_to()` 版本迁移骨架；`save()` 保存前校验失败返回明确错误。未新增 `temperature` 字段（温度由连接配置控制）。剩余：Profile JSON 导入导出（可选扩展）。

上述专项计划完成后，不要直接开新功能。继续执行 `docs/PLAN-POST-MAINLINE.md`，完成端到端验收、回归评测、数据安全、性能成本、发布包和下一阶段战略决策。

## 工作注意事项

- 不要让 `app-agent` 或 `app-pipeline` 直接依赖 `CampaignStore`。
- 不要把 ST 卡兼容层当成写作主模型。
- 不要在后处理里凭空创建永久角色。
- 写作内部必须使用稳定 ID，名字只做显示和 LLM 交互。
- Meta Agent 的补丁必须可预览、可拒绝、可追踪。
