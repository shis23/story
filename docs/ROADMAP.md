# StoryForge 路线图

本路线图按“先打通 Campaign 主线，再扩展外围能力”的顺序排列。

## Phase 1: Campaign 写作主线统一

**状态：已完成**（PLAN-CAMPAIGN-MAINLINE 阶段 1-6，2026-06-17）

目标：写作链路不再以扁平 `Character` 为主，而是以 Campaign + CharacterInstance 为主。

任务：

- ~~实现 `CharacterInstance` persona/behavior fallback 到 `CharacterDefinition`。~~ ✅ 阶段 1
- ~~新增 `CampaignRuntimeContext` 纯快照。~~ ✅ 阶段 2
- ~~Tauri 层从 `CampaignStore` 组装快照，传入 pipeline。~~ ✅ 阶段 2
- ~~Director 可见角色列表改为 CharacterInstance。~~ ✅ 阶段 3
- ~~`Plan` 内部字段改为 `character_instance_id`。~~ ✅ 阶段 3
- ~~Subagent 输入改为 instance + definition + 可见知识 + 变量。~~ ✅ 阶段 4
- ~~Postprocess 输出落盘前做名字到 ID 的归一化。~~ ✅ 阶段 5
- ~~修复删除角色时 Campaign 相关对象的清理/保护策略。~~ ✅ 阶段 5

验收：

- 导入单角色 ST 卡后仍能写作。
- 导入多角色卡后 Director 能选择多个 instance。
- 同名角色不会把知识或变量写串。
- 无 active campaign 时走明确 fallback，而不是隐式混用状态。

详细计划见：

- `docs/archive/2026-06-19-completed-phases/PLAN-CAMPAIGN-MAINLINE.md`（已归档）

## Phase 2: 信息隔离和状态闭环

**状态：已完成**（PLAN-CAMPAIGN-MAINLINE 阶段 3-6，2026-06-17）

目标：让角色真正只知道自己该知道的内容，并让变量/任务影响下一轮写作。

任务：

- ~~定义角色可见知识规则。~~ ✅ 阶段 4：`spawn_subagents` 按 instance 注入 knowledge（信息隔离）
- ~~Postprocess 知识写入绑定 instance。~~ ✅ 阶段 5：`persist_postprocess_outcome` 解析到 persisted `CharacterInstance.id`
- ~~Director tail 注入 pending tasks、story clock、关键 Campaign 变量。~~ ✅ 阶段 3：`build_director_tail` 注入 campaign 全局变量和 pending tasks
- ~~Subagent tail 注入角色变量和该角色可见知识。~~ ✅ 阶段 4：`build_campaign_subagent_volatile` 注入 instance variables 和 knowledge
- ~~Editor provenance 记录采用/裁剪/冲突处理。~~ ✅ 阶段 5：`SubagentSnapshot` 扩展 `character_instance_id`/`display_name`/`fallback_reason`；`build_provenance_with_campaign` 从 CampaignRuntimeContext 填充（注：详细的采用/裁剪/冲突解决记录为后续增强）
- ~~增加数据一致性检查测试。~~ ✅ 阶段 5：`persist_postprocess_outcome` 含 present_chars 校验、campaign_id 校验

验收：

- 同一 Campaign 连续三轮写作后，变量、摘要、任务会影响下一轮。
- A 角色私有知识不会泄漏给 B 角色。
- 任务触发能被 Director 稳定看到。

**隔离加固（2026-06-18，harness 收尾）**：

- ✅ P0：子 agent get_character 绑定-unresolvable 读侧泄漏已修 + 钉测。
- ✅ I1 对抗性探针实跑通过（`deepseek-v4-flash`）：对抗 prompt 成功诱导 LLM 尝试越权查询 `get_character("Chen")`，被 P0 拦下返回 NotFound，成文拿不出真秘密——隔离在 LLM 行为层端到端生效。
- ✅ P3：知识写回门禁按 `KnowledgeSource` 分流——`Witnessed`/`Inferred` 受在场约束，`ToldByOther`/`Backstory` 放行（跨在场告知/开局知识）。空集不再 hack 式全放行。
- ✅ P4：同名 instance 时 name 匹配路失效逼 id，消除同名写串。
- ✅ 知识传播引擎方向 1+2+3（2026-06-19）：显式全体广播（`BroadcastTarget::All`）、身份组广播（`BroadcastTarget::Group`）和定向告知强化（postprocess 输出告知目标，读侧带告知者名字）。广播/告知不再依赖空集 hack。
- ✅ 知识传播引擎方向 5 MVP（2026-07-06）：`PropagationPolicy::Private`、postprocess `propagation` 解析、private+broadcast 拒绝和来源私有知识阻断已落地；仍需真实 LLM 对抗评测。
- ✅ 知识传播引擎方向 4 MVP（2026-07-06）：`ToldByOther`/广播写入会用 `source_knowledge_id` 链接来源角色已有匹配知识，知识面板展示 A→B→C 传话链；仍需真实 LLM 行为评测和语义匹配增强。

