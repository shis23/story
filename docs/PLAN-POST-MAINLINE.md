# 计划：主线完成后的收口与发布准备

> 状态：进行中（2026-07-06 已建立自动化验证基线和 release checklist 初版）
> 前置：`docs/archive/2026-06-19-completed-phases/PLAN-CAMPAIGN-MAINLINE.md`（已归档）、`docs/archive/2026-06-19-completed-phases/PLAN-META-AGENT.md`（已归档）、`docs/archive/2026-06-19-completed-phases/PLAN-FRONTEND-WORKBENCH.md`（已归档）、`docs/PLAN-PLUGIN-MVU.md`、`docs/PLAN-ANDROID.md` 的核心阶段已完成。
> 目标读者：完成前序计划后，负责把项目从“功能打通”推进到“可验证、可发布、可继续迭代”的执行者。

## 目标

把 StoryForge 从“主链路可用”收口到一个可交付版本：

- 主流程能稳定跑通。
- 数据闭环可验证。
- 失败时可恢复、可排查。
- Android 端具备真实使用条件。
- 后续方向有明确取舍，而不是继续无序扩张。

## 非目标

- 不在本阶段新增大型功能。
- 不重写存储层，除非迁移/备份验证证明当前 JSON store 阻塞发布。
- 不做插件市场、云同步、多用户协作等二阶段能力。
- 不为了追求“完美架构”重构已经稳定的主链路。

## 前置完成条件

进入本计划前，应满足：

- Campaign 是写作运行时真相源。
- Director、Subagent、Editor、Postprocess 都能消费或回写 Campaign 相关状态。
- Meta Agent 能做 Campaign health check、解释生成、preview patch。
- 前端主入口围绕 active Campaign，而不是 active Character。
- MVU 至少能进入 variable schema preview，并支持基础状态栏渲染。
- Android 能完成导入、创建 Campaign、写作、查看结果的主流程。

如果以上任一项不满足，回到对应 `PLAN-*.md`，不要提前进入发布准备。

## 阶段 1：端到端验收矩阵

目标：建立一组固定场景，证明核心体验不是“单点能跑”，而是闭环稳定。

改动文件：

- `docs/RELEASE-CHECKLIST.md`（已新增初版，持续补充真实卡/Android 手工结果）
- `docs/HANDOFF.md`（2026-06-16 旧版已归档至 `docs/archive/2026-06-18-pre-phase-completion/HANDOFF.md`；本阶段需重新生成）
- 如需要，新增 `fixtures/` 或 `tests/fixtures/` 下的样例卡说明。

任务：

1. 定义最小验收场景：
   - 单角色 ST 卡导入 -> 创建 Campaign -> 写第一轮。
   - 多角色 ST 卡导入 -> Director 选择多个实例 -> Subagent 分角色输出。
   - 同名角色场景 -> 知识/变量不串写。
   - 连续三轮写作 -> summary、knowledge、variables、tasks 影响下一轮。
   - Meta 解释本轮生成 -> 能引用真实 trace/provenance。
   - Meta patch preview -> accept/dismiss 后状态正确。
   - MVU 卡 -> schema preview -> 状态栏渲染基础变量。
   - Android 端重复主流程。
2. 为每个场景写清楚：
   - 输入材料。
   - 操作步骤。
   - 预期结果。
   - 失败时查看哪些日志/导出包。
3. 把不能自动化的验收标成手动验收，不要伪装成单元测试。

验收：

- `docs/RELEASE-CHECKLIST.md` 能让一个没有上下文的小模型或人工测试者按步骤完成验收。
- 每个核心能力都有至少一个场景覆盖。

## 阶段 2：回归测试与评测基线

目标：把最容易返工的地方变成可重复检查项。

改动文件：

- Rust 相关测试文件，按实际模块选择。
- 前端测试文件，按现有测试框架选择；如果项目没有前端测试基线，先写文档化手动检查。
- `docs/RELEASE-CHECKLIST.md`

任务：

