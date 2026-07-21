# StoryForge 视觉重设计规格「纸上编辑部」（2026-07-21）

> 状态：**已拍板，可执行**。本文件是视觉与呈现层重设计的唯一规范。
> 能力地图见 `docs/FRONTEND-COMPONENTS.md`（能力冲突时以能力为准，改视觉表达）。
> 视觉参考仅限 `docs/效果预览/selected/` 两张；`rejected/` 与现有 AppV2 截图禁止当规范。

## 0. 拍板决策

| 项 | 决定 |
| --- | --- |
| 主方向 | **A 纸上编辑部**：暖米白纸感 + 金棕（赭石）点缀 + 中文衬线正文 |
| 主题 | **浅色纸面默认** + `.dark` 夜读深色变体（暖深棕黑，非冷黑） |
| 排印 | ~~正文衬线~~ → **2026-07-21 改判：全无衬线**（Inter + 系统中文字体）；长文靠行高 1.85 + 限宽保障可读性；mono 仅日志/数据 |
| 装饰 | **纯排版 + 内联 SVG 符号**；不引入插画图片资产 |
| 参考图 | `selected/20260721-board-desktop-v1-editorial-paper.png`（主）+ `…-v2-minimal-ink.png`（辅） |

## A. 视觉重设计规格（可执行）

### A1. 气质原则

1. 第一眼是**愿意打开的写作台**：纸面、墨、细线、留白；不是 IDE、运维台、游戏 HUD。
2. **写作区是唯一主角**；管理/调试/trace 一律退后（抽屉、收起、弱化）。
3. 无发光（glow）、无玻璃拟态滥用、无渐变堆料；层次靠 **1px 细线 + 克制落影 + 字重**。
4. 中文长文优先：正文衬线、行高 1.9、栏宽限宽；UI 元信息无衬线、字号降档。
5. 空状态、加载态、流式态与完成态同等精修。

### A2. 布局骨架（桌面）

| 区域 | 规格 |
| --- | --- |
| `PrimarySidebar` | 常驻，宽 **232px**；纸面底（`bg`），右侧 1px `line`；含 Logo、新建 Campaign 主按钮、导航、当前 Campaign 摘要、底部主题/电源入口 |
| `TopBar` | 高 **52px**；面包屑（活动 / Campaign 名）+ 写作状态徽章 + 右侧 Inspector 开关与菜单；`surface` 底 + 底部 1px `line` |
| 写作栏 | 居中，内容最大宽 **720px**（≈68ch），左右留白 ≥48px；消息间距 28px |
| `InspectorDrawer` | 右侧 **320px**，**默认收起**；TopBar 入口展开，覆盖式（overlay）而非挤压写作栏 |
| 移动端（<768px） | 侧栏与 Inspector 全部抽屉化；TopBar 40px 精简；写作栏全宽、padding 16px |

### A3. 材质与层次

- 背景：`bg` 纸面；卡片/面板：`surface` 纸卡 + 1px `line` 描边；凹陷区（输入框、代码块）：`surface-2`。
- 投影仅三档：`shadow-card`（列表卡）、`shadow-rise`（悬停浮起）、`shadow-float`（抽屉/弹窗）。
- 旧 `shadow-glow-*` 语义**重定义为 1px 状态描边**（token 层已改，组件 class 不动）。
- 圆角克制：控件 6px（`rounded-md`），卡片 10px（`rounded-lg`），弹层 16px（`rounded-xl`），徽章/头像全圆。

### A4. 排版刻度

| 用途 | 字体 | 字号/行高 |
| --- | --- | --- |
| 展示标题（空状态/Campaign 名） | sans 600 | 22–28px / 1.4 |
| 区块标题（章节、面板题） | sans 600 | 15–18px / 1.5 |
| **写作正文** | sans 400（`.prose-fiction`） | 15.5–16px / **1.85**，字距 0.01em |
| UI 正文/按钮 | sans 400/500 | 13–14px / 1.55 |
| 元信息（时间、字数、来源） | sans 400，`ink-soft` | 12px / 1.5 |
| 数据/日志/trace | mono 400 | 12px / 1.6 |

### A5. 状态表达（视觉语义）

| 状态 | 表达 |
| --- | --- |
| streaming | 正文末尾细竖线光标（accent，1.5px 宽，1.1s 呼吸）；TopBar 状态徽章 `running`；Composer 变「停止」 |
| loading | 骨架屏用 `surface-2` 呼吸块，不用旋转大 spinner 占屏 |
| empty | 衬线大标题 + 一句副文案 + ≤3 个入口卡；禁止堆调试信息 |
| error | `err` 细描边卡片 + 重试按钮；不弹窗打断写作 |
| disabled | `ink-faint` 文字 + 无 hover；写作中冲突操作禁用并给 tooltip 原因 |
| dirty | 标题旁 accent 圆点（6px），保存后消失 |
| danger | `err` 文字按钮，二次确认走 `Dialog` |
| partial/降级 | `warn` 细徽章，一行说明，可展开细节 |

### A6. 动效

