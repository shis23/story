# StoryForge 前端交接文档：解耦架构与双轨计划（2026-07-22）

> 读者：**接手的前端/接线 agent**。本文是唯一入口。
> 配套：`docs/VISUAL-REDESIGN-2026-07-21.md`（视觉规格「纸上编辑部」）、
> `docs/FRONTEND-COMPONENTS.md`（能力地图，冲突时以能力为准）、
> `docs/效果预览/selected/`（视觉参考，仅此两张可用）。

---

## 1. 现状快照（已完成，勿返工）

| 区块 | 状态 | 位置 |
| --- | --- | --- |
| 设计 token（浅色纸面默认 + `.dark` 夜读） | ✅ | `frontend/src/style.css` |
| 主题切换默认浅色 | ✅ | `useTheme.js`、`index.html` |
| 24 个基础 UI 组件换肤 | ✅ | `frontend/src/components-v2/ui/*` |
| 壳：侧栏（桌面常驻/移动抽屉）+ TopBar | ✅ | `components-v2/shell/*` |
| 写作区/历史/概览 三屏换肤（旧结构） | ✅ | `components-v2/writing/*` |
| **新设计写作主屏（重绘·纯展示）** | ✅ 暂停于此 | `frontend/src/design/writing/`（预览 `http://localhost:1420/#design-writing`） |
| 全量测试基线 | ✅ | `npm test` 313 + `npm run test:ui` 28 全绿 |

**当前决策**：新前端设计暂停；先完成旧前端优化收尾（B2/B3/B4），保证全产品能用、统一观感；新前端后续按 §4 继续。

---

## 2. 解耦架构（三层，所有新代码必须遵守）

```text
┌─ frontend/src/design/            美术层（纯展示）
│    只准：props 进、events 出、fixtures 演示、内联 SVG、token 取色
│    禁止：import stores / composables / tauri-api / utils（lint 强制，见 §5）
│    每屏三件套：组件 + fixtures.js + CONTRACT.md（或 api.d.ts）
│
├─ frontend/src/adapter/           适配层（接线 agent 的工作区）
│    每屏一个 use<Screen>Adapter.js：
│      读 store 字段 → 组装 screenProps
│      组件 events → 调 composable handler
│    只准 import：stores、composables、design 组件
│
└─ stores / composables / tauri-api 功能层（现有，冻结约束见 §6）
```

**契约即 API**：美术层与功能层不见面，只通过该屏 `CONTRACT.md`（props↔store 映射、
events→handler 映射、红线、验收）协作。首个完整范例：`frontend/src/design/writing/CONTRACT.md`。

**预览通道**：`main.js` 按 hash 分流（`#design-writing` → fixture 演示页；其余 → 正式 AppV2）。
新屏预览沿用此模式加 hash 分支；正式切换后删除对应分支。

---

## 3. 计划 A：旧前端优化收尾（先能用，优先级最高）

> 目标：全产品无"旧工程原型观感"残留；不动结构只换肤；每批完成后
> `npm run build` + `npm test`(313) + `npm run test:ui`(28+) 全绿。
> 任务卡模板见 `docs/VISUAL-REDESIGN-2026-07-21.md` §E；每卡必须附：只改路径清单、
> 参考金样（`components-v2/writing/*` 或 `ui/*`）、禁止清单、四态+双主题验收。

### A1. Campaign 主线（B2）— 先壳后件，2 批

- **A1a（建议熟手/主模型做）**：`campaign/CampaignPanel.vue` 壳 + `campaign/NewCampaignForm.vue`
  - ⚠️ 契约红线：CampaignPanel 的 `refreshActiveDetailTab` 暴露方法不许动；NewCampaignForm 与
    `useNewCampaignForm` 的接线字段不许动。只改模板 class 与样式。
- **A1b（可派弱模型，2 卡并行）**：
  - 卡① `campaign/CampaignInstancesTab.vue` + `campaign/InstanceVariableEditor.vue`
  - 卡② `campaign/CampaignKnowledgeTab.vue` + `campaign/CampaignTasksTab.vue` +
        `campaign/CampaignSummariesTab.vue`
  - 参照金样：`ui/DataTable.vue`、`ui/DataList.vue`、`writing/ConversationHistoryList.vue`
- 卡③ `campaign/CardLibrary.vue` + `campaign/CardDetailPreview.vue` + `campaign/CampaignExportImportBar.vue`

### A2. Meta / 配置（B3）— 2 卡并行

- 卡④ `meta/MetaPanel.vue`（⚠️ `mvu-applied` + `lastConversationNode` 红线）+ `meta/MetaChat.vue`
- 卡⑤ `config/ConnectionConfigPanel.vue` + `config/PresetPanel.vue` +
      `config/PluginPanel.vue` + `config/AgentProfileManager.vue`
