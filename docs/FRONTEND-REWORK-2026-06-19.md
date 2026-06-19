# 前端重做工作记录（2026-06-19）

> 本文档记录 StoryForge 前端从"原基础上修"到"重做骨架 + 设计语言"再到"一 Campaign 一对话模型"的完整工作过程。
> 范围：前端视觉/骨架/交互重做 + 配套后端数据模型调整。
> 状态：进行中（部分待验证）。

## 背景

起点：前端经多轮功能开发后积累了系统性问题——22 个组件三套弹层、三套下拉、三套对话框混用；硬编码颜色脱离主题；触控区普遍 16-20px；写作区与调试区视觉不分；单列窄柱 + 顶栏堆 10+ 按钮导致拥挤。用户要求"重做一个牛逼的软件前端"。

## 工作阶段

### 阶段 1：共享原语 + 主题修复（基础设施）

先建地基，后续组件迁移到原语上。

**新建 `frontend/src/components/base/`**：
- `BaseOverlay.vue` — 统一弹层。支持 `position`（drawer/center/left/right）、`size`、`bodyScroll`、Teleport + Transition + ESC + click-outside + body 滚动锁。后期加 left/right 侧滑动画。
- `BaseDialog.js` — 统一对话框。`confirmDialog`/`alertDialog`（包 Tauri plugin-dialog）+ `promptDialog`（自实现输入弹层，Tauri 无原生 prompt）。替换所有 `alert()`/`window.confirm`/`window.prompt`。
- `BaseDropdown.vue` — 统一下拉，click-outside + ESC + Transition。
- `BaseButton.vue` — 统一按钮，移动端 ≥44px 触控。
- `useClickOutside.js` — composable，pointerdown + capture。

**`style.css` 修复真 bug**：
- 加 `--color-error: var(--color-err)` 别名（修 MetaPanel/MvuStatusBar 的 `text-error` 笔误失效）
- 注册 `--breakpoint-xs: 30rem`（修 `hidden xs:inline` 永不生效）

**`MvuJsRuntime.vue` 安全修复**：
- `sandbox` 去掉 `allow-same-origin`（沙箱逃逸反模式），只保留 `allow-scripts`。验证 shim 仅靠 postMessage 通信不依赖同源。
- 加执行超时兜底（卡脚本卡死时回传 error，避免 Rust 永久等待）。

### 阶段 2：组件迁移到原语（一致性清扫）

22 个组件逐一迁移到 BaseOverlay/BaseDialog，消除三套实现：

- 7 个弹层迁移 BaseOverlay：CharacterList、CharacterDetail（消除双层滚动）、ConnectionConfig（补 backdrop-blur + loading 骨架）、CampaignPanel、PresetPanel（嵌套滚动单层化）、MetaPanel（三层滚动收敛 + 两个嵌套浮层改 BaseOverlay）、PluginPanel。
- 所有 `alert()`/`window.confirm`/`window.prompt` → BaseDialog（AgentConfigCard、AgentProfileManager、CampaignInstancesTab、CampaignTasksTab、CampaignPanel、PresetPanel、PluginPanel、CharacterDetail、App.vue 等）。
- 硬编码颜色 → 主题 token：
  - PluginPanel 整组件（white/zinc/red/emerald/amber/blue/purple 13 处）重写进主题
  - LogPanel 日志级别 5 色 → err/warn/running/ink-soft token
  - CharacterDetail 路由 select 蓝/绿/紫/灰 → accent/ok/running/ink-soft
  - App.vue/AppHeader/ConnectionConfig 硬编码绿 → ok token
- 触控区放大：所有 `px-1.5 py-0.5 text-[10px]`（16-20px）→ `min-h-[44px]` 或 `min-h-[36px]`。
- 文案统一：`加载中...`（ASCII 三点）→ `加载中…`（省略号字符）；中英混排文案统一中文。

### 阶段 3：设计语言重建（现代质感）

用户反馈"仍很廉价，是在原基础上改不是重做"。校正方向：重建设计语言，不只换 token。

**方向**：现代质感——深色为主、柔和发光强调色、玻璃模糊层、微动效、精致字体。锚点 Arc/Raycast + Linear/Notion + VSCode。

