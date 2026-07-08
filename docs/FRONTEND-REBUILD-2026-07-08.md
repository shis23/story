# StoryForge 前端重构执行手册(Phase 8)

> 创建日期:2026-07-08
> 目的:给执行者(人或低级模型)提供一份照着做就能完成的、精确到文件和字段的前端重构手册。
> 决策定稿见本文件 §0。当前执行状态见文末「执行记录」。

---

## 0. 决策定稿(不可更改)

| 维度 | 选择 |
|---|---|
| 策略 | **全新重写并存**。新代码放 `frontend/src/components-v2/` + `frontend/src/stores/` + `frontend/src/composables/`,复用现有 `frontend/src/utils/` + `frontend/src/tauri-api.js` + `frontend/src/plugin-bridge.js`。新代码跑通后改 `main.js` 挂载点一次性切换,旧代码逐个删。 |
| 状态管理 | **全面引入 Pinia**。App.vue 的 ~60 个 ref 按域抽成 4 个 store。 |
| 基础组件库 | **`@headlessui/vue`(交互复杂件)+ 手写(展示件)**。 |
| 视觉 | **精炼现有深色**。复用现有 `style.css` 的 `@theme` 令牌系统(深色 `#0a0a0f` 底 + 紫强调 `#8b7cf6`/`#a594ff`),不另建 design-tokens。 |
| 切换方式 | 新代码并行存在,`AppV2.vue` 跑通后改 `main.js`,旧代码阶段 9 删。 |

---

## 1. 契约红线(执行全程只 import 不修改,否则断后端或断 141 个回归测试)

### 1.1 冻结文件清单(禁止修改导出/签名/字符串字面量)

| 文件 | 冻结内容 | 断了会怎样 |
|---|---|---|
| `frontend/src/tauri-api.js` | 全部 export 函数名、所有 `invoke('命令名')` 字符串(~110 个)、`listen('mvu:load_card_assets'\|'mvu:unload_card'\|'mvu:execute')` 3 事件名 | 断前后端契约,所有后端调用失败 |
| `frontend/src/plugin-bridge.js` | 全部 export + `API_METHODS` 的 `command`/`permission` 字段、`ST_EVENT_TYPES` 全部事件名常量、6 个 `MSG_*` 协议常量、`DEFAULT_PLUGIN_HOOK_TIMEOUT_MS`/`PROMPT_HOOK_PERMISSION`/`READ_MEMORY_PERMISSION` | 断插件系统,57 个 plugin-bridge 测试失败 |
| `frontend/src/utils/*.js`(17 个) | 每个文件的导出函数名和签名。清单见 §1.2 | 断 141 个回归测试的护城河 |
| `frontend/src/mvu-runtime-bridge.js` | 全部 export | 断 10 个 mvu-runtime-bridge 测试 |
| `frontend/src/components/base/documentKeydownController.js` | 导出接口 | 断 3 个 base-keydown 测试 |
| `frontend/src/useTheme.js` | 顶层 `const theme = ref(...)` 单例、`useTheme()` 返回的 `{ theme, toggle }` | 断主题切换,所有调用方共享同一 ref |
| `frontend/src/style.css` 的 `@theme` 块 | 已定义的全部 `--color-*`/`--font-*`/`--shadow-*`/`--ease-soft` 令牌名(见 §2.3) | 断所有用这些令牌的组件 |

### 1.2 utils/ 17 个文件的导出清单(冻结签名)

执行时新组件可以 import 这些函数,但**不能改它们**:

```
utils/agentProfileConfig.js      — agent profile 配置校验
utils/campaignCardStatus.js      — campaignCardOptionSuffix / preferredCampaignCard
utils/campaignDisplay.js         — routingText / formatDiffValue / inferVarType 等
utils/campaignGreetingOptions.js — buildGreetingOptionsFromDetail
utils/campaignMvuStatusBar.js    — campaign MVU 状态条
utils/campaignTabRefresh.js      — DETAIL_SUB_TABS / SUB_TAB_REFS / subTabRefKey / refreshSubTab
utils/conversationNodes.js       — findLastAssistantConversationNode
utils/formatContent.js          — formatContent / shouldRenderHtmlDisplay 等(render-content 测试覆盖)
utils/metaPanelFlow.js          — meta 面板流程
utils/metaToolResults.js        — meta 工具结果
utils/mvuStatusBarModel.js      — mvuBarPercent / mvuBarColor / getMvuValue
utils/pipelineTrace.js          — subagentRolesFromProvenance
utils/promptHookAudit.js        — exportPromptHookAudit / parsePromptHookAuditExport
utils/promptHooks.js            — appendPromptHookAuditRecord / emitPromptHookEventAndWaitForPlugins / resolveHookedIntent / resolveHookedMessages / summarizePromptHookPayload / promptHookChangedKeys
utils/regexPlacement.js         — 正则 placement
utils/taskStatus.js             — 任务状态
```

> **校验命令**:`cd frontend && npm test`。如果 141 个旧测试里有任何一个失败,说明你动了冻结文件,必须回退。

### 1.3 必须保持的组件接缝(重写对应组件时签名不变)

| 组件 | 契约 | 来源 |
|---|---|---|
| `ChatMessage` | 8 个 emit:`reroll` / `reroll-user` / `switch-variant` / `edit-variant` / `accept-variant` / `delete-variant` / `add-variant` / `branch`。props:`message`(必含 `variants[]`/`active_variant`/`display_content`/`provenance`)、`conversationId`、`busy`、`canBranch`。 | App.vue:1326-1341 |
| `CampaignPanel` | `defineExpose({ refreshActiveDetailTab })`。被 App.vue 通过 `campaignPanelRef` 跨层调用(MVU apply 后刷新)。emits:`close`、`campaign-changed`。 | App.vue:1406 |
| `MetaPanel` | prop `lastConversationNode` shape `{ conversation_id, node_id }`。emits:`close`、`mvu-applied`(触发 CampaignPanel 刷新链)。 | App.vue:1407 |

### 1.4 半脆弱测试(重写对应组件时主动盯)

| 测试文件 | 用例数 | 依赖什么 | 重写时注意 |
|---|---|---|---|
| `tests/plugin-host-slots.test.mjs` | 6 | 正则读 `PluginHost.vue` 的 `<script>...</script>` 段,抽取 slot helper | 新 PluginHost.vue 要保留可被正则匹配的 `<script>` 段 + slot 辅助函数 |
| `tests/mvu-runtime-bridge.test.mjs` | 10 | 正则读 `MvuJsRuntime.vue` 的 `` const SHIM_SCRIPT = `...` `` 标记(8822 字符) | 新 MvuJsRuntime.vue 要保留这个精确标记,或同步更新测试 |

> **校验时机**:每次重写 PluginHost / MvuJsRuntime 后立即 `npm test`,断则当场修。

---

## 2. 现有资源(执行时复用,不重复造)

### 2.1 tauri-api.js 的导出(App.vue 已用的,新 composables 照样 import)

写作流水线相关(从 App.vue:18 的 import 语句摘出):
```
importCharacter, getCharacter, getVersion, startWriting, cancelWriting,
pluginPromptHookResult, regenerate, getActiveConnection, editVariant,
acceptVariant, softDeleteVariant(死导入,别用), deleteMessageFrom, addVariant,
switchVariant, listConversations, deleteConversation, getConversation,
logAppendFrontend, getActiveCampaign, listCards, getCard, createCampaign,
forkCampaign, setActiveCampaign, listInstances, listPlugins, extractCharacters
```

> 全部命令经 `tauri-api.js` 包装,新代码只 import 这些具名函数,**不直接调 invoke**。

### 2.2 plugin-bridge.js 的关键导出

```
ST_EVENT_TYPES              — 30 个事件名常量,广播插件事件时用
canModifyPrompt(plugin)     — prompt hook 权限门
canReadMemory(plugin)       — memory 权限门
```

> App.vue:19 只 import 了 `ST_EVENT_TYPES`。新代码按需 import。

### 2.3 style.css 的 @theme 令牌(新组件直接用 Tailwind 工具类)

