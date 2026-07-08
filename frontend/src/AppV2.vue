<script setup>
/**
 * AppV2 — StoryForge 重构版根组件（Phase 8）
 *
 * 组装 AppShell + 三视图（overview/history/write）+ Composer + 功能面板。
 *
 * Composable 接线（核心）：
 *   ┌─ usePluginBridge()  无注入依赖，提供事件广播 / payload 构造 / prompt hook 编排
 *   │   ↳ broadcastPluginEvent / broadcastChatChanged / chatEventPayload /
 *      messageEventPayload / runPromptHookEvents
 *   ├─ useGreeting()      注入 broadcastChatChanged + scrollToBottom
 *   │   ↳ normalizeGreetingSelection / applySelectedOpeningMessage / selectGreeting
 *   ├─ useConversation()  注入 loadInstanceNameMap + loadCharDetail + applySelectedOpeningMessage
 *   │   ↳ applyConversation / loadConversationHistory / handleDeleteConversation /
 *      openConversation / startNewConversation
 *   ├─ usePipeline()      注入 scrollToBottom
 *   │   ↳ handlePipelineEvent
 *   ├─ useWriting()       注入 handlePipelineEvent + runPromptHookEvents + applyConversation +
 *   │   broadcastPluginEvent + messageEventPayload + scrollToBottom + loadInstanceNameMap + alertDialog
 *   │   ↳ startWriting / cancelWriting
 *   ├─ useMessageVariants()  注入 handlePipelineEvent + applyConversation + broadcastPluginEvent +
 *   │   messageEventPayload + chatEventPayload + loadInstanceNameMap + loadConversationHistory +
 *   │   scrollToBottom + alertDialog + startWriting
 *   │   ↳ 8 个 handler（reroll / reroll-user / edit-variant / accept-variant /
 *      delete-variant / branch / add-variant / switch-variant）
 *   ├─ useNewCampaignForm()  注入 loadInstanceNameMap + applyConversation + broadcastPluginEvent +
 *   │   loadConversationHistory + alertDialog
 *   │   ↳ openNewCampaignDialog / handleCreateCampaign 等
 *   └─ useCharacterImport() 注入 broadcastPluginEvent + applySelectedOpeningMessage + normalizeGreetingSelection
 *       ↳ handleImport
 *
 * 范围外（AppV2 内联实现，未列入迁移清单）：
 *   - loadInstanceNameMap()        App.vue:42-54
 *   - loadCharDetail(id)           App.vue:661-669
 *   - refreshActiveConnection()    App.vue:612-616
 *   - loadSidebarPlugins()         App.vue:113-127
 *   - setupConsoleForwarding()     App.vue:402-412
 */
import { ref, computed, onMounted } from 'vue'
import AppShell from './components-v2/shell/AppShell.vue'
import CampaignOverview from './components-v2/writing/CampaignOverview.vue'
import ConversationHistoryList from './components-v2/writing/ConversationHistoryList.vue'
import ConversationViewport from './components-v2/writing/ConversationViewport.vue'
import Composer from './components-v2/writing/Composer.vue'
import CampaignPanel from './components-v2/campaign/CampaignPanel.vue'
import MetaPanel from './components-v2/meta/MetaPanel.vue'
import {
  useWritingStore,
  useCampaignStore,
  useUiStore,
} from './stores/index.js'
import { usePluginBridge } from './composables/usePluginBridge.js'
import { useGreeting } from './composables/useGreeting.js'
import { useConversation } from './composables/useConversation.js'
import { usePipeline } from './composables/usePipeline.js'
import { useWriting } from './composables/useWriting.js'
import { useMessageVariants } from './composables/useMessageVariants.js'
import { useNewCampaignForm } from './composables/useNewCampaignForm.js'
import { useCharacterImport } from './composables/useCharacterImport.js'
import {
  getVersion,
  getActiveCampaign,
  getActiveConnection,
  getCharacter,
  listInstances,
  listPlugins,
  logAppendFrontend,
} from './tauri-api.js'
import { ST_EVENT_TYPES } from './plugin-bridge.js'
import { alertDialog } from './components/base/BaseDialog.js'

// ─── stores ───
const writing = useWritingStore()
const campaign = useCampaignStore()
const ui = useUiStore()