- 统一缓动 `--ease-soft`；微交互 120–160ms，面板/抽屉 200–240ms，弹层 240ms。
- 列表进入用既有 `.sf-stagger`；流式文本**不做**逐字位移动画，只出光标。
- 遵循 `prefers-reduced-motion`（已有）。

## B. 设计 Token 方案（已落地于 `frontend/src/style.css`）

> 策略：**沿用既有 token 名，整体换值**——60+ 组件 class 零改动即换肤；新增半径/层级 token。旧外观（紫、glow）不保留。

### B1. 色彩（浅色默认 / `.dark` 夜读）

| token | 浅色（纸） | 夜读（暖深棕） | 用途 |
| --- | --- | --- | --- |
| `bg` | `#f6f3ec` | `#1b1712` | 纸面 |
| `surface` | `#fffdf7` | `#241f18` | 纸卡/面板 |
| `surface-2` | `#eee9dd` | `#2e2820` | 凹陷/输入/骨架 |
| `line` | `#e2dccc` | `#3a3328` | 1px 分隔线 |
| `ink` | `#29241c` | `#ece4d3` | 主文字 |
| `ink-soft` | `#6e6759` | `#a89e8a` | 次要文字 |
| `ink-faint` | `#a49c8a` | `#6f6757` | 占位/禁用 |
| `accent` | `#9a6425` | `#d2a55e` | 金棕主强调 |
| `accent-bright` | `#7d4f1a` | `#e4bd7f` | hover/按下 |
| `accent-soft` | 10% 透明 | 13% 透明 | 选中底/引用块 |
| `accent-border` | 34% 透明 | 38% 透明 | 强调描边/焦点环 |
| `ok` | `#3e7c4f` | `#7fae85` | 完成/健康 |
| `running` | `#4a6d9c` | `#8aa5c6` | 生成中 |
| `warn` | `#a87b22` | `#cfa055` | 降级/警告 |
| `err`/`error` | `#b0432d` | `#d2745f` | 错误/危险 |
| `wait` | `#99917e` | `#6f6757` | 排队/闲置 |

### B2. 形状 / 投影 / 层级 / 动效

| 类别 | token | 值 |
| --- | --- | --- |
| 圆角 | `radius-xs/sm/md/lg/xl` | 2 / 4 / 6 / 10 / 16px |
| 投影 | `shadow-card` / `shadow-rise` / `shadow-float` | 见 style.css（低透明暖墨落影，无 glow） |
| 状态描边 | `shadow-glow-accent/ok/err`（旧名留用） | `0 0 0 1px <状态色 40%±>` |
| 层级 | `z-overlay/drawer/dialog/toast/tooltip` | 40 / 50 / 60 / 70 / 80（**用法**：Tailwind 不生成 `--z-*` 命名工具类，须写 `z-[var(--z-overlay)]` 任意值形式） |
| 缓动 | `--ease-soft` | `cubic-bezier(0.22,1,0.36,1)` |
| 断点 | `breakpoint-xs` | 30rem（既有，保留） |

### B3. 字体资产

沿用本地 @fontsource：Inter（UI + 正文）、JetBrains Mono（数据/日志）。Noto Serif SC 已随改判移除。不新增依赖。

## C. P0 文件级计划

### P0-a Token + 基础 UI（本轮已做 token；基础 UI 待做）

| 文件 | 动作 |
| --- | --- |
| `frontend/src/style.css` | ✅ 已重写 token（浅色默认 + 夜读 `.dark`） |
| `frontend/src/useTheme.js` | ✅ 默认 `light`，theme-color 换纸色 |
| `frontend/index.html` | ✅ 内联脚本默认浅色 |
| `components-v2/ui/Button.vue` `IconButton.vue` `Badge.vue` `Input.vue` `Textarea.vue` `Select.vue` `SegmentedControl.vue` `Menu.vue` `Tooltip.vue` | 换肤：仅改模板 class/样式块，**props/emits 不动** |
| `components-v2/ui/Overlay.vue` `Dialog.vue` `Toast.vue` `EmptyState.vue` `LoadingState.vue` `Progress.vue` | 换肤 + 按 A3/A5 材质与状态规格 |

### P0-b 三金样屏（写作主角）

| 金样 | 涉及文件 | 要点 |
| --- | --- | --- |
| ① empty | `shell/AppShell.vue`、`shell/PrimarySidebar.vue`、`shell/TopBar.vue`、`writing/ConversationViewport.vue`（空态）、`ui/EmptyState.vue` | 衬线大标题 + 三入口：导入角色卡 / 新建 Campaign / 打开会话历史；入口走既有 `useCharacterImport` / `useNewCampaignForm` / `useConversation` |
| ② writing（streaming） | `writing/ConversationViewport.vue`、`writing/StreamingMessage.vue`、`writing/Composer.vue`、`shell/TopBar.vue` | 正文衬线流式 + 细光标；Composer 停止态；TopBar `running` 徽章；Inspector 默认收起 |
| ③ written（变体+回顾） | `writing/ChatMessage.vue`、`writing/ProcessReview.vue`、`writing/GreetingSelector.vue` | 变体横排卡片（行为仍走 `useMessageVariants` 8 handler，**ChatMessage 8 emit 不动**）；ProcessReview 默认收起为一行摘要，展开为时间线 |