Tailwind v4 的 `@theme` 块定义的就是工具类命名空间。例如 `--color-bg` → `bg-bg`,`--color-accent` → `text-accent`/`bg-accent`/`border-accent`,`--shadow-glow-accent` → `shadow-glow-accent`。

**深色模式令牌(默认):**
| 令牌 | 值 | Tailwind 类 |
|---|---|---|
| `--color-bg` | `#0a0a0f` | `bg-bg` |
| `--color-surface` | `#131319` | `bg-surface` |
| `--color-surface-2` | `#1b1b23` | `bg-surface-2` |
| `--color-line` | `#26262f` | `border-line` |
| `--color-ink` | `#ececf1` | `text-ink` |
| `--color-ink-soft` | `#8a8a98` | `text-ink-soft` |
| `--color-ink-faint` | `#55556a` | `text-ink-faint` |
| `--color-accent` | `#8b7cf6` | `text-accent`/`bg-accent`/`border-accent` |
| `--color-accent-bright` | `#a594ff` | `text-accent-bright` |
| `--color-ok` | `#34d399` | `text-ok` |
| `--color-running` | `#60a5fa` | `text-running` |
| `--color-warn` | `#fbbf24` | `text-warn` |
| `--color-err` | `#f87171` | `text-err` |
| `--shadow-glow-accent` | (见 css) | `shadow-glow-accent` |
| `--shadow-float` | (见 css) | `shadow-float` |
| `--shadow-card` | (见 css) | `shadow-card` |
| `--font-sans` | Inter | `font-sans` |
| `--font-serif` | Noto Serif SC | `font-serif` |
| `--font-mono` | JetBrains Mono | `font-mono` |

**工具类:**
- `.glass` / `.glass-strong` — 玻璃层
- `.prose-fiction` — 文学正文(衬线)
- `.sf-stagger > *` — 列表错位淡入(配合 `style="--i: N"`)
- `:focus-visible` — accent 发光环(全局已设)

> **浅色模式**已自动覆盖(`:root:not(.dark)` 块),新组件无需关心,只要用令牌类即可。

---

## 3. 阶段分解(每阶段独立可验证、可提交)

### 阶段 0:基础设施

**产出物:**
1. 本文档(`docs/FRONTEND-REBUILD-2026-07-08.md`)— 已存在,你正在读。
2. `npm install pinia @headlessui/vue @vue/test-utils happy-dom --save`(pinia/headlessui 进 dependencies;test-utils/happy-dom 进 devDependencies)
3. 新建目录(用 `.gitkeep` 占位):
   - `frontend/src/components-v2/{shell,ui,writing,campaign,meta,st,config,debug}/`
   - `frontend/src/stores/`
   - `frontend/src/composables/`
   - `frontend/tests/stores/`
4. `frontend/src/stores/index.js` — `createPinia()` 工厂
5. 不新建 design-tokens.css — **复用现有 `style.css` 的 `@theme`**(见 §2.3)

**验证:**
```bash
cd frontend
npm test                    # 仍 157 通过(没动任何旧代码)
npm run build               # 成功(还没新入口,不影响)
```

**提交信息:** `chore: scaffold frontend rebuild v2 (pinia, headless ui, dirs)`

---

### 阶段 1:Pinia stores(状态层)

**目标:** 把 App.vue 的 ~60 个 ref 按域抽成 4 个 store。store 只持有状态 + getter,**不放 action 逻辑**(那是 composables 的职责)。

#### stores/campaign.js — useCampaignStore

```js
import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { findLastAssistantConversationNode } from '../utils/conversationNodes.js'

export const useCampaignStore = defineStore('campaign', () => {
  const activeChar = ref(null)              // App.vue:31
  const activeCharDetail = ref(null)        // App.vue:32
  const activeCampaign = ref(null)          // App.vue:37
  const instanceNameMap = ref({})           // App.vue:41
  const conversationHistory = ref([])       // App.vue:335
  const currentConversationId = ref(null)   // App.vue:322

  // getter(App.vue:323-325)
  const lastConversationNode = computed(() =>
    findLastAssistantConversationNode(/* messages 来自 writingStore,需跨 store 引用 */)
  )
  // 注意:lastConversationNode 依赖 messages,实现见 composables 阶段(跨 store 组合)

  return {
    activeChar, activeCharDetail, activeCampaign, instanceNameMap,
    conversationHistory, currentConversationId, lastConversationNode,
  }
})
```

