# StoryForge 前端组件蓝图

> 更新日期：2026-07-08
> 目的：给下一版前端重设计提供组件地图。本文按“用户界面需要什么组件”组织，不要求沿用当前视觉样式；现有实现路径仅用于帮助定位业务能力。
>
> **本文已落地**：Phase 8 前端重构已完成，60 个 v2 组件按本文 P0/P1/P2/P3 分区全部实现（详见末尾“实现状态映射”节与 `docs/FRONTEND-REBUILD-2026-07-08.md`）。下面正文保留原始设计蓝图作为意图参考，实现状态以末尾映射表为准。

## 设计目标

StoryForge 的第一屏应该是可用的写作工作台，而不是介绍页。新版前端建议保持“中间写作、左右管理/诊断”的工作流，但把当前散落在页面里的按钮、列表、状态和弹层收敛为稳定组件。

核心体验优先级：

1. 写作不被管理功能打断：主写作区始终清晰，管理面板用抽屉/分栏承载。
2. Campaign 是主线：角色卡、实例、知识、任务、摘要、MVU 都围绕 active Campaign 组织。
3. 插件/ST 兼容要可见但不喧宾夺主：普通用户看到降级提示，高玩模式看到事件、hook、trace。
4. 移动端保留主流程：导入、建 Campaign、写作、查看结果、导出，其他复杂功能可降级为抽屉。

## 信息架构

### 主应用框架

- `AppShell`：应用根布局，负责全屏高度、主区域、移动/桌面断点。
- `TopBar`：当前标题、写作状态、菜单按钮、调试入口。
- `PrimarySidebar`：主导航，包含写作、素材、配置、Meta、导入入口。
- `InspectorDrawer`：右侧调试/高玩抽屉，展示 pipeline、插件事件、prompt hook 审计。
- `WorkspaceView`：中间工作区容器，在写作、会话历史、Campaign 概览之间切换。
- `PanelHost`：统一承载左抽屉、右抽屉、居中弹窗和嵌入式管理面板。

### 主要视图

- 写作视图：消息流、开场白选择、流式生成、底部输入框。
- 会话历史：会话列表、删除、打开、分支入口。
- Campaign 概览：当前 Campaign 摘要、故事时间、快捷操作。
- Campaign 管理：角色卡、Campaign 列表、详情子页、导入导出。
- Meta 助手：健康检查、修复建议、MVU 分析、生成解释。
- 配置中心：连接、预设、Agent Profile、插件。

## 基础组件

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

## 应用框架组件

| 组件 | 当前来源 | 新版职责 |
| --- | --- | --- |
| `AppShell` | `App.vue` | 根布局、视图路由、全局弹层状态。 |
| `TopBar` | `App.vue` header | 当前页面标题、模式副标题、生成状态、菜单/调试入口。 |
| `PrimarySidebar` | `AppSidebar.vue` | 主导航、当前上下文摘要、主题切换、power mode。 |
| `MobileNavigationDrawer` | `BaseOverlay + AppSidebar` | 移动端主菜单，点击后回到主工作区。 |
| `InspectorDrawer` | `DebugDrawer.vue` | pipeline、连接状态、插件事件、prompt hook 审计。 |
| `PanelHost` | 多处 `BaseOverlay` | 统一弹层尺寸、标题、返回、关闭、滚动策略。 |

新版可以保留“左导航 + 中写作 + 右调试”的心智模型，但桌面端建议允许左栏常驻，移动端用抽屉。

## 写作组件

| 组件 | 当前来源 | 新版职责 |
| --- | --- | --- |
| `ConversationViewport` | `App.vue` 消息区 | 承载空状态、开场白选择、消息列表、流式消息。 |
| `GreetingSelector` | `App.vue` 内联按钮 | 多开场白选择，支持横向滚动和当前选中。 |
| `ChatMessage` | `ChatMessage.vue` | 单条用户/助手消息，展示 active variant、操作菜单、provenance 摘要。 |
| `MessageVariantSwitcher` | `ChatMessage.vue` 内部 | 上/下一版、版本计数、采纳、删除、分支。 |
| `MessageActionMenu` | `ChatMessage.vue` 内部 | 重 roll、编辑、添加变体、分支、删除。 |
| `RichContent` | `RichContent.vue` | 安全展示 display-only HTML、Markdown/纯文本降级。 |
| `StreamingMessage` | `StreamingMessage.vue` | Director/Subagent/Editor 流式过程，支持折叠。 |
| `Composer` | `Composer.vue` | 写作意图输入、开始/停止、禁用提示、快捷键。 |
| `ConversationHistoryList` | `App.vue` history view | 会话列表、删除、打开、空状态。 |
| `CampaignOverview` | `App.vue` overview view | 当前 Campaign 快捷入口、故事时间、历史数量。 |