**`style.css` 重写**：
- 深色为默认基调（中性深灰带冷紫，非纯黑）：bg `#0a0a0f`、surface `#131319`、surface-2 `#1b1b23`
- 强调色紫 + 发光：accent `#8b7cf6`，加 `--shadow-glow-accent` 等发光阴影 token
- 玻璃层工具类 `.glass` / `.glass-strong`（backdrop-blur + 半透明）
- 动效系统：统一缓动 `--ease-soft`、列表 stagger（`sf-stagger`）、hover 微抬、状态脉冲、focus-visible 发光环
- 浅色模式保留（`:root:not(.dark)` 覆盖），默认深色（useTheme + index.html 默认 dark）
- 滚动条从藏死改为细而克制

**字体本地化**（@fontsource，离线一致）：
- 装 `@fontsource/inter`（UI）、`@fontsource/noto-serif-sc`（正文衬线）、`@fontsource/jetbrains-mono`（日志/seed）
- 之前字体只声明未加载，回退系统字体——修为真加载

**组件形态**：卡片 `bg-surface rounded-xl shadow-card` 无粗边框；按钮去边框改底色+发光；导航项 active 指示条；消息去气泡阅读器化。

### 阶段 4：骨架重排（三栏 → 顶栏触发弹出）

最初做三栏常驻（左导航 + 中写作 + 右调试），但用户窗口 480px，右调试区 `hidden lg:flex` 在窄窗永不显示且无触发按钮，根本看不到。校正为：

**左右侧栏改顶栏触发弹出，不常驻**：
- 顶栏左上角 ☰ → 左导航从左侧滑出
- 顶栏右上角 🛠 → 右调试区从右侧滑出
- 中间写作区永远占满
- BaseOverlay 加 `position="left"/"right"` 侧滑支持（translateX 动画）
- 去掉高玩概念：🛠 总显示，右调试区总是可弹（不再需要先开 powerMode）

**左导航 `AppSidebar.vue`**（新建）：
- 分组：写作（新建 Campaign / 会话历史）、素材（Campaign 管理 / 角色卡 / 导入）、配置（连接 / 预设 / 插件 / Meta 助手）
- 底部主题切换 + 当前上下文摘要
- active 指示条 + glass hover

**右调试区 `DebugDrawer.vue`**（改造）：
- Agent 配置 / Profile / 日志 / 插件侧栏 四 tab
- 从主滚动流抽出，桌面常驻改为抽屉

**废弃 `AppHeader.vue`**：顶栏内联到 App.vue，逻辑迁 AppSidebar。

### 阶段 5：写作过程流式在对话区

用户反馈"看不到 agent 输出"，要把 agent 流式过程显示在对话区最后一条消息。

**去掉 PipelinePanel**（用户不知它是什么，过程全在对话区）。

**新建 `StreamingMessage.vue`**：
- Director 折叠块（可展开看流式 delta）
- 各子 Agent 折叠块（进度 + done 后可展开看 full_text）
- Editor 底部逐字流式（衬线正文 + 打字光标）
- 全在对话区最后一条消息位置

**App.vue handlePipelineEvent 改造**：
- 废弃 `editor-streaming` 占位消息机制（8 处引用清理）
- Editor 逐字流式改由 StreamingMessage 读 `pipeline.editor.output` 渲染
- 写作完成（startWriting/handleReroll 成功后）`showPipeline=false` 收起 StreamingMessage

**停止生成按钮移到发送键**：
- Composer 发送键在写作中变红色停止键（同一槽位），点停止生成
- 去掉对话区顶部独立"停止生成"横条

**user 消息可编辑 + 重roll**：
- 后端已支持（`edit_variant` 不限 role）
- ChatMessage 给 user 消息加"编辑"按钮，保存调 editVariant 持久化，重 roll 用新内容

**会话删除功能**：
- 后端新增 `delete_conversation` 命令（调 conv_store.delete）
- 前端 tauri-api.js 加 `deleteConversation`，会话历史列表每项加删除按钮

### 阶段 6：一 Campaign 一对话模型

用户指出"一个 Campaign 下多个会话"逻辑不对，应是：导入卡 → 识别角色 → 建 Campaign → 开对话。交流后定为**一 Campaign 一对话**（Campaign 是存档边界，INTENT.md 第 25 行）。

**后端数据模型**：
- `Conversation` 加 `campaign_id: Option<Id>`（`crates/domain/src/conversation.rs`）
- `Campaign` 加 `conversation_id: Option<Id>`（`crates/domain/src/campaign.rs`），双向绑定
- `ConversationSummary` 加 `campaign_id` + `card_name`（前端列表显示角色卡名）