> **跨 store 依赖**:lastConversationNode 需要 messages(writingStore)。两种做法:(a) 放 writingStore;(b) 放 campaignStore 但在 composable 里组装。推荐 (a),见下。

#### stores/writing.js — useWritingStore

```js
import { defineStore } from 'pinia'
import { ref, reactive, computed } from 'vue'
import { findLastAssistantConversationNode } from '../utils/conversationNodes.js'

export const useWritingStore = defineStore('writing', () => {
  const messages = ref([])                  // App.vue:26
  const isWriting = ref(false)              // App.vue:320
  const showPipeline = ref(false)           // App.vue:27
  const activeConnection = ref(null)        // App.vue:342
  const selectedGreetingIndex = ref(0)      // App.vue:326

  // pipeline 状态机(App.vue:285-292)— 结构冻结
  const pipeline = reactive({
    state: 'idle',
    stateLabel: '',
    director: { status: 'idle', detail: '', output: '' },
    subagents: [],
    editor: { status: 'idle', detail: '', output: '' },
    postprocess: { status: 'idle', detail: '', knowledge: 0, variable: 0, task: 0, reason: '' },
  })

  // getters
  const writingMode = computed(() => {       // App.vue:307-311
    // 注意:依赖 activeCampaign/activeChar(campaignStore),需跨 store
    // 实现时用 useCampaignStore() 引用
  })

  return { messages, isWriting, showPipeline, activeConnection,
           selectedGreetingIndex, pipeline, writingMode }
})
```

> **跨 store getter**(writingMode / lastConversationNode / streamingRoleLabel):在 store 内部 `const campaign = useCampaignStore()` 引用另一个 store 的 state。Pinia 支持这种用法。

#### stores/plugin.js — usePluginStore

```js
import { defineStore } from 'pinia'
import { ref } from 'vue'

export const usePluginStore = defineStore('plugin', () => {
  const sidebarPlugins = ref([])            // App.vue:102
  const hookPlugins = ref([])               // App.vue:103
  const hookPluginSlots = ref({})           // App.vue:105
  const pluginPipelineEvents = ref([])      // App.vue:106
  const promptHookAuditRecords = ref([])    // App.vue:107

  // 非响应式(模块级 let / Map)
  // hookPluginHostRefs: Map — 放 store 外的模块级变量,或 store 内普通变量
  // pluginPipelineEventSeq: number — 同上

  return { sidebarPlugins, hookPlugins, hookPluginSlots,
           pluginPipelineEvents, promptHookAuditRecords }
})
```

> **注意**:`hookPluginHostRefs`(Map)和 `pluginPipelineEventSeq`(序号)不是响应式的,放 store 里作为普通变量返回即可,或放 composable 模块级。`MAX_PLUGIN_PIPELINE_EVENTS=100`/`MAX_PROMPT_HOOK_AUDIT_RECORDS=100` 是常量,放 store 顶部或单独 constants 文件。
> **死状态**:`showSidebarPlugins`(App.vue:104)声明后未读,**不迁移**。

#### stores/ui.js — useUiStore