1. Rust 回归重点：
   - `CampaignRuntimeContext` 装配。
   - instance id/name 归一。
   - 变量注入渲染。
   - knowledge visibility。
   - postprocess 写回。
2. 前端回归重点：
   - active Campaign 写作入口。
   - CampaignPanel 各标签数据刷新。
   - Pipeline trace 展示。
   - Meta patch preview/accept/dismiss。
3. LLM 质量评测重点：
   - Director 是否选择正确角色。
   - 子 Agent 是否只使用可见知识。
   - Editor 是否保留角色差异。
   - Postprocess 是否避免凭空创建永久事实。
4. 记录每项评测的可接受波动范围。

验收：

- 核心身份/状态闭环有自动化测试覆盖。
- LLM 输出质量有固定样例和人工评分表，不依赖临场感觉。

## 阶段 3：数据安全、迁移和恢复

目标：发布前确认用户数据不会因为升级、崩溃或导入失败而不可恢复。

改动文件：

- `crates/tauri-app/src/lib.rs`
- store 相关模块。
- `docs/DATA_MODEL.md`
- `docs/RELEASE-CHECKLIST.md`

任务：

1. 明确本地数据目录：
   - Windows 桌面端。
   - Android 端。
2. 增加或验证备份导出：
   - Campaign。
   - Character definitions/instances。
   - raw ST JSON。
   - conversation tree。
   - summaries/knowledge/variables/tasks。
   - Meta patches 和 MVU translations。
3. 明确数据版本字段。
4. 若需要迁移，迁移必须满足：
   - 幂等。
   - 可检测失败。
   - 失败不覆盖原始数据。
5. 验证异常场景：
   - 导入坏卡。
   - 写作中断。
   - postprocess 失败。
   - app 重启后恢复 active Campaign。
6. 技术债闸门：
   - `CampaignStore` 写入错误已改为 `Result` 并在 Tauri 命令路径传播；postprocess 后台写回失败会记录 warning。
   - 仍需对 `CampaignStore` 做一次长任务/连续写回压测，记录单 Mutex + JSON I/O 的锁持有时间和 UI 可感知卡顿。
   - 若确认为发布阻塞，优先拆“内存态 + 后台批量 flush / 原子写”边界；不要在没有压测证据时整层重写。

验收：

- 用户能导出完整排障 bundle。
- 升级/迁移失败不会破坏原始数据。
- Android 和桌面端的数据目录策略写入文档。

## 阶段 4：性能、成本和长会话稳定性

目标：确认长会话、长文本和多 Agent 调用不会让体验不可用。

改动文件：

- `crates/app-pipeline/src/lib.rs`
- `crates/app-agent/src/runtime.rs`
- `frontend/src/components/StreamingMessage.vue`
- `frontend/src/components/ChatMessage.vue`
- `frontend/src/App.vue`
- `docs/RELEASE-CHECKLIST.md`

任务：

1. 测量多轮写作：
   - 首 token 时间。
   - 总生成时间。
   - Agent 调用次数。
   - prompt 长度。
   - summary/knowledge/variables 注入长度。
2. 检查 cache 友好布局是否仍成立：
   - 稳定 system 不混入每轮变量。
   - 易变内容留在 tail。
3. 长文本 UI 检查：
   - 流式输出不卡顿。
   - `StreamingMessage.vue` 不因 trace/流式内容过长卡死。
   - `ChatMessage.vue` provenance 和 reroll 控件在长 trace 下仍可用。
   - 移动端滚动正常。
4. 成本保护：
   - Agent 最大轮次有上限。
   - 向量/知识注入有长度上限。
   - 失败重试不无限循环。

验收：

- 连续长会话不会明显退化到不可用。
- 有一张记录表说明典型场景的延迟、调用次数和风险。

## 阶段 4.5：架构技术债闸门

目标：把已知分层债和存储债纳入发布前判断，避免它们在 Android/长任务场景里变成隐性阻塞。

改动文件：

