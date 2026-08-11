# StoryForge 前端组件蓝图

> 更新日期：2026-07-21  
> 用途：**组件 / 能力 / 信息架构地图**，给视觉重设计与出图（含 gpt-image）当输入。  
> **不是**现有 UI 的视觉规范，也**不要**照抄当前颜色、字号、间距、阴影、卡片密度。  
> （历史效果稿已不随仓库分发。）

## 0. 视觉重设计立场（2026-07-21）

### 必须保留（产品结构）

- Campaign-first 多角色写作工作台（不是纯聊天玩具页）。
- 主路径：导入卡 → 建/开 Campaign → 写作 → 看结果/变体 → 管理知识任务 → 导出。
- 信息架构：中间主写作；左右为导航/管理与调试（桌面可分栏，移动端可抽屉）。
- 本文列出的**业务能力与组件职责**（能写、能停、能重 roll、能看变体、能管 Campaign、有 Meta/插件入口等）。

### 明确丢掉（现有外观）

当前 `AppV2` + `components-v2` 已能跑通主流程，但视觉被视为**工程原型，不够好看**。重设计时：

| 丢掉 | 说明 |
| --- | --- |
| 现有配色 / token 外观 | 含当前深色底、紫强调、glow 阴影等；可全新定调 |
| 现有字号、行高、间距、圆角、边框 | 不继承“密、灰、调试台”观感 |
| 现有组件皮肤 | Button/TopBar/消息气泡等只保留职责，不保留长相 |
| “像 IDE / 控制台 / 管理后台”的默认感 | 目标是创作/叙事写作工具，不是运维面板 |

实现文件路径（`components-v2/**`）仅供开发对照，**禁止**当作视觉参考截图或风格约束。

### 出图 / 重设计目标

第一屏应是**愿意打开的写作工作台**，而不是组件堆叠演示页。

1. 写作区是视觉主角：消息与输入清晰、呼吸感够，管理功能不抢戏。  
2. Campaign 主线一眼可懂：当前故事、角色、写作状态可读。  
3. 高级能力（pipeline trace、插件、hook 审计）默认退后，需要时再展开。  
4. 桌面精致、移动端主流程完整；空状态与加载态也要好看。

风格方向**开放**（深色或浅色、编辑器感或阅读器感均可），只要统一、现代、适合长文中文叙事；不要 steampunk 堆料、不要赛博霓虹噪音、不要纯仪表盘。

## 1. 体验优先级（能力，不是皮肤）

1. 写作不被管理功能打断：主写作区始终清晰，管理面板用抽屉/分栏承载。
2. Campaign 是主线：角色卡、实例、知识、任务、摘要、MVU 都围绕 active Campaign 组织。
3. 插件/ST 兼容要可见但不喧宾夺主：普通用户看到降级提示，高玩模式看到事件、hook、trace。
4. 移动端保留主流程：导入、建 Campaign、写作、查看结果、导出，其他复杂功能可降级为抽屉。

## 2. 信息架构

### 主应用框架

- `AppShell`：应用根布局，负责全屏高度、主区域、移动/桌面断点。
- `TopBar`：当前标题、写作状态、菜单按钮、调试入口。
- `PrimarySidebar`：主导航，包含写作、素材、配置、Meta、导入入口。
- `InspectorDrawer`：右侧调试/高玩抽屉，展示 pipeline、插件事件、prompt hook 审计。
- `WorkspaceView`：中间工作区容器，在写作、会话历史、Campaign 概览之间切换。
- `PanelHost`：统一承载左抽屉、右抽屉、居中弹窗和嵌入式管理面板。

### 主要视图

- 写作视图：消息流、开场白选择、流式生成、底部输入框、写后过程回顾。
- 会话历史：会话列表、删除、打开、分支入口。
- Campaign 概览：当前 Campaign 摘要、故事时间、快捷操作。
- Campaign 管理：角色卡、Campaign 列表、详情子页、导入导出。
- Meta 助手：健康检查、修复建议、MVU 分析、生成解释。
- 配置中心：连接、预设、Agent Profile、插件。

## 3. 基础组件

这些组件应该先统一，后续业务组件只组合它们。