```js
import { defineStore } from 'pinia'
import { ref, computed } from 'vue'

export const useUiStore = defineStore('ui', () => {
  const powerMode = ref(false)              // App.vue:25
  const appVersion = ref('...')             // App.vue:28
  const importError = ref('')               // App.vue:33

  // 面板开关(全 v-if)
  const showCharList = ref(false)           // App.vue:34
  const showCampaignPanel = ref(false)      // App.vue:35
  const showMetaPanel = ref(false)          // App.vue:36
  const showPresetPanel = ref(false)        // App.vue:63
  const showPluginPanel = ref(false)        // App.vue:64
  const showDebugDrawer = ref(false)        // App.vue:65
  const showSidebar = ref(false)            // App.vue:66
  const showConnConfig = ref(false)         // App.vue:340
  const showHistory = ref(true)             // App.vue:336
  const activeCampaignOverview = ref(true)  // App.vue:338

  // getters
  const currentView = computed(() => {      // App.vue:70-74
    // 依赖 showHistory / activeCampaignOverview / activeCampaign(campaignStore)
  })
  const pageTitle = computed(() => {        // App.vue:77-83
    // 依赖 currentView / writingMode / activeCampaign / activeChar
  })

  // actions(简单切换,可放 store)
  function viewHistory() { showHistory.value = true; activeCampaignOverview.value = false }
  function viewOverview() { showHistory.value = true; activeCampaignOverview.value = true }
  // 注意:viewWrite(App.vue:94)是死代码,不迁移

  return { /* 全部 state + getters + actions */ }
})
```

#### 验证

为每个 store 写 `tests/stores/{campaign,writing,plugin,ui}.test.mjs`:
```bash
cd frontend && npm test   # 增 ~15 个用例,总 ~172
```

测试要点:
- 用 `setActivePinia(createPinia())` 激活
- 测 state 读写:`const s = useCampaignStore(); s.activeCampaign = {id:'x'}; assert(s.activeCampaign.id === 'x')`
- 测 getter 派生:设 showHistory=true + mock activeCampaign,断言 currentView === 'overview'

**提交信息:** `feat: extract pinia stores from app.vue state`

---

### 阶段 2:ui/ 基础组件库(18 个)

**目标:** 按 `docs/FRONTEND-COMPONENTS.md` 的 ui/ 清单建组件。后续业务组件只组合它们。

#### Headless UI 承载(行为可靠,样式自写)

| 组件 | Headless UI 基础 | 关键状态 |
|---|---|---|
| `Overlay.vue` | Dialog + TransitionRoot | side: left/right/center/full |
| `Dialog.vue` | Dialog | confirm / cancel / danger |
| `Tabs.vue` | TabGroup/TabList/Tab/TabPanel | lazy / dirty |
| `Menu.vue` | Menu/MenuButton/MenuItems/MenuItem | keyboard close |
| `Tooltip.vue` | (手写或 Headless) | hover / focus |

#### 手写(纯展示,`<script setup>` + Tailwind 令牌类)

`Button.vue`、`IconButton.vue`、`Input.vue`、`Textarea.vue`、`Select.vue`、`SegmentedControl.vue`、`Checkbox.vue`、`Toggle.vue`、`Slider.vue`、`Badge.vue`、`Progress.vue`、`Toast.vue`、`EmptyState.vue`、`ErrorState.vue`、`LoadingState.vue`、`DataList.vue`、`DataTable.vue`、`DiffView.vue`、`CodeBlock.vue`

**组件规范:**
- 全部 `<script setup>` + `defineProps`/`defineEmits`
- 样式用 §2.3 的 Tailwind 令牌类(`bg-surface`/`text-ink`/`border-accent`/`shadow-glow-accent` 等),不用硬编码颜色
- 状态覆盖参考 FRONTEND-COMPONENTS.md:156-170

**Button.vue 示例(给执行者参照风格):**
```vue
<script setup>
const props = defineProps({
  variant: { type: String, default: 'default' }, // default | primary | danger | ghost
  size: { type: String, default: 'md' },         // sm | md | lg
  loading: { type: Boolean, default: false },
  disabled: { type: Boolean, default: false },
})
const variantClass = {
  default: 'bg-surface-2 text-ink border border-line hover:border-accent-border',
  primary: 'bg-accent text-bg font-medium shadow-glow-accent hover:opacity-90',
  danger: 'bg-err/15 text-err border border-err/40 hover:bg-err/25',
  ghost: 'text-ink-soft hover:text-ink hover:bg-surface-2',
}
const sizeClass = { sm: 'px-2.5 py-1 text-xs', md: 'px-3.5 py-1.5 text-sm', lg: 'px-5 py-2.5' }
</script>
<template>
  <button :class="['rounded-lg transition-colors duration-150 disabled:opacity-40 disabled:cursor-not-allowed',
    variantClass[variant], sizeClass[size]]" :disabled="disabled || loading">
    <slot v-if="!loading" />
    <span v-else class="inline-block animate-spin">⏳</span>
  </button>
</template>
```