- `crates/infra-plugin-host/src/mvu_runtime.rs`
- `crates/infra-plugin-host/Cargo.toml`
- `crates/tauri-app/src/lib.rs`
- `crates/tauri-app/src/mvu_webview_runtime.rs`
- `crates/tauri-app/src/campaign_store.rs`
- `docs/ARCHITECTURE-AUDIT.md`
- `docs/RELEASE-CHECKLIST.md`

任务：

1. `infra-plugin-host` 分层债：
   - 已拆分：`infra-plugin-host` 只保留 `MvuRuntime` trait、DTO、事件名和纯错误类型，`Cargo.toml` 不再依赖 Tauri。
   - `WebViewMvuRuntime` 已移动到 `crates/tauri-app/src/mvu_webview_runtime.rs`，由 Tauri 层持有 `tauri::AppHandle`、emit event 并等待 pending oneshot。
   - `app-pipeline` / `app-agent` 仍只依赖 infra trait，不直接依赖 `tauri-app`。
2. `CampaignStore` 存储债：
   - 当前适合桌面开发和小数据量；写入错误已可见，但单 Mutex + 同步 JSON I/O 在 Android 和长会话里仍可能放大卡顿。
   - 发布前先测锁持有时间、连续 postprocess 写回、导入大卡和 app 重启恢复；只有确认阻塞后再做后台 flush / 分文件索引 / schema 迁移。
3. 文档同步：
   - 每次完成技术债切片后同步 `ARCHITECTURE-AUDIT.md`、`DATA_MODEL.md`、`PLAN-POST-MAINLINE.md` 和 README 的代码事实。

验收：

- 技术债是否阻塞发布有明确证据，而不是凭感觉。
- `infra-plugin-host` 的 Tauri 依赖已拆掉；release checklist 仅保留 WebView MVU 真实卡回归风险。
- `CampaignStore` 的性能风险有可复现实验记录和处理结论。

## 阶段 5：发布包和用户入口

目标：让项目具备可交付版本的基本形态。

改动文件：

- Tauri 配置。
- Android 配置。
- `README.md`
- `docs/RELEASE-CHECKLIST.md`
- 如需要，新增 `docs/USER-GUIDE.md`。

任务：

1. 桌面端：
   - 验证 dev 与 release build。
   - 明确配置文件位置。
   - 明确 LLM API key 配置方式。
2. Android：
   - 验证 debug/release 构建链路。
   - 确认签名策略。
   - 验证文件导入权限。
   - 验证日志/排障 bundle 导出。
3. 首次使用流程：
   - 导入角色卡。
   - 创建 Campaign。
   - 写第一轮。
   - 查看 Campaign 状态。
   - 使用 Meta 修复一个问题。
4. 用户文档：
   - 写清楚 ST 卡兼容范围。
   - 写清楚 Campaign 与角色卡的区别。
   - 写清楚数据本地存储和备份方式。

验收：

- 一个新用户按 README/USER-GUIDE 能完成第一局 Campaign。
- 一个测试者能按 RELEASE-CHECKLIST 完成发布前检查。

## 阶段 6：下一阶段战略决策

目标：在发布前明确下一阶段做什么和不做什么，避免重新发散。

候选方向：

1. 云同步和多设备。
2. 插件市场。
3. 更完整的 MVU/WebView 兼容。
4. 桌面端产品化。
5. Android 深度体验优化。
6. Campaign 分享/导出生态。
7. LLM 评测与自动优化。

决策标准：

- 是否直接提升 Campaign 主体验。
- 是否依赖已经稳定的数据模型。
- 是否会引入安全、隐私、同步冲突等新风险。
- 是否能被小模型拆成低风险阶段。

验收：

- `docs/ROADMAP.md` 增加“Post-mainline 之后”的明确优先级。
- 至少列出三个明确不做的方向和原因。

## 禁止改动

- 禁止在发布准备阶段绕过 Campaign 主线新增平行写作入口。
- 禁止让 Meta patch 或插件绕过 preview/accept 直接写核心数据。
- 禁止为通过验收而硬编码样例卡。
- 禁止把失败隐藏成成功；失败必须能被日志、UI 或导出 bundle 观察到。
- 禁止改 archive 文档。