## Campaign 与角色卡组件

| 组件 | 当前来源 | 新版职责 |
| --- | --- | --- |
| `CampaignPanel` | `CampaignPanel.vue` | Campaign 管理总入口，负责 tab 和局部刷新。 |
| `CardLibrary` | `CampaignPanel.vue` cards tab | 已导入角色卡列表、抽取状态、重新识别。 |
| `CardDetailPreview` | `CampaignPanel.vue` 内联详情 | 卡名、tags、alternate greetings、extensions 保真提示。 |
| `CharacterListPanel` | `CharacterList.vue` | legacy/兼容入口的角色卡列表。 |
| `CharacterDetailPanel` | `CharacterDetail.vue` | 角色详情、世界书 entry 编辑、路由更新。 |
| `CampaignList` | `CampaignPanel.vue` campaigns tab | 某张卡下的 Campaign 列表、设为 active、打开详情。 |
| `NewCampaignForm` | `CampaignPanel.vue` 和 `App.vue` | 选卡、选开场白、命名、创建并开始。 |
| `CampaignDetailTabs` | `CampaignPanel.vue` detail tab | instances / knowledge / tasks / summaries 分页。 |
| `InstancesTab` | `CampaignInstancesTab.vue` | 实例列表、临时实例升格、变量编辑。 |
| `InstanceVariableEditor` | `CampaignInstancesTab.vue` 内部 | bool/number/json/text 变量编辑。 |
| `KnowledgeTab` | `CampaignKnowledgeTab.vue` | 知识列表、来源、传话链、private/封口标记。 |
| `TasksTab` | `CampaignTasksTab.vue` | 任务列表、状态变更、完成/放弃。 |
| `SummariesTab` | `CampaignSummariesTab.vue` | 轮次摘要、时间线、关联节点。 |
| `CampaignExportImportBar` | `CampaignPanel.vue` | StoryForge bundle、ST 卡、共享 lorebook 导入导出状态。 |

## MVU、ST 与富内容组件

| 组件 | 当前来源 | 新版职责 |
| --- | --- | --- |
| `MvuStatusBar` | `MvuStatusBar.vue` | 变量状态条、文本、tag、icon 渲染。 |
| `MvuStatusBlock` | `MvuStatusBar.vue` 内部 | 单个变量块，支持 bar/text/tag/icon/default。 |
| `MvuJsRuntimeHost` | `MvuJsRuntime.vue` | 隐藏 JS runtime 容器，展示降级/错误时可接到 Inspector。 |
| `MvuSchemaPreview` | `MetaPanel.vue` | schema 新增/覆盖/无变化 preview。 |
| `MvuApplyResult` | `MetaPanel.vue` | apply 后刷新提示、失败提示、关联 Campaign tab。 |
| `StCompatibilityBadge` | 散落文案 | regex、HTML、TavernHelper、Slash、prompt hook 的支持/降级提示。 |
| `RegexScriptSummary` | `PresetPanel.vue` / 文档能力 | 来源、placement、promptOnly/markdownOnly/minDepth/maxDepth 摘要。 |
| `DisplayContentBoundary` | `RichContent.vue` | 明确 raw content 与 display content 的展示边界。 |

## Meta Agent 组件

| 组件 | 当前来源 | 新版职责 |
| --- | --- | --- |
| `MetaPanel` | `MetaPanel.vue` | Meta 助手总入口，承载聊天、健康检查、patch、MVU、解释。 |
| `MetaChat` | `MetaPanel.vue` | 用户与 Meta Agent 对话，支持流式和工具结果折叠。 |
| `MetaToolResultCard` | `MetaPanel.vue` + `metaToolResults.js` | 世界书报告、卡片报告、patch proposal 等结构化展示。 |
| `HealthCheckPanel` | `MetaPanel.vue` | Campaign health issue 列表、修复建议入口。 |
| `TypedPatchList` | `MetaPanel.vue` | typed patch 列表、状态、展开。 |
| `PatchPreview` | `MetaPanel.vue` | preview diff、影响范围、accept/dismiss。 |
| `GenerationExplanation` | `MetaPanel.vue` | 当前生成的 trace/provenance 解释。 |
| `MvuAnalyzer` | `MetaPanel.vue` | 选择卡、分析 MVU、查看翻译列表。 |

## 配置、插件与调试组件