#### 验证

为每个展示组件写挂载测试 `tests/components-v2/*.test.mjs`:
```bash
cd frontend && npm test   # 增 ~30 个用例,总 ~202
```

挂载测试要点(需 happy-dom 环境 + @vue/test-utils):
```js
import { mount } from '@vue/test-utils'
import Button from '../src/components-v2/ui/Button.vue'
test('primary variant renders accent class', () => {
  const w = mount(Button, { props: { variant: 'primary' } })
  assert(w.classes().some(c => c.includes('accent')))
})
```

> **node --test 跑 .vue 挂载测试**:需在 package.json test 脚本加 `--import` happy-dom,或测试文件顶部 `import 'happy-dom/global.js'`。执行时确认 node --test 能解析 .vue(通过 vite 的 ssrCompile 或 @vue/test-utils 的 transform)。若 node --test 无法直接跑 .vue,改用 vitest(需额外装)。**这是阶段 2 第一个要验证的技术点。**

**提交信息:** `feat: add ui component library (headless ui + hand-written)`

---

### 阶段 3:composables(逻辑层)

**目标:** 把 App.vue 的内联业务逻辑抽成 composable,消费阶段 1 的 store。

| Composable | 文件 | 来源(App.vue 行号) |
|---|---|---|
| `useWriting` | `composables/useWriting.js` | startWriting(696-809)、cancelWriting(812-819) |
| `usePipeline` | `composables/usePipeline.js` | handlePipelineEvent(1100-1211) |
| `usePluginBridge` | `composables/usePluginBridge.js` | 129-282 整段 |
| `useConversation` | `composables/useConversation.js` | openConversation(476)、handleDeleteConversation(456)、startNewConversation(521)、applyConversation(416) |
| `useMessageVariants` | `composables/useMessageVariants.js` | 822-1097 整段 |
| `useCharacterImport` | `composables/useCharacterImport.js` | handleImport(623-658) |
| `useGreeting` | `composables/useGreeting.js` | 347-385 |
| `useNewCampaignForm` | `composables/useNewCampaignForm.js` | 530-609 + 模板 1412-1445 |

**纯函数补 utils/(补强现有 util 层,加测试):**
- `utils/forkCampaignName.js` — `makeForkCampaignName`(App.vue:1008-1017)
- `utils/roleLabel.js` — 合并 `getAssistantRoleLabel`(314)与 `streamingRoleLabel`(86)
- `utils/consoleForwarding.js` — `setupConsoleForwarding`(402-412)

**composable 写法规范:**
```js
import { useWritingStore, useCampaignStore, usePluginStore } from '../stores/index.js'
import * as api from '../tauri-api.js'

export function useWriting() {
  const writing = useWritingStore()
  const campaign = useCampaignStore()
  const plugin = usePluginStore()

  async function startWriting(intent) { /* 迁移 App.vue:696-809 逻辑 */ }
  function cancelWriting() { /* 迁移 812-819 */ }

  return { startWriting, cancelWriting }
}
```

**迁移要点(逐个 composable):**
1. 把 App.vue 对应行号的函数体原样搬进 composable
2. 把函数体内的 `xxx.value` 改成 `store.xxx`(因为 store 自动解包)
3. 把 `tauri-api` 调用保持原样(import 具名函数)
4. 把 ST_EVENT_TYPES 广播保持原样(import from plugin-bridge)
5. 把跨 store 引用通过 `useXxxStore()` 拿

#### 验证

每个 composable 写单元测试 `tests/composables/*.test.mjs`(mock store + mock tauri-api):
```bash
cd frontend && npm test   # 增 ~40 个用例,总 ~242
```

**提交信息:** `feat: extract app.vue logic into composables`

---

### 阶段 4:shell/ 应用框架

**目标:** 三栏应用框架空壳能跑。

