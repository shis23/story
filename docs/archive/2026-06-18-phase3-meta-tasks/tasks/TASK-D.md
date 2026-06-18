# Task D：前端 —— MetaPanel 修复建议 UI + 生成溯源接入

> 你的 worktree：`C:\tmp\sf-taskD`，分支：`codex/meta-repair-frontend`
> 起始 commit：main `06e53fe`。
> 完整契约见同目录 `ROUND-3-README.md`（**必读**）。

## 你负责的文件（只动这两个）

- `frontend/src/components/MetaPanel.vue`
- `frontend/src/tauri-api.js`（**仅追加** export，不改现有函数）

**禁止改动**：
- `crates/**`（A 和 C 的领地）
- `frontend/src/App.vue` 及其他组件

## 背景：你要闭合两个前端缺口

### 缺口 1（主）：修复建议 UI

`meta_health_check` 已接入 MetaPanel（体检区块，line ~368-402）。体检发现问题后，**用户没有任何修复入口**。C 任务（并行）新增了 5 个 Tauri 命令，你在前端接入它们，让「体检发现问题 → 生成修复建议 → 预览 diff → 接受/忽略」成为闭环。

### 缺口 2（次，但重要）：生成溯源接入

`meta_explain_generation(conversation_id, node_id)` 命令**已存在**（main HEAD 已含），但**前端从不调用它**（PLAN-META-AGENT 阶段 2 前端验收项未完成）。你接入它：在 MetaPanel 提供一个入口，让用户解释某条消息的生成溯源。

## 要做的 api 包装（tauri-api.js，追加到文件末尾）

仿现有 `metaHealthCheck`（line ~871）的写法，追加：

```js
export async function metaProposeCampaignRepairs(campaignId) {
  if (window.__TAURI_INTERNALS__) {
    return await invoke('meta_propose_campaign_repairs', { campaignId })
  }
  return [] // mock 空数组
}

export async function metaListTypedPatches() {
  if (window.__TAURI_INTERNALS__) {
    return await invoke('meta_list_typed_patches')
  }
  return []
}

export async function metaPreviewTypedPatch(patchId, campaignId) {
  if (window.__TAURI_INTERNALS__) {
    return await invoke('meta_preview_typed_patch', { patchId, campaignId })
  }
  return { stale: false, patch: null, diff: [] }
}

export async function metaAcceptTypedPatch(patchId, campaignId) {
  if (window.__TAURI_INTERNALS__) {
    return await invoke('meta_accept_typed_patch', { patchId, campaignId })
  }
}

export async function metaDismissTypedPatch(patchId) {
  if (window.__TAURI_INTERNALS__) {
    return await invoke('meta_dismiss_typed_patch', { patchId })
  }
}

// 生成溯源（命令已存在于后端）
export async function metaExplainGeneration(conversationId, nodeId) {
  if (window.__TAURI_INTERNALS__) {
    return await invoke('meta_explain_generation', { conversationId, nodeId })
  }
  return null
}
```

**重要**：所有包装用 `window.__TAURI_INTERNALS__` 守卫（仿现有写法），命令不存在时走 mock。这样即使 C 还没合并，前端构建不会崩，只是功能空跑。**不要**让命令缺失导致整个 MetaPanel 报错 —— invoke 失败用 try/catch 包住，UI 显示「该功能暂不可用」。

## 要做的 MetaPanel.vue 改动

### 1. 修复建议区块（紧接体检区块之后，line ~402 的 `</div>` 后插入）

UI 结构（用项目现有 Tailwind 风格，参考体检区块的 class）：

- 标题行：「🔧 修复建议」
- 按钮「生成修复方案」：调 `metaProposeCampaignRepairs(activeCampaign.id)`，loading 态。**只在体检跑过且有问题时显示**（`v-if="healthRan && healthIssues.length > 0"`）。
- patch 列表（`v-for` over `typedPatches`）：
  - 每条 card：description + source_issue_category 标签 + 受影响 id。
  - **展开/折叠 diff**：点击展开显示 `patch.diff`（每条 FieldDiff：`path` + before → after，before/after 用 `<pre>` 或 JSON 折行显示）。
  - 两个按钮：「接受」（调 `metaAcceptTypedPatch`，成功后从列表移除并提示）、「忽略」（调 `metaDismissTypedPatch`）。
  - **stale 标记**：preview 返回 `stale: true` 时，patch card 显示「⚠️ 已过期（target 已变更）」，禁用接受按钮。
  - 接受成功后：调一次 `metaHealthCheck` 刷新体检（让用户看到问题减少 —— 这是 PLAN-META-AGENT 阶段 3 的验收项「接受 patch 后 health check 问题减少」）。

### 2. 状态变量（script setup 顶部 ref 区，仿现有 healthIssues）

