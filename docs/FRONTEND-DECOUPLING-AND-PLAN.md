# StoryForge 前端交接文档：解耦架构与双轨计划（2026-07-22）

> 读者：**接手的前端/接线 agent**。本文是唯一入口。
> 配套：`docs/VISUAL-REDESIGN-2026-07-21.md`（视觉规格「纸上编辑部」）、
> `docs/FRONTEND-COMPONENTS.md`（能力地图，冲突时以能力为准）、
> `docs/效果预览/selected/`（视觉参考，仅此两张可用）。

---

## 1. 现状快照（2026-07-22 更新）

| 区块 | 状态 | 位置 |
| --- | --- | --- |
| 设计 token（浅色纸面默认 + `.dark` 夜读） | ✅ | `frontend/src/style.css` |
| 主题切换默认浅色 | ✅ | `useTheme.js`、`index.html` |
| 24 个基础 UI 组件换肤 | ✅ | `frontend/src/components-v2/ui/*` |
| 壳：侧栏 + TopBar（纸面 token） | ✅ | `components-v2/shell/*` |
| 旧 writing 三屏换肤 | ✅ 保留回退 | `components-v2/writing/*` |
| **写作主屏 design + 生产接线** | ✅ | `design/writing/*` + `adapter/useWritingScreenAdapter.js` → AppV2 |
| **会话历史 design + 生产接线** | ✅ | `design/history/*` + `adapter/useHistoryScreenAdapter.js` → AppV2 |
| **活动概览 design + 生产接线** | ✅ | `design/overview/*` + `adapter/useOverviewScreenAdapter.js` → AppV2 |
| **活动管理 design 壳生产外包** | ✅ | `CampaignPanel` 外包 `design/campaign/CampaignScreen` 全宽双栏；`#detail` 注入 Instances/Knowledge/Tasks/Summaries（含 MVU）；`refreshActiveDetailTab` 红线保留 |
| Meta 壳 design 生产外包 | ✅ | `MetaPanel` 用 `design/meta/MetaScreen` 作纸面壳；业务 tab 仍 v2；`mvu-applied` 红线保留 |
| AppFrame design 生产切换 | ✅ | `AppV2` 根壳已改为 `design/shell/AppFrame.vue`；Inspector 覆盖式；`#panels` 保留 PluginHost/MvuJsRuntime |
| 旧面板 emoji/调试台残留清理 | ✅ | campaign/meta/config/debug 按钮与标题去 emoji |
| 插件/卡内组件 | ✅ 未改坏 | `PluginHost` / `MvuJsRuntime` / `RichContent` / `MvuStatusBar` / `plugin-bridge` 冻结 |
| 全量测试基线 | ✅ | `npm test` 317 + `npm run test:ui` 28 + `npm run build` |

**当前决策**：

1. 主写作与历史已切到 **design + adapter** 生产路径。
2. Campaign 深度管理（实例变量、MVU 条、角色提取、export/import 全路径）仍走已换肤的 `CampaignPanel`，避免破坏 Meta refresh 与卡内组件。
3. 新屏预览：`#design-writing`、`#design-campaign`。

---

## 2. 解耦架构（三层，所有新代码必须遵守）

```text
┌─ frontend/src/design/            美术层（纯展示）
│    只准：props 进、events 出、fixtures 演示、内联 SVG、token 取色
│    禁止：import stores / composables / tauri-api / utils
│    每屏三件套：组件 + fixtures/demo + CONTRACT.md
│
├─ frontend/src/adapter/           适配层
│    useWritingScreenAdapter.js    ✅ 生产
│    useHistoryScreenAdapter.js    ✅ 生产
│    useOverviewScreenAdapter.js   ✅ 生产
│    useCampaignScreenAdapter.js   ✅ 脚手架（预览/后续切换）
│    useMetaScreenAdapter.js       ✅ 脚手架（预览/后续切换）
│
└─ stores / composables / tauri-api 功能层（冻结约束见 §6）
```

**契约即 API**：美术层与功能层不见面，只通过该屏 `CONTRACT.md` 协作。

**预览通道**：`main.js` hash 分流（`#design-writing` / `#design-campaign`）。

---

## 3. 计划 A：旧前端优化收尾 — **已完成**