组件(放 `components-v2/shell/`):
- `AppShell.vue` — 根布局,`h-screen flex flex-col`,主区域 + 移动/桌面断点
- `TopBar.vue` — 标题(uiStore.pageTitle)/状态(writingStore.isWriting)/菜单/调试入口
- `PrimarySidebar.vue` — 主导航,emit 切视图
- `InspectorDrawer.vue` — 右调试抽屉,消费 pluginStore
- `PanelHost.vue` — 统一弹层承载(Overlay 包装)

**验证:** AppShell 挂载测试 + dev server 空壳可见。

**提交信息:** `feat: add app shell framework`

---

### 阶段 5:writing/ 写作视图(P0 核心)

**目标:** 完整写作工作台。完成后用户能导入/打开 Campaign、写作、停止、查看消息、切换历史。

组件(放 `components-v2/writing/`,消费 writingStore + campaignStore):
- `ConversationViewport.vue` — 空态/开场白/消息列表/流式
- `GreetingSelector.vue` — 多开场白选择
- `ChatMessage.vue` — **⚠️ 保持 §1.3 的 8 emit + message shape 契约**
- `MessageVariantSwitcher.vue`、`MessageActionMenu.vue`
- `RichContent.vue`(放 `components-v2/st/`)— 复用 `utils/formatContent.js`
- `StreamingMessage.vue` — 消费 writingStore.pipeline
- `Composer.vue` — 意图输入/开始停止
- `ConversationHistoryList.vue` — 消费 campaignStore.conversationHistory
- `CampaignOverview.vue` — Campaign 概览

**组装 `AppV2.vue`**(此时只含 shell + writing,其他面板用占位):
```vue
<script setup>
import { useWriting, useConversation, useMessageVariants } from './composables/...'
// ... AppShell + ConversationViewport + Composer
</script>
```

**验证:**
- `AppV2.vue` 通过临时 main.js 切换挂载后,dev server 跑通
- 能看消息流、触发 startWriting、切换历史
- `npm test` 全绿

**提交信息:** `feat: rebuild writing workspace (p0)`

---

### 阶段 6:campaign/ Campaign 主线(P1)

**目标:** Campaign 管理面板。

组件(放 `components-v2/campaign/`):
- `CampaignPanel.vue` — **⚠️ 保持 `refreshActiveDetailTab` expose 契约(§1.3)**
- `CardLibrary.vue`、`CardDetailPreview.vue`、`NewCampaignForm.vue`
- `CampaignDetailTabs.vue`(用 ui/Tabs)
- `InstancesTab.vue`、`KnowledgeTab.vue`、`TasksTab.vue`、`SummariesTab.vue`
- `CampaignExportImportBar.vue`

各 Tab 复用 `utils/campaignDisplay.js`/`campaignCardStatus.js`。

**验证:** 各 Tab 挂载测试。`npm test` 全绿。

**提交信息:** `feat: rebuild campaign management panel (p1)`

---

### 阶段 7:meta + st + config + debug(P2/P3)

**目标:** Meta 助手、MVU/ST 兼容、配置、调试面板。

组件清单:
- `meta/`:MetaPanel(**⚠️ 保持 lastConversationNode/mvu-applied 契约 §1.3**)、MetaChat、HealthCheckPanel、PatchPreview、GenerationExplanation、MvuAnalyzer
- `st/`:MvuStatusBar(复用 `mvuStatusBarModel.js`)、MvuSchemaPreview、StCompatibilityBadge、MvuJsRuntime(**⚠️ 保持 SHIM_SCRIPT 标记 §1.4**)、RegexScriptSummary
- `config/`:ConnectionConfigPanel、PresetPanel、AgentProfileManager、PluginPanel、PluginHost(**⚠️ 保持 slot helper 结构 §1.4**)
- `debug/`:PipelineTracePanel(复用 `pipelineTrace.js`)、PluginEventLog、PromptHookAuditLog(复用 `promptHookAudit.js`)、LogPanel

**验证:** 半脆弱测试(plugin-host-slots 6 + mvu-runtime-bridge 10)必须仍通过。`npm test` 全绿。

