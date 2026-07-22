<script setup>
/**
 * AppV2 — StoryForge 重构版根组件（Phase 8）
 *
 * 组装 design AppFrame + 三视图（overview/history/write）+ 功能面板。
 * Composer 已并入 WritingScreen；插件/MVU runtime 挂在 #panels。
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
import AppFrame from './design/shell/AppFrame.vue'
import PrimarySidebar from './components-v2/shell/PrimarySidebar.vue'
import TopBar from './components-v2/shell/TopBar.vue'
import InspectorDrawer from './components-v2/shell/InspectorDrawer.vue'
import WritingScreen from './design/writing/WritingScreen.vue'
import HistoryScreen from './design/history/HistoryScreen.vue'
import OverviewScreen from './design/overview/OverviewScreen.vue'
import CampaignPanel from './components-v2/campaign/CampaignPanel.vue'
import MetaPanel from './components-v2/meta/MetaPanel.vue'
import NewCampaignForm from './components-v2/campaign/NewCampaignForm.vue'
import ConnectionConfigPanel from './components-v2/config/ConnectionConfigPanel.vue'
import PresetPanel from './components-v2/config/PresetPanel.vue'
import PluginPanel from './components-v2/config/PluginPanel.vue'
import AgentProfileManager from './components-v2/config/AgentProfileManager.vue'
// 保留原位(未迁移 v2,功能简单/隐藏运行时):
import CharacterList from './components/CharacterList.vue'
import MvuJsRuntime from './components/MvuJsRuntime.vue'
import CardShellHost from './components/CardShellHost.vue'
import TavernHelperRuntime from './components/TavernHelperRuntime.vue'
import PluginHost from './components/PluginHost.vue'
import { useWritingScreenAdapter } from './adapter/useWritingScreenAdapter.js'
import { useHistoryScreenAdapter } from './adapter/useHistoryScreenAdapter.js'
import { useOverviewScreenAdapter } from './adapter/useOverviewScreenAdapter.js'
import ShellAwareContent from './components-v2/st/ShellAwareContent.vue'
import {
  useWritingStore,
  useCampaignStore,
  useUiStore,
  usePluginStore,
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
  getActiveTurnQuality,
  getCharacter,
  listInstances,
  listPlugins,
  logAppendFrontend,
  getCardShellManifest,
  getCard,
} from './tauri-api.js'
import { ST_EVENT_TYPES } from './plugin-bridge.js'
import { alertDialog } from './components/base/BaseDialog.js'

// ─── stores ───
const writing = useWritingStore()
const campaign = useCampaignStore()
const ui = useUiStore()
const plugin = usePluginStore()

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

// loadSidebarPlugins（App.vue:113-127）：列出已启用插件并写入 plugin store。
// hookPlugins = 全部 enabled；sidebarPlugins = 带 SidebarPanel slot 的 enabled。
async function loadSidebarPlugins() {
  try {
    const all = await listPlugins()
    const enabled = (all || []).filter((p) => p.enabled)
    const enabledIds = new Set(enabled.map((p) => p.id))
    // 清理已卸载/禁用插件的 slot 注册
    plugin.hookPluginSlots = Object.fromEntries(
      Object.entries(plugin.hookPluginSlots || {}).filter(([pluginId]) =>
        enabledIds.has(pluginId),
      ),
    )
    plugin.hookPlugins = enabled
    plugin.sidebarPlugins = enabled.filter((p) => p.ui_slots?.includes('SidebarPanel'))
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

// ─── 视图滚动桥：WritingScreen 暴露 scrollToBottom，AppV2 转发为函数引用 ───
const writingScreenRef = ref(null)
function scrollToBottom() {
  // 写作屏仅在 write 视图存在；其他视图调用为 no-op
  writingScreenRef.value?.scrollToBottom?.()
}

// ─── 可见 Card Shell（开场/状态；宿主代持远程）──────────────────────────
const cardShellStatusUrl = ref(null)
const cardShellOpeningUrl = ref(null)
const cardShellLabel = ref('')
const cardShellLoading = ref(false)
const cardShellShells = ref([])
const cardShellThCount = ref(0)

async function refreshCardShellManifest() {
  cardShellStatusUrl.value = null
  cardShellOpeningUrl.value = null
  cardShellLabel.value = ''
  cardShellShells.value = []
  cardShellThCount.value = 0
  let characterId =
    campaign.activeChar?.id ||
    campaign.activeCharDetail?.id ||
    campaign.activeCharDetail?.source_character_id ||
    null
  // 活动路径：从 card_id 反查 source_character_id
  if (!characterId && campaign.activeCampaign?.card_id) {
    try {
      const card = await getCard(campaign.activeCampaign.card_id)
      characterId = card?.source_character_id || null
    } catch (e) {
      console.error('getCard for shell manifest:', e)
    }
  }
  if (!characterId) return
  cardShellLoading.value = true
  try {
    const m = await getCardShellManifest(characterId)
    cardShellStatusUrl.value = m?.status_bar_url || null
    // 优先首页；无则自定义开局
    cardShellOpeningUrl.value = m?.opening_home_url || m?.opening_custom_url || null
    cardShellLabel.value = m?.character_id || characterId
    cardShellShells.value = Array.isArray(m?.shells) ? m.shells : []
    cardShellThCount.value = m?.tavern_helper_count || 0
  } catch (e) {
    console.error('getCardShellManifest:', e)
  } finally {
    cardShellLoading.value = false
  }
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

// handleSelectChar（App.vue:672-693,范围外辅助):CharacterList 选中角色卡。
// 用 active campaign / char 状态 + loadCharDetail + applySelectedOpeningMessage +
// broadcastPluginEvent 拼装,降级路径为 no-op。
async function handleSelectChar(char) {
  ui.showCharList = false
  if (!char) {
    campaign.activeChar = null
    campaign.activeCharDetail = null
    writing.messages = []
    campaign.currentConversationId = null
    writing.selectedGreetingIndex = 0
    broadcastChatChanged('character_cleared')
    return
  }
  campaign.activeChar = char
  campaign.currentConversationId = null
  await loadCharDetail(char.id)
  await refreshCardShellManifest()
  broadcastPluginEvent(ST_EVENT_TYPES.CHARACTER_LOADED, {
    characterId: char.id,
    name: char.name,
  })
  greeting.applySelectedOpeningMessage()
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
  beginPromptHookGeneration,
  cancelPromptHooks,
  isPromptHooksCancelled,
  setHookPluginHostRef,
  onHookPluginSlotMount,
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
const pipeline = usePipeline({ scrollToBottom, pluginBridge })
const { handlePipelineEvent } = pipeline

// 5. writing —— 开始 / 取消写作（注入 handlePipelineEvent + 范围外辅助）
const writingApi = useWriting({
  handlePipelineEvent,
  runPromptHookEvents,
  cancelPromptHooks,
  isPromptHooksCancelled,
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
  beginPromptHookGeneration,
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

  // 9. writing screen adapter —— design/writing 纯展示层接线
  const { screenProps: writingScreenProps, screenEvents: writingScreenEvents } =
    useWritingScreenAdapter({
      contentComponent: ShellAwareContent,
      startWriting,
      cancelWriting,
      selectGreeting: greeting.selectGreeting,
      openNewCampaign: openNewCampaignDialog,
      handleImport,
      viewHistory: () => ui.viewHistory(),
      handleReroll,
      handleRerollUser,
      handleEditVariant,
      handleAcceptVariant,
      handleDeleteVariant,
      handleBranch,
      handleAddVariant,
      handleSwitchVariant,
    })

  // 10. history screen adapter —— design/history 纯展示层接线
  const { screenProps: historyScreenProps, screenEvents: historyScreenEvents } =
    useHistoryScreenAdapter({
      openConversation,
      handleDeleteConversation,
      openNewCampaign: openNewCampaignDialog,
    })

  // 11. overview screen adapter —— design/overview 纯展示层接线
  const { screenProps: overviewScreenProps, screenEvents: overviewScreenEvents } =
    useOverviewScreenAdapter({
      openCampaign: () => { ui.showCampaignPanel = true },
      viewHistory: () => ui.viewHistory(),
      openNewCampaign: openNewCampaignDialog,
      continueWriting: () => ui.viewWrite(),
    })

  // 活动 Turn 质量报告回填（刷新后 ProcessReview 仍可显示）
  async function hydrateActiveTurnQuality() {
  const campaignId = campaign.activeCampaign?.id
  if (!campaignId || writing.isWriting) return
  try {
    const dto = await getActiveTurnQuality(campaignId)
    if (dto) writing.applyQualityFromTurn(dto)
  } catch (e) {
    console.error('getActiveTurnQuality:', e)
  }
}

// ─── 初始化（App.vue:388-399） ───
onMounted(async () => {
  try { ui.appVersion = await getVersion() } catch (e) { console.error('getVersion:', e) }
  await refreshActiveConnection()
  await loadConversationHistory()
  try {
    campaign.activeCampaign = await getActiveCampaign()
    await loadInstanceNameMap()
    await hydrateActiveTurnQuality()
    await refreshCardShellManifest()
  } catch (e) { console.error('getActiveCampaign:', e) }
  await loadSidebarPlugins()
  setupConsoleForwarding()
  broadcastPluginEvent(ST_EVENT_TYPES.APP_READY, chatEventPayload({ version: ui.appVersion }))
})
</script>

<template>
  <!-- design/shell AppFrame：纯布局；侧栏/顶栏/调试经 slot 注入；#panels 保留插件 runtime -->
  <AppFrame
    :sidebar-open="ui.showSidebar"
    :inspector-open="ui.showDebugDrawer"
    @update:sidebar-open="(v) => { ui.showSidebar = v }"
    @update:inspector-open="(v) => { ui.showDebugDrawer = v }"
  >
    <template #sidebar="{ docked }">
      <PrimarySidebar
        :docked="docked"
        @close="ui.showSidebar = false"
        @new-campaign="openNewCampaignDialog"
        @view-history="ui.viewHistory()"
        @open-campaign="ui.showCampaignPanel = true"
        @open-char-list="ui.showCharList = true"
        @import="handleImport"
        @open-conn="ui.showConnConfig = true"
        @open-preset="ui.showPresetPanel = true"
        @open-plugin="ui.showPluginPanel = true"
        @open-agent-profile="ui.showAgentProfile = true"
        @open-meta="ui.showMetaPanel = true"
      />
    </template>

    <template #topbar>
      <TopBar />
    </template>

    <template #content>
      <!-- 导入/抽取进度与错误提示条 -->
      <div
        v-if="ui.extracting || ui.importError"
        class="shrink-0 px-3 py-2 text-xs flex items-center gap-2 border-b"
        :class="ui.importError ? 'bg-err/10 text-err border-err/20' : 'bg-accent/10 text-accent border-accent/20'"
      >
        <span v-if="ui.extracting" class="flex items-center gap-1.5">
          <span class="animate-spin inline-block">◌</span>
          正在识别角色「{{ ui.extracting.name }}」…（可能需要数十秒）
        </span>
        <template v-else>
          <span class="flex-1">{{ ui.importError }}</span>
          <button class="text-ink-soft hover:text-ink" @click="ui.importError = ''">✕</button>
        </template>
      </div>
      <!-- 根据 ui.currentView 切换 overview/history/write -->
      <div v-if="ui.currentView === 'overview'" class="h-full overflow-y-auto">
        <OverviewScreen
          v-bind="overviewScreenProps"
          v-on="overviewScreenEvents"
        />
      </div>
      <div v-else-if="ui.currentView === 'history'" class="h-full overflow-y-auto">
        <HistoryScreen
          v-bind="historyScreenProps"
          v-on="historyScreenEvents"
        />
      </div>
      <!-- 写作主屏：design/writing + adapter 接线（含 ComposerBar） -->
      <WritingScreen
        v-else
        ref="writingScreenRef"
        class="h-full min-h-0"
        v-bind="writingScreenProps"
        v-on="writingScreenEvents"
      >
        <template #shell>
          <div
            v-if="cardShellStatusUrl || cardShellOpeningUrl || cardShellThCount"
            class="space-y-2 pb-2"
          >
            <CardShellHost
              v-if="cardShellStatusUrl"
              :url="cardShellStatusUrl"
              label="状态栏壳"
              compact
              height="110px"
            />
            <CardShellHost
              v-if="cardShellOpeningUrl && (!writing.messages || writing.messages.length <= 1)"
              :url="cardShellOpeningUrl"
              label="开场壳"
              height="420px"
            />
            <TavernHelperRuntime
              v-if="cardShellThCount"
              :shells="cardShellShells"
              :show-status="true"
              :auto-run="true"
            />
          </div>
        </template>
      </WritingScreen>
    </template>

    <template #composer>
      <!-- Composer 已并入 WritingScreen（稿纸下方指令条）；概览/历史不占底部 -->
    </template>

    <template #inspector>
      <InspectorDrawer />
    </template>

    <template #panels>
      <!-- ═══ 管理面板（每个用 ui.show* 控制；close 同时关面板并恢复侧栏） ═══ -->

      <!-- 角色卡列表（保留原位组件,未迁 v2） -->
      <CharacterList
        v-if="ui.showCharList"
        :active-id="campaign.activeChar?.id"
        @select="handleSelectChar"
        @close="ui.showCharList = false; ui.showSidebar = true"
      />

      <!-- LLM 连接配置 -->
      <ConnectionConfigPanel
        v-if="ui.showConnConfig"
        @close="ui.showConnConfig = false; ui.showSidebar = true"
        @changed="refreshActiveConnection"
      />

      <!-- Campaign 面板 -->
      <CampaignPanel
        v-if="ui.showCampaignPanel"
        ref="campaignPanelRef"
        @close="ui.showCampaignPanel = false; ui.showSidebar = true"
        @campaign-changed="(c) => { campaign.activeCampaign = c; loadInstanceNameMap(); refreshCardShellManifest() }"
      />

      <!-- Meta 面板 -->
      <MetaPanel
        v-if="ui.showMetaPanel"
        :active-campaign="campaign.activeCampaign"
        :last-conversation-node="lastConversationNode"
        @close="ui.showMetaPanel = false; ui.showSidebar = true"
        @mvu-applied="handleMvuApplied"
      />

      <!-- 预设面板 -->
      <PresetPanel
        v-if="ui.showPresetPanel"
        @close="ui.showPresetPanel = false; ui.showSidebar = true"
      />

      <!-- 插件面板（关后刷新侧栏插件） -->
      <PluginPanel
        v-if="ui.showPluginPanel"
        @close="ui.showPluginPanel = false; ui.showSidebar = true; loadSidebarPlugins()"
      />

      <!-- Agent Profile 配置面板（Phase 8 补挂,P3-5） -->
      <AgentProfileManager
        v-if="ui.showAgentProfile"
        @close="ui.showAgentProfile = false; ui.showSidebar = true"
      />

      <!-- ═══ 新建 Campaign 表单（Overlay,消费 useNewCampaignForm composable） ═══ -->
      <NewCampaignForm
        :show="newCampaignForm.showNewCampaignForm.value"
        :load-instance-name-map="loadInstanceNameMap"
        :apply-conversation="applyConversation"
        :broadcast-plugin-event="broadcastPluginEvent"
        :load-conversation-history="loadConversationHistory"
        :alert-dialog="alertDialog"
        @update:show="(v) => { newCampaignForm.showNewCampaignForm.value = v }"
        @close="newCampaignForm.showNewCampaignForm.value = false"
      />

      <!-- ═══ MVU JS Runtime（隐藏运行时,保留原位组件） ═══ -->
      <MvuJsRuntime />

      <!-- ═══ 插件 Hook Host 循环（隐藏,保留原位组件） ═══ -->
      <div class="hidden" aria-hidden="true">
        <PluginHost
          v-for="p in plugin.hookPlugins"
          :key="`hook-${p.id}`"
          :ref="(el) => setHookPluginHostRef(p.id, el)"
          :plugin="p"
          :plugin-events="plugin.pluginPipelineEvents"
          compact
          height="0px"
          @slot-mount="onHookPluginSlotMount"
        />
      </div>
    </template>
  </AppFrame>
</template>