- 其余 `meta/*`（HealthCheckPanel/TypedPatchList/PatchPreview/GenerationExplanation/MvuAnalyzer）
  与 `st/*`（MvuStatusBar/StCompatibilityBadge 等）随卡④⑤附带换肤，或追加卡⑥。

### A3. 调试退后区（B4）— 1 卡

- 卡⑦ `shell/InspectorDrawer.vue` + `debug/*`（PipelineTracePanel/LogPanel/PluginEventLog/
  PromptHookAuditLog）+ `components-v2/st/MvuJsRuntimeHost.vue` 外观
- 原则：默认收起、低对比、mono 字号 12px，不追求精美，只去 glow/emoji/写死色。

### A4. 收尾走查

- 移动端窄屏主流程（导入→建 Campaign→写作→历史）人工走查。
- 深浅两主题逐屏过一遍；`grep -rn "glow\|🎬\|🎭\|📋\|✏️\|🔄" frontend/src/components-v2` 应基本无残留。

---

## 4. 计划 B：新前端重绘与切换（当前暂停，A 完成后或并行启动）

### B1. 继续重绘（美术层，`design/` 下每屏三件套）

| 顺序 | 屏 | 参考 |
| --- | --- | --- |
| ~~1~~ | ~~写作主屏~~ ✅ `design/writing/` | selected 图②③ |
| 2 | Campaign 管理（含 4 Tab） | selected 图④ |
| 3 | 会话历史 / 空态衍生 | selected 图① |
| 4 | Meta 助手 / 连接配置 | 蓝图 §8-9（无图，先补 inbox 图再画） |
| 5 | 壳（侧栏/TopBar/抽屉）v2 | selected 全图 |

### B2. 接线（适配层，接线 agent）

- 为每屏写 `adapter/use<Screen>Adapter.js`，按该屏 CONTRACT.md 映射；
  首个样例待写：`adapter/useWritingScreenAdapter.js`（照 `design/writing/CONTRACT.md`）。
- **接线必守**：8 个变体事件 payload 不变；`scrollToBottom` expose 签名不变；
  正文渲染换回 `components-v2/st/RichContent.vue`；删除走 tauri `ask` 确认；
  reroll 恢复三级菜单（整体/编剧/子Agent）。
- 切换策略：**逐屏替换** AppV2 中的旧组件（先写作区，再其余），每换一屏跑全量测试+人工走查；
  全切完后删 `main.js` 预览分支与旧 `components-v2/writing/*`。

---

## 5. 美术层 import 禁令（建议落地为 lint 规则）

`frontend/eslint` 或构建约束（可后补）：`src/design/**` 禁止 import
`stores/`、`composables/`、`tauri-api.js`、`plugin-bridge.js`、`utils/`、`components/`、
`components-v2/`（RichContent 除外——接线时由适配层以 slot/prop 注入，或 CONTRACT 显式豁免）。
临时人工检查：`grep -rn "from '../../stores\|from '../../composables\|tauri-api" frontend/src/design` 应无输出。

## 6. 冻结与红线（任何 agent 不得突破）

- 冻结文件：`tauri-api.js`、`plugin-bridge.js`、`utils/**`、`mvu-runtime-bridge.js`、
  `components/PluginHost.vue`、`components/MvuJsRuntime.vue`（协议与测试敏感结构）。
- 契约红线：ChatMessage 8 emit（见 `design/writing/CONTRACT.md` 表）、
  CampaignPanel `refreshActiveDetailTab`、MetaPanel `mvu-applied` + `lastConversationNode`。
- store 字段 / composable 签名不动；确需动时先写「字段兼容策略」并获确认。
- 视觉只准参考 `docs/效果预览/selected/`；`rejected/` 与旧 AppV2 截图禁止当规范。

## 7. 已知陷阱（前车之鉴，验收必查）

1. **z token**：Tailwind 不生成 `--z-*` 命名类，须写 `z-[var(--z-overlay)]`（规格 §B2）。
2. **headlessui 弹层动画**：禁裸用 `TransitionChild`（必须配 TransitionRoot）；
   自卸载面板（ListboxOptions/MenuItems）用 Vue 原生 `<Transition>` 包裹。
   happy-dom 测不出真实浏览器 transition 问题——**涉及弹层的卡必须 dev server 人工点一遍**。
3. **Select** 已重写为 headlessui Listbox，回归测试 `tests/components-v2/select.test.mjs`。
4. **不写死色值**：深浅主题只靠 token；`grep -rn "#[0-9a-fA-F]\{6\}" frontend/src/components-v2` 新增处应无。

## 8. 验收基线（每卡完成定义）

1. `cd frontend && npm run build` ✅
2. `npm test`（313）✅；`npm run test:ui`（28+）✅——样式断言随皮肤更新需注理由，逻辑断言不动
3. `git diff --stat` 只含任务卡允许的文件
4. 空/载/流/错四态 + 深浅双主题人工走查
5. 汇报格式：每文件改动点 ≤5 行 + 测试结果 + 是否动测试及原因