// ─── 范围外辅助函数（App.vue 内联实现，未列入迁移清单） ───
// loadInstanceNameMap（App.vue:42-54）：刷新 activeCampaign 对应的实例名映射
async function loadInstanceNameMap() {
  if (!campaign.activeCampaign) {
    campaign.instanceNameMap = {}
    return
  }
  try {
    const insts = await listInstances(campaign.activeCampaign.id)
    const map = {}
    for (const inst of insts) {
      if (inst.id) map[inst.id] = inst.name || inst.character_name || ''
    }
    campaign.instanceNameMap = map
  } catch (e) {
    console.error('加载实例名映射失败:', e)
  }
}

// loadCharDetail（App.vue:661-669）：加载角色详情 + 归一化开场白索引
// 依赖 greeting.normalizeGreetingSelection（下方声明；运行时调用时已初始化）
async function loadCharDetail(id) {
  try {
    campaign.activeCharDetail = await getCharacter(id)
    writing.selectedGreetingIndex = 0
    greeting.normalizeGreetingSelection()
  } catch (e) {
    console.error('加载详情失败:', e)
  }
}

// refreshActiveConnection（App.vue:612-616）
async function refreshActiveConnection() {
  try {
    writing.activeConnection = await getActiveConnection()
  } catch (e) {
    console.error('获取活跃连接失败:', e)
  }
}

// loadSidebarPlugins（App.vue:113-127）：本阶段由 plugin store 承载，AppV2 暂为 stub。
// TODO 阶段 6-7：迁移插件侧栏装配逻辑。
async function loadSidebarPlugins() {
  try {
    await listPlugins()
  } catch (e) {
    console.error('加载侧栏插件失败:', e)
    logAppendFrontend('error', `loadSidebarPlugins: ${e}`).catch(() => {})
  }
}

// setupConsoleForwarding（App.vue:402-412）：拦截 console 转发到后端 LogStore
function setupConsoleForwarding() {
  const levels = { log: 'info', warn: 'warn', error: 'error', debug: 'debug' }
  for (const [method, level] of Object.entries(levels)) {
    const original = console[method]
    console[method] = (...args) => {
      original.apply(console, args)
      const msg = args.map((a) => (typeof a === 'string' ? a : JSON.stringify(a))).join(' ')
      logAppendFrontend(level, msg).catch(() => {})
    }
  }
}

// ─── 视图滚动桥：ConversationViewport 暴露 scrollToBottom，AppV2 转发为函数引用 ───
const viewportRef = ref(null)
function scrollToBottom() {
  // viewport 仅在 write 视图存在；其他视图调用为 no-op
  viewportRef.value?.scrollToBottom?.()
}

// ─── 功能面板 refs + 事件桥 ───
// CampaignPanel ref：MetaPanel mvu-applied 后调用其 refreshActiveDetailTab（App.vue:56-61 链）
const campaignPanelRef = ref(null)

// lastConversationNode：生成溯源入口（campaign store getter）
const lastConversationNode = computed(() => campaign.lastConversationNode)

// MVU Apply 成功后刷新 Campaign 当前 detail 子 tab
function handleMvuApplied() {
  campaignPanelRef.value?.refreshActiveDetailTab?.()
}

// ─── Composable 装配 ───
// 顺序遵循依赖拓扑：pluginBridge（无依赖）→ greeting → conversation → pipeline → writing → messageVariants → forms/import。
// 注入的函数引用全部在运行时（用户操作触发）才被调用，此时下方所有 const 已完成初始化。

// 1. pluginBridge —— 事件广播 / payload 构造 / prompt hook 编排（无注入依赖）
const pluginBridge = usePluginBridge()
const {
  broadcastPluginEvent,
  broadcastChatChanged,
  chatEventPayload,
  messageEventPayload,
  runPromptHookEvents,
} = pluginBridge

// 2. greeting —— 开场白选择（注入 broadcastChatChanged + scrollToBottom）
const greeting = useGreeting({
  broadcastChatChanged,
  scrollToBottom,
})

// 3. conversation —— 会话装配 / 历史 / 打开 / 删除（注入范围外依赖）
const conversation = useConversation({
  loadInstanceNameMap,
  loadCharDetail,
  applySelectedOpeningMessage: greeting.applySelectedOpeningMessage,
})
const {
  applyConversation,
  loadConversationHistory,
  handleDeleteConversation,
  openConversation,
  startNewConversation,
} = conversation