| 组件 | 用途 | 关键状态 |
| --- | --- | --- |
| `Button` | 文本按钮、主操作、危险操作 | default / hover / active / loading / disabled |
| `IconButton` | 菜单、关闭、删除、重 roll、分支、复制 | tooltip / selected / danger |
| `Input` | 单行文本、名称、搜索 | invalid / disabled / loading |
| `Textarea` | 写作意图、Meta 输入、长文本编辑 | auto-resize / submit hint |
| `Select` | 角色卡、开场白、预设、模型 | empty / loading |
| `SegmentedControl` | tab、模式切换、视图切换 | selected / disabled |
| `Tabs` | Campaign detail 子页、Meta 子区 | lazy load / dirty |
| `Checkbox` / `Toggle` | 启用插件、开关 postprocess、power mode | checked / indeterminate |
| `Slider` / `NumberInput` | rounds、并发数、温度类参数 | min/max/error |
| `Menu` | 消息更多操作、插件操作、导出选项 | keyboard close |
| `Tooltip` | 图标说明、风险说明 | hover / focus |
| `Badge` | 状态、权限、来源、模式 | ok / warn / danger / neutral |
| `Progress` | 生成中、MVU 变量条、导入进度 | determinate / indeterminate |
| `Overlay` | 抽屉/弹窗底座 | left / right / center / full |
| `Dialog` | 确认、警告、删除 | confirm / cancel / danger |
| `Toast` | 成功/失败/后台任务反馈 | auto dismiss / persistent |
| `EmptyState` | 空列表、未导入、未选择 | action slot |
| `ErrorState` | 导入失败、API 失败、插件错误 | retry / details |
| `LoadingState` | skeleton、spinner、inline loading | compact / full |
| `DataList` | 卡片/会话/插件列表 | selectable / active item |
| `DataTable` | 知识、变量、任务、审计日志 | sort / filter / row action |
| `DiffView` | patch preview、MVU schema apply | added / changed / removed |
| `CodeBlock` | raw JSON、trace、插件日志 | copy / wrap |

## 4. 应用框架组件

| 组件 | 能力职责（与长相无关） |
| --- | --- |
| `AppShell` | 根布局、视图路由、全局弹层状态。 |
| `TopBar` | 当前页面标题、模式副标题、生成状态、菜单/调试入口。 |
| `PrimarySidebar` | 主导航、当前上下文摘要、主题切换、power mode。 |
| `MobileNavigationDrawer` | 移动端主菜单，点完回到主工作区。 |
| `InspectorDrawer` | pipeline、连接状态、插件事件、prompt hook 审计。 |
| `PanelHost` | 统一弹层尺寸、标题、返回、关闭、滚动策略。 |

布局心智可以仍是“左导航 + 中写作 + 右调试”，但**栏宽、材质、层次、是否常驻**全部可重画；桌面可常驻侧栏，移动端用抽屉。

## 5. 写作组件

| 组件 | 能力职责（与长相无关） |
| --- | --- |
| `ConversationViewport` | 空状态、开场白、消息列表、流式消息、写后过程回顾。 |
| `GreetingSelector` | 多开场白选择，当前选中可读。 |
| `ChatMessage` | 单条用户/助手消息；active variant、操作菜单、provenance 摘要。 |
| `MessageVariantSwitcher` | 上/下一版、版本计数、采纳、删除、分支。 |
| `MessageActionMenu` | 重 roll、编辑、添加变体、分支、删除。 |
| `RichContent` | 安全展示 display-only HTML；Markdown/纯文本降级。 |
| `StreamingMessage` | Director/Subagent/Editor 流式过程，可折叠。 |
| `ProcessReview` | 本轮写完后的过程/质量回顾（可收起，不抢消息主角）。 |
| `Composer` | 写作意图输入、开始/停止、禁用提示、快捷键。 |
| `ConversationHistoryList` | 会话列表、删除、打开、空状态。 |
| `CampaignOverview` | 当前 Campaign 快捷入口、故事时间、历史数量。 |

## 6. Campaign 与角色卡组件