- A1 Campaign 面板：token 化 + LoadingState 补 import；保留能力结构。
- A2 Meta / 配置：去 emoji 标题与按钮；纸面 token。
- A3 调试区：LogPanel 图标 SVG 化；低对比 mono 风格保持。
- A4 残留扫描：业务 UI 无 glow/紫/emoji 按钮；注释内 emoji 可保留。

---

## 4. 计划 B：新前端重绘与切换

| 顺序 | 屏 | 状态 |
| --- | --- | --- |
| 1 | 写作主屏 | ✅ 重绘 + 生产接线 |
| 2 | 会话历史 | ✅ 重绘 + 生产接线 |
| 3 | 活动概览 | ✅ 重绘 + 生产接线 |
| 4 | Campaign 管理 | ✅ CampaignPanel 外包 CampaignScreen；深度 Tab slot 注入 |
| 5 | Meta 助手壳 | ✅ MetaPanel 已外包 MetaScreen；业务 tab 仍 v2 |
| 6 | AppFrame 壳 | ✅ 生产切换完成（AppV2 根） |

### 接线必守（写作已满足）

- 8 个变体事件 payload 不变。
- `scrollToBottom` expose 签名不变。
- 正文渲染注入 `RichContent`。
- 删除走 tauri `ask` 确认。
- reroll 三级菜单（整体/编剧/子Agent）。

### 后续可选

- 将 CampaignPanel 深度编辑（变量/MVU）以 slot 注入 `CampaignScreen` 后整屏切换。
- MetaPanel 外包 `MetaScreen` 壳，不改子组件 props。
- ~~AppShell → AppFrame~~：已完成；`#panels` 仍挂 PluginHost/MvuJsRuntime。

### 人工/自动化走查（2026-07-22）

| 项 | 证据 |
| --- | --- |
| AppFrame 生产挂载 | `AppV2.vue` 根组件 `AppFrame`；`tests/components-v2/app-frame-walkthrough.test.mjs` |
| 深浅主题切换 | 侧栏「夜读模式」toggle；走查测试断言 `html.dark` + localStorage |
| 移动端菜单 | TopBar 菜单按钮 → `ui.showSidebar`；走查测试窄屏 390 |
| Inspector 覆盖式 | 不挤压主栏；走查测试打开调试后写作空态仍在 |
| 连接弹层 | 侧栏「连接」打开配置面板 |
| 插件 runtime | `list_plugins` 仍被调用；`PluginHost`/`MvuJsRuntime` 仍挂在 `#panels` |
| 单元/UI/构建 | `npm test` 317；`npm run test:ui` 35；`npm run build` ✅ |

---

## 5. 美术层 import 禁令

`src/design/**` 禁止 import `stores/`、`composables/`、`tauri-api.js`、`plugin-bridge.js`、`utils/`、`components/`、`components-v2/`。

临时检查：

```bash
rg -n "from ['\"].*(stores|composables|tauri-api)" frontend/src/design
```

应无输出。

---

## 6. 冻结与红线

- 冻结文件：`tauri-api.js`、`plugin-bridge.js`、`utils/**`、`mvu-runtime-bridge.js`、
  `components/PluginHost.vue`、`components/MvuJsRuntime.vue`。
- 契约红线：ChatMessage/MessageItem 8 emit、CampaignPanel `refreshActiveDetailTab`、
  MetaPanel `mvu-applied` + `lastConversationNode`。
- store 字段 / composable 签名不动。
- 视觉只准参考 `docs/效果预览/selected/`。

---

## 7. 已知陷阱

1. **z token**：须写 `z-[var(--z-overlay)]`。
2. **headlessui 弹层**：弹层相关改动必须 dev server 人工点。
3. **AppShell 滚动**：主区 `overflow-hidden` + 子屏自滚动；overview/history 外包 `overflow-y-auto`。
4. **不写死色值**：只用 token。

---

## 8. 验收基线

1. `cd frontend && npm run build` ✅
2. `npm test`（317）✅；`npm run test:ui`（28）✅
3. 主写作路径可走通（design 屏）
4. 插件 host / MVU runtime 仍挂载于 AppV2 `#panels`
5. 深浅两主题人工走查