详细计划见：

- `docs/archive/2026-06-19-completed-phases/PLAN-CAMPAIGN-MAINLINE.md`（已归档） 的阶段 5-6。
- 隔离加固详情见 `docs/HARNESS-FINDINGS-2026-06-18.md`。

## Phase 3: Meta Agent 维护层

**状态：已完成**（2026-06-18，PLAN-META-AGENT 阶段 1-5 全部实现）

目标：把 Meta Agent 做成 Campaign 可解释性和修复入口。

任务：

- ~~增加 Campaign health check。~~ ✅ 阶段 1：`health_check.rs` + `meta_health_check` + MetaPanel 体检按钮，4 类 issue
- ~~增加“解释本轮生成”视图。~~ ✅ 阶段 2：`explain.rs` + `meta_explain_generation` + `inspect_generation` 工具
- ~~Meta patch 类型化。~~ ✅ 阶段 3：`typed_patch.rs`（8 action：4 health-issue + 4 agent-proposed）+ preview/accept/dismiss 闭环
- ~~patch 需要 preview、accept、dismiss。~~ ✅ 阶段 3：4 个 Tauri 命令闭环
- ~~MVU 分析结果接入 variable schema。~~ ✅ 阶段 5：`mvu_apply.rs` + `meta_preview_mvu_apply`/`meta_apply_mvu_schema`
- ~~统一 tool 注册中心。~~ ✅ 第四轮 E：`tool_center.rs` + 按角色选配
- ~~Campaign-aware Meta Tools。~~ ✅ 阶段 4：6 工具（inspect_campaign/instance/variables/knowledge/tasks + propose_campaign_patch）

验收：

- ✅ 用户能问“为什么这轮这样写”，系统能引用真实 provenance。
- ✅ 用户能看到数据问题列表并接受修复。
- ✅ Meta patch 不直接越权改数据（propose → preview → accept 才写盘）。
- ✅ Meta 对 active Campaign 的回答不再只基于 `tool_ctx.characters`。

详细计划见 `docs/archive/2026-06-19-completed-phases/PLAN-META-AGENT.md`（已归档）。统一 tool 注册中心见 `docs/archive/2026-06-18-phase3-meta-tasks/PLAN-TOOL-REGISTRY.md`（已实现并归档）。

## Phase 4: 前端工作台重构

**状态：已完成**（阶段 1/2/3/4/5/6 全部完成；2026-06-19 W9/W10 落地后全链路验收通过）

目标：让前端围绕 Campaign 工作流，而不是围绕零散面板。

任务：

- ~~首屏聚焦 active campaign。~~ ✅ 阶段 1
- ~~写作入口绑定 active campaign。~~ ✅ 阶段 2：`writingMode` 三态（campaign/legacy/none），Campaign 模式传 null characterId
- ~~Campaign 面板拆成角色实例、变量、知识、任务、摘要标签页。~~ ✅ 阶段 3：拆成 4 个独立 tab 组件 + 变量类型化显示 + 修复知识/摘要字段名静默 bug
- ~~PipelinePanel 做成可检查的 Agent trace。~~ ✅ 阶段 4：subagent trace 用 instance display name，reroll 用稳定 id
- ~~MetaPanel 和 Campaign health check 打通。~~ ✅ 阶段 5：W9 补完 MetaPanel accept 后变量 tab 刷新
- ~~对移动端布局做一次专项整理。~~ ✅ 阶段 6：Pipeline 默认折叠、CharacterList 底部 sheet

验收：

- ✅ 新用户导入卡 -> 创建 Campaign -> 写第一轮的路径清晰（阶段 1/2）。
- ✅ 调试用户能看到每个 Agent 的输入输出摘要（阶段 4 trace 用 display name）。
- ✅ 移动端不依赖桌面宽屏才能操作主流程（阶段 6）。

详细计划见 `docs/archive/2026-06-19-completed-phases/PLAN-FRONTEND-WORKBENCH.md`（已归档）。

## Phase 5: ST 兼容和导入/导出