| 组件 | 能力职责（与长相无关） |
| --- | --- |
| `CampaignPanel` | Campaign 管理总入口：tab 与局部刷新。 |
| `CardLibrary` | 已导入角色卡列表、抽取状态、重新识别。 |
| `CardDetailPreview` | 卡名、tags、alternate greetings、extensions 保真提示。 |
| `CharacterListPanel` | 兼容入口的角色卡列表。 |
| `CampaignList` | 某张卡下的 Campaign 列表；设 active、打开详情。 |
| `NewCampaignForm` | 选卡、选开场白、命名、创建并开始。 |
| `CampaignDetailTabs` | instances / knowledge / tasks / summaries 分页。 |
| `InstancesTab` | 实例列表、临时实例升格、变量编辑。 |
| `InstanceVariableEditor` | bool/number/json/text 变量编辑。 |
| `KnowledgeTab` | 知识列表、来源、传话链、private/封口标记。 |
| `TasksTab` | 任务列表、状态变更、完成/放弃。 |
| `SummariesTab` | 轮次摘要、时间线、关联节点。 |
| `CampaignExportImportBar` | StoryForge bundle / ST 卡 / 共享 lorebook 导入导出状态。 |

## 7. MVU、ST 与富内容组件

| 组件 | 能力职责（与长相无关） |
| --- | --- |
| `MvuStatusBar` | 变量状态条；text / tag / icon / bar 等块。 |
| `MvuStatusBlock` | 单个变量块。 |
| `MvuJsRuntimeHost` | 隐藏 JS runtime；降级/错误可接到 Inspector。 |
| `MvuSchemaPreview` | schema 新增/覆盖/无变化 preview。 |
| `MvuApplyResult` | apply 结果、失败提示、关联 Campaign tab。 |
| `StCompatibilityBadge` | regex / HTML / TavernHelper / Slash / prompt hook 支持或降级提示。 |
| `RegexScriptSummary` | placement、promptOnly / markdownOnly / depth 摘要。 |
| `DisplayContentBoundary` | raw content 与 display content 的展示边界。 |

## 8. Meta Agent 组件

| 组件 | 能力职责（与长相无关） |
| --- | --- |
| `MetaPanel` | Meta 总入口：聊天、健康检查、patch、MVU、解释。 |
| `MetaChat` | 与 Meta Agent 对话；流式与工具结果可折叠。 |
| `MetaToolResultCard` | 世界书报告、卡片报告、patch proposal 等结构化展示。 |
| `HealthCheckPanel` | Campaign health issue 与修复入口。 |
| `TypedPatchList` | typed patch 列表与状态。 |
| `PatchPreview` | preview diff、影响范围、accept / dismiss。 |
| `GenerationExplanation` | 本轮 trace / provenance 解释。 |
| `MvuAnalyzer` | 选卡、分析 MVU、看翻译列表。 |

## 9. 配置、插件与调试组件

| 组件 | 能力职责（与长相无关） |
| --- | --- |
| `ConnectionConfigPanel` | LLM / embedder 配置、SecretRef 状态、测试连接。 |
| `PresetPanel` | prompt preset、regex scripts、active preset。 |
| `AgentConfigCard` | 当前 PromptProfile / Agent 配置摘要入口。 |
| `AgentProfileManager` | AgentRunConfig、tool whitelist、开关、校验错误。 |
| `PluginPanel` | 插件列表、启停、权限、错误。 |
| `PluginHostFrame` | iframe 插件宿主、slot mount、ready / error。 |
| `PluginSlotRenderer` | SidebarPanel、statusbar、hook host 等 slot。 |
| `PluginEventLog` | 事件 feed、订阅过滤。 |
| `PromptHookAuditLog` | hook 请求/响应脱敏记录、耗时、错误。 |
| `PipelineTracePanel` | Director / Subagent / Editor / Postprocess trace。 |
| `LogPanel` | app / frontend 日志、过滤、复制。 |

## 10. 状态清单（能力验收，不是皮肤）

重设计后每个业务区域仍应能表达这些状态，避免“好看但主流程跑不动”：

| 状态 | 需要出现的位置 |
| --- | --- |
| `empty` | 未导入角色卡、无 Campaign、无会话、无知识、无任务、无插件。 |
| `loading` | 导入、识别角色、加载 Campaign、Meta 分析、插件 ready、导出。 |
| `streaming` | 写作生成、Meta 对话、LLM 调用中。 |
| `partial success` | 真实卡部分降级、JS fallback 失败但主流程继续、prompt hook fail-open。 |
| `error` | 导入失败、存储失败、连接失败、插件异常、MVU apply 失败。 |
| `disabled` | 未选 Campaign、未配置连接、写作中禁止冲突操作。 |
| `dirty` | 表单修改未保存、变量编辑未提交、Agent Profile 未保存。 |
| `readonly` | 内置配置、归档记录、历史 trace。 |
| `danger` | 删除会话、删除变体、清空正则、禁用插件。 |