| 组件 | 当前来源 | 新版职责 |
| --- | --- | --- |
| `ConnectionConfigPanel` | `ConnectionConfig.vue` | LLM/embedder 配置、SecretRef 状态、测试连接。 |
| `PresetPanel` | `PresetPanel.vue` | prompt preset、regex scripts、active preset。 |
| `AgentConfigCard` | `AgentConfigCard.vue` | 当前 PromptProfile/Agent 配置摘要入口。 |
| `AgentProfileManager` | `AgentProfileManager.vue` | AgentRunConfig 管理、tool whitelist、开关、校验错误。 |
| `PluginPanel` | `PluginPanel.vue` | 插件列表、启停、权限、错误。 |
| `PluginHostFrame` | `PluginHost.vue` | iframe 插件宿主、slot mount、ready/error。 |
| `PluginSlotRenderer` | `App.vue` / `DebugDrawer.vue` | SidebarPanel、statusbar、hook host 等 slot 展示。 |
| `PluginEventLog` | `DebugDrawer.vue` | 事件 feed、订阅过滤、最近 100 条。 |
| `PromptHookAuditLog` | `DebugDrawer.vue` + `promptHookAudit.js` | hook 请求/响应脱敏记录、耗时、错误、payload hash。 |
| `PipelineTracePanel` | `DebugDrawer.vue` | Director/Subagent/Editor/Postprocess trace。 |
| `LogPanel` | `LogPanel.vue` | app/frontend 日志、过滤、复制。 |

## 状态组件清单

每个业务组件都应该明确覆盖这些状态，避免重设计后“好看但验收跑不动”。

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

## 重设计优先级

### P0：主工作台

- `AppShell`
- `TopBar`
- `PrimarySidebar`
- `ConversationViewport`
- `ChatMessage`
- `StreamingMessage`
- `Composer`
- `Overlay/Dialog/Toast`

P0 完成后，用户应该能导入/打开 Campaign、写作、停止、查看消息、切换历史。

### P1：Campaign 主线

- `CampaignPanel`
- `CardLibrary`
- `NewCampaignForm`
- `CampaignDetailTabs`
- `InstancesTab`
- `KnowledgeTab`
- `TasksTab`
- `SummariesTab`
- `CampaignExportImportBar`

P1 完成后，验收矩阵里的 Bronze/Silver 主路径才有稳定 UI。

### P2：Meta、MVU、插件兼容

- `MetaPanel`
- `HealthCheckPanel`
- `PatchPreview`
- `MvuStatusBar`
- `MvuSchemaPreview`
- `PluginPanel`
- `PromptHookAuditLog`
- `PluginEventLog`

P2 完成后，可以支撑复杂 ST/MVU 卡和插件兼容验收。

### P3：高级配置与诊断

- `ConnectionConfigPanel`
- `PresetPanel`
- `AgentProfileManager`
- `PipelineTracePanel`
- `LogPanel`
- `RegexScriptSummary`

P3 面向 power user 和发布排障，不应该阻塞普通写作第一屏。

## 建议的目录结构

```text
frontend/src/components/
  shell/
    AppShell.vue
    TopBar.vue
    PrimarySidebar.vue
    InspectorDrawer.vue
  ui/
    Button.vue
    IconButton.vue
    Input.vue
    Textarea.vue
    Select.vue
    SegmentedControl.vue
    Tabs.vue
    Badge.vue
    Overlay.vue
    Dialog.vue
    Toast.vue
    EmptyState.vue
    ErrorState.vue
    DataList.vue
    DataTable.vue
    DiffView.vue
  writing/
    ConversationViewport.vue
    ChatMessage.vue
    MessageActionMenu.vue
    MessageVariantSwitcher.vue
    StreamingMessage.vue
    Composer.vue
    ConversationHistoryList.vue
  campaign/
    CampaignPanel.vue
    CardLibrary.vue
    CardDetailPreview.vue
    NewCampaignForm.vue
    CampaignDetailTabs.vue
    InstancesTab.vue
    KnowledgeTab.vue
    TasksTab.vue
    SummariesTab.vue
  meta/
    MetaPanel.vue
    MetaChat.vue
    HealthCheckPanel.vue
    PatchPreview.vue
    GenerationExplanation.vue
    MvuAnalyzer.vue
  st/
    MvuStatusBar.vue
    MvuSchemaPreview.vue
    StCompatibilityBadge.vue
    RichContent.vue
  config/
    ConnectionConfigPanel.vue
    PresetPanel.vue
    AgentProfileManager.vue
    PluginPanel.vue
  debug/
    PipelineTracePanel.vue
    PluginEventLog.vue
    PromptHookAuditLog.vue
    LogPanel.vue
```