**后端流程**：
- `create_campaign`（`crates/tauri-app/src/lib.rs`）自动建对话 + 存开场白 + 绑定 conversation_id。开场白从 CharacterStore 按 source_character_id 查 first_mes（之前从内存 tool_ctx 找，启动时空 → 开场白没存的 bug 修复）。
- `start_writing` Campaign 模式自动用 active_campaign 的 conversation_id（不再前端传/新建）。
- `list_conversations` 返回 `card_name` + `campaign_id`（联查角色卡名）。
- 新增 `delete_card` 命令（按 CharacterCard.id 删，级联 campaign/instances/mvu）——修角色卡删除失效（之前传 source_character_id 给 delete_character，id 不匹配）。

**前端**：
- 左导航"新对话"→"新建 Campaign"，点开弹表单（选卡 + 起名），建完自动开对话 + 进写作
- 会话历史列表不显示会话 id，显示**角色卡名 + 创建时间**
- 点会话历史项 → 切到该 Campaign（setActiveCampaign）+ 加载对话
- CharacterList 改读 `listCards`（cards.json，与 Campaign 管理同源），不再读空的 characters.json

**清理旧数据**：旧会话 + 旧 Campaign 全清，角色卡/连接/预设/向量保留。

**测试适配**：Conversation/conv_store.create 加 campaign_id 参数，全 workspace 测试调用点适配（app-conversation/app-pipeline/app-meta/harness-real-llm 测试），全部通过。

### 阶段 7：子 Agent 工具循环优化

日志暴露：子 Agent 跑了 10 轮，每轮"未调工具，注入 reminder"。

**根因**（`crates/app-agent/src/runtime.rs`）：子 Agent 注册了 `get_character` 工具，但本职是输出表演（不调工具直接输出内容）。工具循环的 drift recovery 把"直接输出内容"当成"该调工具却忘了"，每轮注入 reminder 拖到 max rounds（10 轮）才停。

**修复**：子 Agent 调 `run_tool_loop_with_layout` 时传 completion_probe——输出内容达 50 字符即视为完成（之前传 None）。子 Agent 从 10 轮降到 1-2 轮，省 8 次 LLM 调用。

## 待办 / 已知问题

- **"大框套小框"**：用户反馈主界面有大框套小框，尚未定位（我无法看画面，需用户指明具体位置）。Composer 已去内层框，但可能别处还有。
- **CharacterDetail 字段空**：CharacterList 改读 cards.json 后，选中卡弹 CharacterDetail 的扁平字段（first_mes/personality/world_info 等）为空（CharacterCard 不存这些，只存 character_definitions）。彻底统一需改造 CharacterDetail 读 CharacterCard 体系。
- **后处理 drift**：后处理 Agent 也有"第 2 轮未调工具"reminder（2 轮完成，浪费较小，未优化）。
- **Campaign 概览视图**：currentView='overview' 仍有残留代码，入口已基本去掉（左导航无触发），待清理。
- **legacy 单卡写作路径**：代码保留（writingMode='legacy'），但一 Campaign 一对话后 legacy 是否还需保留待定。

## 改动文件清单

**新建**：
- `frontend/src/components/base/`（BaseOverlay/BaseDialog/BaseDropdown/BaseButton/useClickOutside）
- `frontend/src/components/AppSidebar.vue`
- `frontend/src/components/DebugDrawer.vue`
- `frontend/src/components/StreamingMessage.vue`

**删除**：
- `frontend/src/components/AppHeader.vue`

**前端重写/大改**：
- `frontend/src/App.vue`（三栏骨架 + 流式 + Campaign 流程）
- `frontend/src/style.css`（设计语言重建）
- `frontend/src/components/Composer.vue`（单框 + 停止键）
- `frontend/src/components/ChatMessage.vue`（阅读器化 + user 编辑）
- 其余 16 个组件（迁移 BaseOverlay/BaseDialog + token + 触控）

**后端**：
- `crates/domain/src/conversation.rs`（Conversation 加 campaign_id）
- `crates/domain/src/campaign.rs`（Campaign 加 conversation_id）
- `crates/app-conversation/src/lib.rs`（conv_store.create 加 campaign_id + find_by_campaign）
- `crates/tauri-app/src/lib.rs`（create_campaign 自动建对话 + start_writing 用 campaign conversation_id + list_conversations 返回 card_name + delete_conversation + delete_card 命令）
- `crates/app-agent/src/runtime.rs`（子 Agent completion_probe）
- 各 crate 测试适配新签名

## 验证

- `cargo test --workspace`：全绿（184+ tests）
- `npm run build`：0 error
- 写作流程端到端跑通（日志确认：识别→导演→子Agent→编剧→后处理）
- 待用户画面验证：开场白加载、角色卡删除、子 Agent 提速、大框套小框定位