// 4. pipeline —— 流水线事件 reducer（注入 scrollToBottom）
const pipeline = usePipeline({ scrollToBottom })
const { handlePipelineEvent } = pipeline

// 5. writing —— 开始 / 取消写作（注入 handlePipelineEvent + 范围外辅助）
const writingApi = useWriting({
  handlePipelineEvent,
  runPromptHookEvents,
  applyConversation,
  broadcastPluginEvent,
  messageEventPayload,
  scrollToBottom,
  loadInstanceNameMap,
  alertDialog,
})
const { startWriting, cancelWriting } = writingApi

// 6. messageVariants —— 8 个变体 handler（注入 handlePipelineEvent + 范围外辅助 + startWriting）
const messageVariants = useMessageVariants({
  handlePipelineEvent,
  applyConversation,
  broadcastPluginEvent,
  messageEventPayload,
  chatEventPayload,
  loadInstanceNameMap,
  loadConversationHistory,
  scrollToBottom,
  alertDialog,
  startWriting,
})
const {
  handleReroll,
  handleRerollUser,
  handleEditVariant,
  handleAcceptVariant,
  handleDeleteVariant,
  handleBranch,
  handleAddVariant,
  handleSwitchVariant,
} = messageVariants

// 7. newCampaignForm —— 新建 Campaign 表单
const newCampaignForm = useNewCampaignForm({
  loadInstanceNameMap,
  applyConversation,
  broadcastPluginEvent,
  loadConversationHistory,
  alertDialog,
})
const { openNewCampaignDialog } = newCampaignForm

// 8. characterImport —— 角色卡导入
const characterImport = useCharacterImport({
  broadcastPluginEvent,
  applySelectedOpeningMessage: greeting.applySelectedOpeningMessage,
  normalizeGreetingSelection: greeting.normalizeGreetingSelection,
})
const { handleImport } = characterImport

// ─── 模板装配辅助 ───
// Composer 占位文案随写作模式变化（App.vue:1358）
const composerPlaceholder = () =>
  writing.writingMode === 'none' ? '请先导入角色卡或打开 Campaign…' : ''

// ─── 初始化（App.vue:388-399） ───
onMounted(async () => {
  try { ui.appVersion = await getVersion() } catch (e) { console.error('getVersion:', e) }
  await refreshActiveConnection()
  await loadConversationHistory()
  try {
    campaign.activeCampaign = await getActiveCampaign()
    await loadInstanceNameMap()
  } catch (e) { console.error('getActiveCampaign:', e) }
  await loadSidebarPlugins()
  setupConsoleForwarding()
  broadcastPluginEvent(ST_EVENT_TYPES.APP_READY, chatEventPayload({ version: ui.appVersion }))
})
</script>

<template>
  <AppShell @new-campaign="openNewCampaignDialog" @import="handleImport">
    <template #content>
      <!-- 根据 ui.currentView 切换 overview/history/write -->
      <CampaignOverview
        v-if="ui.currentView === 'overview'"
        @open-campaign="ui.showCampaignPanel = true"
        @new-campaign="openNewCampaignDialog"
        @view-history="ui.viewHistory()"
      />
      <ConversationHistoryList
        v-else-if="ui.currentView === 'history'"
        @open="openConversation"
        @delete="handleDeleteConversation"
        @new-campaign="openNewCampaignDialog"
      />
      <ConversationViewport
        v-else
        ref="viewportRef"
        :handlers="messageVariants"
        :on-select-greeting="greeting.selectGreeting"
      />
    </template>

    <template #composer>
      <Composer
        @start-writing="startWriting"
        @cancel="cancelWriting"
        :writing="writing.isWriting"
        :disabled="writing.writingMode === 'none'"
        :placeholder="composerPlaceholder()"
      />
    </template>

    <template #panels>
      <!-- 阶段 6-7 补充：CampaignPanel / Meta 等面板 -->
      <CampaignPanel
        v-if="ui.showCampaignPanel"
        ref="campaignPanelRef"
        @close="ui.showCampaignPanel = false"
        @campaign-changed="(c) => { campaign.activeCampaign = c }"
      />
      <MetaPanel
        v-if="ui.showMetaPanel"
        :active-campaign="campaign.activeCampaign"
        :last-conversation-node="lastConversationNode"
        @close="ui.showMetaPanel = false"
        @mvu-applied="handleMvuApplied"
      />
    </template>
  </AppShell>
</template>
