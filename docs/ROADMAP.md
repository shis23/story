# StoryForge 路线图

本路线图按“先打通 Campaign 主线，再扩展外围能力”的顺序排列。

## Phase 1: Campaign 写作主线统一

目标：写作链路不再以扁平 `Character` 为主，而是以 Campaign + CharacterInstance 为主。

任务：

- 实现 `CharacterInstance` persona/behavior fallback 到 `CharacterDefinition`。
- 新增 `CampaignRuntimeContext` 纯快照。
- Tauri 层从 `CampaignStore` 组装快照，传入 pipeline。
- Director 可见角色列表改为 CharacterInstance。
- `Plan` 内部字段改为 `character_instance_id`。
- Subagent 输入改为 instance + definition + 可见知识 + 变量。
- Postprocess 输出落盘前做名字到 ID 的归一化。
- 修复删除角色时 Campaign 相关对象的清理/保护策略。

验收：

- 导入单角色 ST 卡后仍能写作。
- 导入多角色卡后 Director 能选择多个 instance。
- 同名角色不会把知识或变量写串。
- 无 active campaign 时走明确 fallback，而不是隐式混用状态。

详细计划见：

- `docs/PLAN-CAMPAIGN-MAINLINE.md`

## Phase 2: 信息隔离和状态闭环

目标：让角色真正只知道自己该知道的内容，并让变量/任务影响下一轮写作。

任务：

- 定义角色可见知识规则。
- Postprocess 知识写入绑定 instance。
- Director tail 注入 pending tasks、story clock、关键 Campaign 变量。
- Subagent tail 注入角色变量和该角色可见知识。
- Editor provenance 记录采用/裁剪/冲突处理。
- 增加数据一致性检查测试。

验收：

- 同一 Campaign 连续三轮写作后，变量、摘要、任务会影响下一轮。
- A 角色私有知识不会泄漏给 B 角色。
- 任务触发能被 Director 稳定看到。

详细计划见：

- `docs/PLAN-CAMPAIGN-MAINLINE.md` 的阶段 5-6。

## Phase 3: Meta Agent 维护层

目标：把 Meta Agent 做成 Campaign 可解释性和修复入口。

任务：

- 增加 Campaign health check。
- 增加“解释本轮生成”视图：intent、Plan、Subagent 输入输出、Editor 输出、Postprocess 结果。
- Meta patch 类型化：变量修复、知识修复、角色合并、prompt module 修改。
- patch 需要 preview、accept、dismiss。
- MVU 分析结果接入 variable schema。

验收：

- 用户能问“为什么这轮这样写”，系统能引用真实 provenance。
- 用户能看到数据问题列表并接受修复。
- Meta patch 不直接越权改数据。

详细计划见 `docs/PLAN-META-AGENT.md`。

## Phase 4: 前端工作台重构

目标：让前端围绕 Campaign 工作流，而不是围绕零散面板。

任务：

- 首屏聚焦 active campaign。
- 写作入口绑定 active campaign。
- Campaign 面板拆成角色实例、变量、知识、任务、摘要标签页。
- PipelinePanel 做成可检查的 Agent trace。
- MetaPanel 和 Campaign health check 打通。
- 对移动端布局做一次专项整理。

验收：

- 新用户导入卡 -> 创建 Campaign -> 写第一轮的路径清晰。
- 调试用户能看到每个 Agent 的输入输出摘要。
- 移动端不依赖桌面宽屏才能操作主流程。

详细计划见 `docs/PLAN-FRONTEND-WORKBENCH.md`。

## Phase 5: ST 兼容和导入/导出

目标：保持 SillyTavern 卡兼容，同时不被 ST 数据形态限制内部架构。

任务：

- 明确 ST V2/V3 导入保真范围。
- 保留 raw JSON 和 extensions。
- 多角色识别失败时稳定 fallback。
- 设计 StoryForge Campaign 导出格式。
- 评估是否支持导出回 ST 卡或 Lorebook。

验收：

- 常见 ST 卡能导入。
- 不认识的 extensions 不丢。
- StoryForge 内部多角色 Campaign 不强行退化为单角色卡。

详细计划：

- `docs/PLAN-PLUGIN-MVU.md` 覆盖 MVU 状态栏、schema preview 和 JS fallback。
- ST V2/V3 导入保真、StoryForge Campaign 导出、是否导出回 ST/Lorebook 仍缺独立执行计划；进入本阶段前应补 `docs/PLAN-ST-IMPORT-EXPORT.md`。

## Phase 6: Android 打磨

目标：把桌面调试能力收束为移动端可用体验。

任务：

- Android 文件导入路径验证。
- 本地存储迁移策略。
- 长文本流式显示性能检查。
- 断网、取消、失败重试体验。
- 日志和导出 bundle 适配移动端排障。

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