**提交信息:** `feat: rebuild meta/st/config/debug panels (p2/p3)`

---

### 阶段 8:组装 + 切换

**目标:** `AppV2.vue` 完整,切换 main.js 挂载点。

步骤:
1. `AppV2.vue` 串起全部(shell + writing + campaign + meta + st + config + debug),面板通过 uiStore 的 show* 开关 + PanelHost 承载
2. `main.js` 改:
   ```js
   import { createApp } from 'vue'
   import { createPinia } from 'pinia'
   import './style.css'
   import AppV2 from './AppV2.vue'
   createApp(AppV2).use(createPinia()).mount('#app')
   ```
3. 完整验收:dev server 走主流程(建 Campaign→写→看面板→导出)
4. `npm run build` 成功
5. `npm test` 全绿

**验证:** Release gate 6 步全过。

**提交信息:** `feat: switch to AppV2 as main entry`

---

### 阶段 9:旧代码清理

步骤:
1. 删 `frontend/src/App.vue`(旧)、`frontend/src/components/*.vue`(旧 26 个)、`frontend/src/components/base/*`(被 ui/ 替代的)
2. `components-v2/` → `components/`(对齐蓝图最终结构)
3. 更新 `docs/FRONTEND-COMPONENTS.md`(标注已实现)
4. 更新 `docs/HANDOFF.md`(记录 Phase 8 完成)
5. 全量回归:`npm test` + `npm run build`

**提交信息:** `chore: remove legacy frontend, rename components-v2 to components`

---

## 4. 执行顺序与依赖

```
阶段 0(地基) → 阶段 1(stores) → 阶段 2(ui 库)
                                      ↓
                        阶段 3(composables)← 依赖 stores
                                      ↓
                        阶段 4(shell) ← 依赖 ui + stores
                                      ↓
                        阶段 5(writing) ← 依赖 shell + composables
                                      ↓
                        阶段 6(campaign) ┐
                        阶段 7(meta/st/config/debug) ┘ 可并行
                                      ↓
                        阶段 8(组装切换) → 阶段 9(清理)
```

## 5. 风险控制

1. **每阶段独立提交**:完成后验证 + commit,出问题 `git revert` 单阶段回滚。
2. **旧代码全程保留**:阶段 8 切换前,旧 App.vue + 旧组件原封不动,任何时刻 `main.js` 回指旧入口即可恢复。
3. **半脆弱测试即时盯**:每次重写 PluginHost/MvuJsRuntime 后立即跑对应测试。
4. **util 层冻结**:不碰 `utils/` + `plugin-bridge.js` + `tauri-api.js` 导出签名,保 141 测试护城河。
5. **新增测试护城河**:阶段 2 起加组件挂载测试,补强零组件测试。

---

## 6. 执行记录

| 阶段 | 状态 | 提交 | 测试数 | 备注 |
|---|---|---|---|---|
| 0 | ✅ 完成 | (阶段0提交) | 157 旧全过 | 装依赖+建目录+stores/index.js+本文档;build 通过 |
| 1 | ✅ 完成 | (阶段1提交) | 196(157旧+39新) | 4 个 store(campaign/writing/plugin/ui)+ 跨 store getter + test glob 扩为 `tests/**/*.test.mjs` |
| 2 | ✅ 完成 | (本次提交) | node:196 + vitest:21 | 24 个 ui 组件(Button/IconButton/Input/Textarea/Select/SegmentedControl/Checkbox/Toggle/Slider/Badge/Progress/Toast/EmptyState/ErrorState/LoadingState/DataList/DataTable/DiffView/CodeBlock + Headless UI: Overlay/Dialog/Tabs/Menu/Tooltip);双轨测试:node --test 跑纯JS,vitest 跑组件挂载 |
| 3 | ⬜ 待做 | | | |
| 4 | ⬜ 待做 | | | |
| 5 | ⬜ 待做 | | | |
| 6 | ⬜ 待做 | | | |
| 7 | ⬜ 待做 | | | |
| 8 | ⬜ 待做 | | | |
| 9 | ⬜ 待做 | | | |