```js
const typedPatches = ref([])       // TypedPatch[]
const patchesLoading = ref(false)
const expandedPatchId = ref(null)  // 与现有 expandedPatchId（line 25）同名会冲突 → 重命名为 expandedTypedPatchId
const explainLoading = ref(false)
const explainResult = ref(null)    // GenerationExplanation
```

注意：现有已有 `expandedPatchId`（line 25）用于旧 config-meta patch。你的新变量用 `expandedTypedPatchId` 避免冲突。

### 3. 处理函数

```js
async function handleProposeRepairs() {
  if (!props.activeCampaign?.id) return
  patchesLoading.value = true
  try {
    const patches = await metaProposeCampaignRepairs(props.activeCampaign.id)
    typedPatches.value = patches || []
    // 对每条 patch 跑 preview，标记 stale
    for (const p of typedPatches.value) {
      try {
        const prev = await metaPreviewTypedPatch(p.id, props.activeCampaign.id)
        p._stale = prev?.stale || false
      } catch (e) { p._stale = false }
    }
  } catch (e) {
    error.value = '生成修复方案失败: ' + e
  } finally {
    patchesLoading.value = false
  }
}

async function handleAcceptTypedPatch(patchId) {
  try {
    await metaAcceptTypedPatch(patchId, props.activeCampaign.id)
    // 从列表移除
    typedPatches.value = typedPatches.value.filter(p => p.id !== patchId)
    // 刷新体检（让用户看到问题减少）
    await handleHealthCheck()
  } catch (e) {
    error.value = '接受修复失败: ' + e
  }
}

async function handleDismissTypedPatch(patchId) {
  try {
    await metaDismissTypedPatch(patchId)
    typedPatches.value = typedPatches.value.filter(p => p.id !== patchId)
  } catch (e) {
    error.value = '忽略修复失败: ' + e
  }
}
```

### 4. 生成溯源入口（次要，放 MetaPanel 一个小入口）

在 MetaPanel 工具栏（line ~207 的「快速：」区）或体检区块下方，加一个「解释上一条生成」按钮：

- 需要传入 conversation_id 和 node_id。**问题**：MetaPanel 当前 props 只有 `activeCampaign`，没有 conversation/node 信息。
- **处理方式**：从 `activeCampaign` 拿不到 conversation id。**最简方案**：给 MetaPanel 新增一个可选 prop `lastConversationNode`（`{ conversation_id, node_id }`），由 App.vue 传入（当前 active conversation 的最后一条 agent 消息）。
  - **但 App.vue 不在你的允许文件列表里。** 所以：**在你的实现里，这个 prop 设为可选且默认 null**。UI 上：`v-if="lastConversationNode"` 才显示溯源按钮；prop 为 null 时（当前 App.vue 没传）不显示。这样你的改动自洽，不阻塞；App.vue 传 prop 是后续小改（审查 agent 合并后补，或下一轮）。
- 点击按钮调 `metaExplainGeneration(conversation_id, node_id)`，结果用浮层或区块展示 `GenerationExplanation`（scene_brief / subagents[] / last_hint / profile_id / seed）。subagents 列表显示每条的 display_name + task_brief + output_preview（截断）。

> 如果你判断溯源入口因 props 限制做不完整，**优先保证修复建议 UI 完整可用**（这是本轮主任务），溯源入口做成「prop 就绪即可用」的可扩展结构即可，并在报告里说明。

## diff 显示格式

FieldDiff 的 before/after 是 `serde_json::Value`。显示时：
- null → 文字「（无）」或灰字 `null`
- 字符串 → 原样
- 对象/数组 → `<pre class="text-[9px] overflow-x-auto">{{ JSON.stringify(value, null, 2) }}</pre>`
- before → after 用箭头或两行对比，参考现有 MVU 详情浮层的紧凑风格。

## 验证

```cmd
cd frontend
npm run build
```

构建必须通过。由于后端命令可能未合并（C 并行），运行时功能可能空跑 —— 这正常，构建不依赖命令存在（你的 invoke 都有 `__TAURI_INTERNALS__` 守卫 + try/catch）。

**不要**在 build 失败时提交。如果 build 因命令不存在报错，那是你的守卫写错了，修守卫。

## 约束

- 不引入新 npm 依赖（用现有 Vue + Tailwind）。
- 不改其他组件。MetaPanel 内所有改动自洽。
- diff 数据直接用 patch 自带的 `diff` 字段，前端不二次计算。

## 完成后报告

1. 改动文件列表 + 行数。
2. `npm run build` 结果（是否通过、构建时间）。
3. 修复建议 UI 是否完整（生成/预览/接受/忽略/stale 标记/体检刷新）。
4. 生成溯源入口的实现程度（完整 / 仅可扩展结构 / 未做），及原因。
5. 是否动了允许范围外的文件。