## 设计交付检查

重设计稿至少覆盖这些画面：

- 空白首次启动：未导入、未配置连接、无 Campaign。
- Campaign 写作中：流式生成、停止、trace 入口。
- 写作完成后：消息、变体、重 roll、分支、postprocess 状态。
- 真实复杂卡：多开场白、世界书、MVU 状态栏、regex/HTML 降级提示。
- Campaign 管理：卡列表、Campaign 列表、instances/knowledge/tasks/summaries。
- Meta 助手：health issue、patch preview、accept/dismiss、MVU apply。
- 插件/调试：事件 log、prompt hook audit、插件错误。
- 移动端：主菜单抽屉、写作、Campaign 管理、导出入口。

---

## 实现状态映射（Phase 8 落地，2026-07-08）

> 下面是蓝图组件到实际 v2 文件的映射。实际结构在 `frontend/src/components-v2/`，60 个 `.vue` 文件按 `shell/ui/writing/campaign/meta/st/config/debug` 分区。命名规则：蓝图里带 `Panel`/`Tab`/`List`/`Bar`/`Card` 后缀的概念，v2 落地时多数保留原名；带 `Frame`/`Host`/`Renderer` 的运行时承载类组件保留原位（契约依赖），不迁入 `components-v2/`。

### 已实现为独立 v2 组件（47 个）

| 蓝图组件 | v2 文件 | 备注 |
| --- | --- | --- |
| `AppShell` | `shell/AppShell.vue` | 三栏根布局 |
| `TopBar` | `shell/TopBar.vue` | 消费 uiStore.pageTitle |
| `PrimarySidebar` | `shell/PrimarySidebar.vue` | 消费 uiStore 视图切换 |
| `InspectorDrawer` | `shell/InspectorDrawer.vue` | 右调试抽屉，Tabs 承载 4 debug 组件 |
| `PanelHost` | `shell/PanelHost.vue` | 统一弹层承载 |
| `Button`/`IconButton`/`Input`/`Textarea`/`Select` | `ui/{Button,IconButton,Input,Textarea,Select}.vue` | |
| `SegmentedControl`/`Checkbox`/`Toggle`/`Slider` | `ui/{SegmentedControl,Checkbox,Toggle,Slider}.vue` | `NumberInput` 复用 `Input` |
| `Tabs`/`Menu`/`Tooltip` | `ui/{Tabs,Menu,Tooltip}.vue` | Headless UI 承载 |
| `Badge`/`Progress`/`Toast` | `ui/{Badge,Progress,Toast}.vue` | |
| `Overlay`/`Dialog` | `ui/{Overlay,Dialog}.vue` | Headless UI 承载 |
| `EmptyState`/`ErrorState`/`LoadingState` | `ui/{EmptyState,ErrorState,LoadingState}.vue` | |
| `DataList`/`DataTable`/`DiffView`/`CodeBlock` | `ui/{DataList,DataTable,DiffView,CodeBlock}.vue` | |
| `ConversationViewport` | `writing/ConversationViewport.vue` | defineExpose scrollToBottom |
| `GreetingSelector` | `writing/GreetingSelector.vue` | |
| `ChatMessage` | `writing/ChatMessage.vue` | **契约保留：8 emit + message shape** |
| `StreamingMessage` | `writing/StreamingMessage.vue` | 消费 writingStore.pipeline |
| `Composer` | `writing/Composer.vue` | |
| `ConversationHistoryList` | `writing/ConversationHistoryList.vue` | |
| `CampaignOverview` | `writing/CampaignOverview.vue` | |
| `CampaignPanel` | `campaign/CampaignPanel.vue` | **契约保留：refreshActiveDetailTab expose** |
| `CardLibrary` | `campaign/CardLibrary.vue` | |
| `NewCampaignForm` | `campaign/NewCampaignForm.vue` | 消费 useNewCampaignForm |
| `InstancesTab`/`KnowledgeTab`/`TasksTab`/`SummariesTab` | `campaign/Campaign{Instances,Knowledge,Tasks,Summaries}Tab.vue` | 4 个独立 tab |
| `MetaPanel` | `meta/MetaPanel.vue` | **契约保留：lastConversationNode + mvu-applied emit** |
| `MetaChat`/`HealthCheckPanel`/`PatchPreview` | `meta/{MetaChat,HealthCheckPanel,PatchPreview}.vue` | |
| `GenerationExplanation`/`MvuAnalyzer` | `meta/{GenerationExplanation,MvuAnalyzer}.vue` | |
| `MvuStatusBar` | `st/MvuStatusBar.vue` | 复用 `utils/mvuStatusBarModel.js` |
| `RichContent` | `st/RichContent.vue` | 复用 `utils/formatContent.js` |
| `StCompatibilityBadge` | `st/StCompatibilityBadge.vue` | |
| `ConnectionConfigPanel`/`PresetPanel`/`AgentProfileManager`/`PluginPanel` | `config/{ConnectionConfigPanel,PresetPanel,AgentProfileManager,PluginPanel}.vue` | |
| `PipelineTracePanel`/`PluginEventLog`/`PromptHookAuditLog`/`LogPanel` | `debug/{PipelineTracePanel,PluginEventLog,PromptHookAuditLog,LogPanel}.vue` | 复用 `utils/pipelineTrace.js`、`promptHookAudit.js` |