**状态：已完成**（MVU 分析/预览/渲染 + ST 导入保真 + Campaign 导出 + MVU apply 前端接线 + JS runtime 接通写作流程；2026-06-19 W9/W10 落地后全链路打通）

目标：保持 SillyTavern 卡兼容，同时不被 ST 数据形态限制内部架构。

任务：

- ~~明确 ST V2/V3 导入保真范围。~~ ✅ V2/V3 兼容
- ~~保留 raw JSON 和 extensions。~~ ✅ `raw_card_json` + `extensions` 保留
- ~~多角色识别失败时稳定 fallback。~~ ✅ `fallback_from_character`
- ~~设计 StoryForge Campaign 导出格式。~~ ✅ JSON bundle（`format_version`）
- ~~评估是否支持导出回 ST 卡或 Lorebook。~~ ✅ ST 卡 PNG（tEXt 写入）+ 多角色共享 lorebook
- ~~MVU apply 前端接线：后端 `meta_preview_mvu_apply`/`meta_apply_mvu_schema` 命令已有，前端 API 未接。~~ ✅ W9 已实现：tauri-api.js 补 API + MetaPanel 加 preview diff + 确认 apply + 变量 tab 刷新
- ~~JS Fallback WebView Runtime 接通写作流程：`WebViewMvuRuntime` + JSR/ST API 常用子集 shim 已实现（W8），trait 异步化完成，但 postprocess/pipeline 尚未调用 `execute_fragment`——runtime 未接进写作流程。~~ ✅ W10 已实现：DI 注入 + postprocess 调 execute_fragment，harness 传 None 降级；2026-07-06 已将 WebView/Tauri adapter 从 `infra-plugin-host` 拆到 `tauri-app/src/mvu_webview_runtime.rs`

验收：

- ✅ 常见 ST 卡能导入。
- ✅ 不认识的 extensions 不丢（`raw_card_json` 保底）。
- ✅ StoryForge 内部多角色 Campaign 不强行退化为单角色卡。
- ✅ Campaign 可导出为 ST 卡 PNG + 共享 lorebook + StoryForge JSON bundle。

详细计划：

- `docs/PLAN-PLUGIN-MVU.md` 覆盖 MVU 状态栏、schema preview 和 JS fallback。
- `docs/PLAN-ST-IMPORT-EXPORT.md` 覆盖 ST 导入保真 + Campaign 导出（T4/T5 已实现）。

已知限制（非阻塞）：

- **JS 变量归口**：`execute_fragment` 产出的 `variable_updates` 统一写入 campaign 级变量（`instance_id=None`）。若需按角色归属，后续需细化 key 前缀解析或扩展 `MvuExecResult` 携带 `instance_id`。
- **JSR shim 覆盖度**：`WebViewMvuRuntime` 的 JSR/ST API shim 基于常用子集实现，依赖冷门 API 的重 DOM 卡可能降级为跳过 JS 更新（`tracing::warn!` + 执行失败/超时回退），不影响主写作。

## Phase 6: Android 打磨

**状态：已启动**（约 20%，2026-07-06 已完成 arm64-v8a debug/release 构建基线；详见 `DOCS-CODE-AUDIT.md` 和 `PLAN-ANDROID.md`）

目标：把桌面调试能力收束为移动端可用体验。

任务：

- Android 文件导入路径验证。
- 本地存储迁移策略。
- 长文本流式显示性能检查。
- 断网、取消、失败重试体验。
- 日志和导出 bundle 适配移动端排障（当前已补诊断上下文摘要；Android save/share sheet 待真机验证）。

验收：

- Android 端能完成主流程。
- 长会话不会明显卡顿。
- 出错时能导出足够排查的信息。

详细计划见 `docs/PLAN-ANDROID.md`。

## Phase 7: 收口、验收和发布准备

目标：当前所有专项计划完成后，把项目从“功能打通”推进到“可验证、可发布、可继续迭代”。

任务：

- 建立端到端验收矩阵。
- 固定回归测试和 LLM 质量评测样例。
- 验证数据备份、迁移、恢复和排障 bundle。
- 检查长会话性能、Agent 调用成本和移动端稳定性。
- 准备桌面端/Android 发布包和首次使用文档。
- 明确下一阶段战略选择和不做事项。

验收：

- 新用户能按文档完成第一局 Campaign。
- 测试者能按 checklist 完成发布前检查。
- 项目有明确的 post-mainline 优先级，不再无序扩张。

详细计划见 `docs/PLAN-POST-MAINLINE.md`。