边界：`shell/PanelHost.vue`、`shell/InspectorDrawer.vue` 只换壳材质；**P1 之后**才动 campaign/meta/config/debug 内部。

### 验收（P0 完成定义）

1. 三金样在浅色与夜读下均成立；空/载/流/错/disabled/danger 六态可演示。
2. 全部交互仍走既有 store（`writing/campaign/ui/plugin`）与 composables；无伪 API。
3. 契约红线通过：ChatMessage 8 emit、`refreshActiveDetailTab`、`mvu-applied` + `lastConversationNode`。
4. `npm run build` 通过。

## D. 图 → 组件对照表

> 源图：`selected/20260721-board-desktop-v1-editorial-paper.png`（主）、`…-v2-minimal-ink.png`（辅）。图中非蓝图能力（模板中心/智能体广场/世界观设定等）**不落地**。

| 图中区域 | 落为组件 | 备注 |
| --- | --- | --- |
| 左栏 Logo +「新建活动」+ 导航 + 今日创作统计 + 底部用户卡 | `PrimarySidebar` | 「新建活动」→ 打开 `NewCampaignForm`；统计区数据从 campaign store 取，无数据时隐藏 |
| 顶部「活动 / 风起之地·第一卷」+ 自动保存时间 + 右侧图标 | `TopBar` | 状态徽章映射 writing store；右起：Inspector 开关、菜单 |
| 空态「开始你的故事」+ 三入口卡 | `ConversationViewport` 空态 + `EmptyState` | 入口改词：导入角色卡 / 新建 Campaign / 会话历史 |
| 写作区章节标题 + 衬线正文 + 流式光标 +「AI 正在生成」「停止生成」 | `ConversationViewport` + `ChatMessage` + `RichContent`(`.prose-fiction`) + `StreamingMessage` + `Composer` | 光标与停止态见 A5 |
| 右侧「创作助手」面板（当前智能体/思考中/记忆参考/调试 token） | `InspectorDrawer` | **默认收起**；token/耗时等调试信息放抽屉深处 |
| 底部输入框 + 快捷指令 + 圆形发送钮 | `Composer` | 快捷指令（续写/扩写…）若无后端对应则不做，仅保留输入/开始/停止 |
| 变体 A/B/C 横排卡 + 采纳勾选 | `ChatMessage` 变体区（`MessageVariantSwitcher` 行为） | 视觉卡片化；切换/采纳/删除/分支仍走 8 个 emit |
| 「创作过程回顾」时间线 | `ProcessReview` | 默认收起一行（耗时+步骤数），点击展开时间线 |
| Campaign 管理：左活动列表 + 右 tabs（实例/知识/任务/总结）+ 角色卡网格 + 知识概览 | `CampaignPanel` + `CampaignList` + `CampaignDetailTabs` + 4×`*Tab.vue` + `CardLibrary` | P1 范围，本阶段只保证壳兼容 |

## E. 下游任务卡模板（给弱模型填组件）

```markdown
# 任务卡：<组件名> 换肤（纸上编辑部）

## 只改这些路径
- frontend/src/components-v2/<目录>/<组件名>.vue   ← 仅此文件（模板 class + <style>）
## 必须参考
- 视觉：docs/效果预览/selected/20260721-board-desktop-v1-editorial-paper.png 的 <区域>
- 规范：docs/VISUAL-REDESIGN-2026-07-21.md §A3/A4/A5、§B token 表
- 金样：frontend/src/components-v2/writing/<金样组件>.vue（已完成者）
## 禁止
- 禁止改：tauri-api.js、plugin-bridge.js、utils/**、mvu-runtime-bridge.js、
  components/PluginHost.vue、components/MvuJsRuntime.vue、stores/**、composables/**
- 禁止改本组件的 props / emits / 事件名 / store 调用
- 禁止引入新依赖、新图片资产、glow/渐变/玻璃拟态
- 禁止使用 rejected/ 或旧 AppV2 截图当参考
## 样式约束
- 只用 style.css 已有 token（bg/surface/surface-2/line/ink*/accent*/ok/running/warn/err、
  shadow-card/rise/float、radius-*、z-*）；不得写死色值
- 正文类长文本用 .prose-fiction；UI 用 sans；数字日志用 mono
## 验收
1. 空 / 加载 / 流式 / 错误 四态截图或手动走查通过（深浅两主题）
2. 交互仍走既有 store/composable；事件契约不变
3. `npm run build` 通过；无新 console 警告
```

## 附：待补效果图清单（生成后入 `inbox/`）

`2026072x-writing-desktop-v3-stop-error`（停止/错误态）、`…-written-desktop-v2-processreview`（回顾展开）、`…-campaign-desktop-v2-tabs`（四 tab 细节）、`…-meta-desktop-v1-patch`（health+patch preview）、`…-empty-mobile-v1`、`…-writing-mobile-v1`、`…-writing-desktop-v1-nightread`（夜读深色）。