## 11. 出图 / 重设计优先级

### P0：主工作台（先画这些）

- `AppShell` / `TopBar` / `PrimarySidebar`
- `ConversationViewport` / `ChatMessage` / `StreamingMessage` / `ProcessReview` / `Composer`
- `Overlay` / `Dialog` / `Toast`
- 空状态与生成中状态

P0 要让人一眼觉得：**这是想打开的写作工具**，并能完成打开 Campaign、写作、停止、看消息、切历史。

### P1：Campaign 主线

- `CampaignPanel` / `CardLibrary` / `NewCampaignForm`
- instances / knowledge / tasks / summaries
- 导入导出入口

### P2：Meta、MVU、插件兼容

- `MetaPanel` / health / patch preview
- `MvuStatusBar`
- `PluginPanel` 与兼容降级提示

### P3：高级配置与诊断

- 连接、预设、Agent Profile
- pipeline trace、event log、hook audit、log panel  
  这些默认退后，不抢第一屏。

## 12. 设计交付画面（给 gpt-image / 设计稿）

至少覆盖：

1. **空白首次启动**：未导入、未配置连接、无 Campaign；有明确下一步。  
2. **Campaign 写作中**：流式生成、停止、主消息区为主、trace 入口弱化。  
3. **写作完成后**：消息、变体、重 roll、分支、轻量过程/质量回顾。  
4. **复杂卡**：多开场白、MVU 状态、兼容/降级提示不刺眼。  
5. **Campaign 管理**：卡列表、Campaign 列表、四类详情 tab。  
6. **Meta 助手**：health、patch preview、accept/dismiss。  
7. **插件/调试**（可次要）：事件与 hook 审计，偏高玩模式。  
8. **移动端**：主菜单抽屉 + 写作 + 管理入口。

出图约束：

- 桌面 16:10 或 16:9 高保真；移动端另出一帧。  
- **不要**复现当前仓库截图风格。  
- **不要**把调试信息做成主 UI。  
- 中文界面文案可用占位，但层次要真实（标题 / 正文 / 次要说明）。  
- 可参考“现代创作工具 / 长文编辑器 / 沉浸阅读”气质，不要游戏 HUD。

可直接粘贴的简要 prompt 骨架：

```text
Design a high-end desktop UI for StoryForge, a multi-agent interactive story writing app.
Campaign-first writing workbench: left navigation, center conversation/writing canvas as the hero,
optional right inspector collapsed by default. Beautiful empty state and streaming writing state.
Modern, calm, refined typography for long Chinese narrative text. Not an IDE, not a devops dashboard,
not neon cyberpunk. Do not copy any existing purple-glow debug prototype look.
Screens: (1) first-run empty (2) writing in progress (3) finished message with variants.
```

## 13. 工程对照（仅开发用，非视觉参考）

> 下列路径只说明**能力已经落在哪个文件**，方便以后换皮实现。  
> **视觉重设计请忽略这些文件的外观。**

当前运行入口：`frontend/src/main.js` → `AppV2.vue` + Pinia。  
组件根目录：`frontend/src/components-v2/`（shell / ui / writing / campaign / meta / st / config / debug）。

| 区域 | 代表文件 |
| --- | --- |
| Shell | `shell/AppShell.vue` `TopBar.vue` `PrimarySidebar.vue` `InspectorDrawer.vue` `PanelHost.vue` |
| Writing | `writing/ConversationViewport.vue` `ChatMessage.vue` `StreamingMessage.vue` `ProcessReview.vue` `Composer.vue` … |
| Campaign | `campaign/CampaignPanel.vue` 与 4 个 `*Tab.vue`、`CardLibrary.vue`、`NewCampaignForm.vue` |
| Meta / ST / Config / Debug | `meta/*` `st/*` `config/*` `debug/*` |
| 契约未迁入 v2 | `components/PluginHost.vue`、`components/MvuJsRuntime.vue`（宿主/runtime，不是视觉主角） |

（Phase 8 前端重构执行手册已不随仓库分发。）
- `npm run build`：422KB 产物。
- 契约红线：ChatMessage 8 emit、CampaignPanel `refreshActiveDetailTab`、MetaPanel `mvu-applied` + `lastConversationNode` 全部保留。