### 内联实现（蓝图概念在 v2 里内联到父组件，未单独建文件，9 项）

| 蓝图组件 | 内联位置 | 原因 |
| --- | --- | --- |
| `MessageVariantSwitcher`/`MessageActionMenu` | `writing/ChatMessage.vue` | 8 emit 契约要求单一组件持有，拆分增加 prop 传递复杂度 |
| `InstanceVariableEditor` | `campaign/CampaignInstancesTab.vue` | 变量类型分支多但只在该 tab 用 |
| `CampaignExportImportBar` | `campaign/CampaignPanel.vue` | |
| `MvuStatusBlock` | `st/MvuStatusBar.vue` | 单变量块只服务状态栏 |
| `MvuSchemaPreview`/`MvuApplyResult`/`MetaToolResultCard`/`TypedPatchList` | `meta/MetaPanel.vue` | Meta 内聚，拆分会跨组件传 patch DTO |
| `RegexScriptSummary` | `config/PresetPanel.vue` | |
| `DisplayContentBoundary` | `st/RichContent.vue` | |

### 保留原位未迁入 v2（契约依赖，3 项）

| 蓝图组件 | 原位文件 | 原因 |
| --- | --- | --- |
| `PluginHostFrame`/`PluginSlotRenderer` | `components/PluginHost.vue` | 半脆弱测试 `tests/plugin-host-slots.test.mjs` 正则读 `<script>` 段 + 6 个 slot helper 导出 |
| `MvuJsRuntimeHost` | `components/MvuJsRuntime.vue` | 半脆弱测试 `tests/mvu-runtime-bridge.test.mjs` 正则读 `SHIM_SCRIPT` 标记 |
| `CharacterListPanel` | `components/CharacterList.vue` | AppV2 仍引用作角色选择列表 |

### 未实现 / 蓝图未要求单独建件（2 项）

| 蓝图组件 | 状态 | 说明 |
| --- | --- | --- |
| `MobileNavigationDrawer` | 由 `PrimarySidebar` + AppShell 断点响应承载 | 桌面常驻左栏，移动端用 Overlay 抽屉模式 |
| `CampaignDetailTabs` | 由 `CampaignPanel` 内联 `ui/Tabs` 承载 | 不单独建件 |
| `CardDetailPreview` | 由 `CardLibrary` 选中态承载 | |
| `CampaignList` | 由 `CardLibrary` 卡片展开承载 | |
| `CharacterDetailPanel` | **删除** | 重构后无对应入口，角色详情由 CharacterList + Campaign tabs 承载 |

### 状态层与逻辑层（非组件，配套落地）

- **Pinia stores**（`frontend/src/stores/`）：`campaign.js` / `writing.js` / `plugin.js` / `ui.js` + `index.js` 工厂。
- **Composables**（`frontend/src/composables/`）：`useWriting` / `usePipeline` / `usePluginBridge` / `useConversation` / `useMessageVariants` / `useGreeting` / `useCharacterImport` / `useNewCampaignForm`，共 8 个。
- **纯 util**（`frontend/src/utils/`）：新增 `forkCampaignName.js` / `roleLabel.js` / `consoleForwarding.js`；既有 17 个 util 全部冻结签名未改。
- **根组装**：`frontend/src/AppV2.vue` 串起 shell + writing + campaign + meta + st + config + debug，`main.js` 挂载 `AppV2 + createPinia()`。

### 验证

- `node --test`：212 pass（含 16 个半脆弱测试：plugin-host-slots 6 + mvu-runtime-bridge 10）。
- `vitest`：21 pass（ui 库组件挂载测试，`tests/components-v2/`）。
- `npm run build`：422KB 产物。
- 契约红线：ChatMessage 8 emit、CampaignPanel `refreshActiveDetailTab`、MetaPanel `mvu-applied` + `lastConversationNode` 全部保留。

