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
import { ref, computed, onMounted, provide, watch } from 'vue'
import AppFrame from './design/shell/AppFrame.vue'
import PrimarySidebar from './components-v2/shell/PrimarySidebar.vue'
import TopBar from './components-v2/shell/TopBar.vue'
import InspectorDrawer from './components-v2/shell/InspectorDrawer.vue'
import StorageHealthGate from './components-v2/shell/StorageHealthGate.vue'
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
import CharacterCardDetail from './components-v2/campaign/CharacterCardDetail.vue'
import MvuJsRuntime from './components/MvuJsRuntime.vue'
import CardShellHost from './components/CardShellHost.vue'
import CardShellFloatingStatus from './components/CardShellFloatingStatus.vue'
import TavernHelperRuntime from './components/TavernHelperRuntime.vue'
import PluginHost from './components/PluginHost.vue'
import { useWritingScreenAdapter } from './adapter/useWritingScreenAdapter.js'
import { useHistoryScreenAdapter } from './adapter/useHistoryScreenAdapter.js'
import { useOverviewScreenAdapter } from './adapter/useOverviewScreenAdapter.js'
import ShellAwareContent from './components-v2/st/ShellAwareContent.vue'
import MvuStatusPanel from './components-v2/st/MvuStatusPanel.vue'
import ShellVariableProposalBar from './components-v2/st/ShellVariableProposalBar.vue'
import HeavyShellDock from './components-v2/st/HeavyShellDock.vue'
import { splitHeavyThShells } from './utils/heavyShellApps.js'
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
import { useMvuStatusPanel } from './composables/useMvuStatusPanel.js'
import {
  applyCampaignOpening,
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
  setCampaignVariable,
  setCharacterVariable,
} from './tauri-api.js'
import { ST_EVENT_TYPES } from './plugin-bridge.js'
import { alertDialog } from './components/base/BaseDialog.js'
import { persistShellVariableWrite, createVariableWriteAudit } from './utils/shellVariableOutbox.js'
import {
  takeShellVariableProposal,
  upsertShellVariableProposal,
} from './utils/shellVariableProposals.js'
import { findLatestCampaignConversation } from './utils/overviewNavigation.js'
import { getOpeningShellPresentation } from './utils/cardShellPresentation.js'
import { shouldShowOpeningShell } from './utils/cardShellPresentation.js'
import { resolveCardShellManifestTarget } from './utils/cardShellPresentation.js'
import {
  buildOpeningChatSeed,
  resolveOpeningChatSelection,
  rewriteOpeningMessages,
  selectOpeningGreetingOptions,
} from './utils/cardShellOpeningChat.js'
import { buildGreetingOptionsFromDetail } from './utils/campaignGreetingOptions.js'

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
  campaign.activeCharDetail = null
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
/** Greeting texts for the opening shell's ST chat seed (first_mes + alternates). */
const cardShellOpeningGreetings = ref([])
const cardShellLabel = ref('')
const cardShellLoading = ref(false)
const cardShellShells = ref([])
const cardShellRemoteUrls = ref([])
const cardShellThCount = ref(0)
const cardShellCharacterId = ref(null)
// L7-A：重型 TH 应用（≥30K inline_js）拆给写作面 HeavyShellDock 可见挂载；
// 隐藏运行时只跑常规逻辑脚本（拆分器：utils/heavyShellApps.js）。
const cardShellSplit = computed(() => splitHeavyThShells(cardShellShells.value))
const shellVarAudit = createVariableWriteAudit(40)
const shellVarAuditTick = ref(0)
const openingShellArmed = ref(false)

function armCardShellOpening() {
  openingShellArmed.value = true
}

function disarmCardShellOpening() {
  openingShellArmed.value = false
}

watch(
  () => [writing.messages.length, writing.isWriting],
  ([messageCount, isWriting]) => {
    if (messageCount > 1 || isWriting) disarmCardShellOpening()
  },
)

const showCardShellOpening = computed(() => shouldShowOpeningShell({
  openingUrl: cardShellOpeningUrl.value,
  openingArmed: openingShellArmed.value,
  messageCount: writing.messages.length,
  isWriting: writing.isWriting,
}))
const openingShellPresentation = computed(() => getOpeningShellPresentation(
  typeof window !== 'undefined' ? window.innerHeight : 900,
))
const openingShellChatSeed = computed(() => {
  if (!showCardShellOpening.value) return null
  // Campaign 态一律用卡自己的 greetings：writing.greetingOptions 来自遗留
  // activeCharDetail，可能属于此前点过的其它角色（H2）。
  const options = selectOpeningGreetingOptions({
    writingMode: writing.writingMode,
    storeOptions: writing.greetingOptions,
    cardGreetings: cardShellOpeningGreetings.value,
  }).map((option) => (typeof option === 'string' ? { content: option } : option))
  if (!options.length && !cardShellOpeningUrl.value) return null
  return buildOpeningChatSeed(options, {
    selectedIndex: writing.selectedGreetingIndex,
    name: campaign.activeCampaign?.name
      || campaign.activeCharDetail?.name
      || campaign.activeChar?.name
      || 'Assistant',
  })
})

// The page-level opening and status surfaces own their slots in the reading
// layout. Suppress only matching display-content mounts so the same card shell
// cannot appear once as a message iframe and once in its intended position.
provide('storyforgeCardShellLayout', {
  suppressedMessageShellUrls: computed(() => {
    const urls = []
    if (cardShellOpeningUrl.value) urls.push(cardShellOpeningUrl.value)
    if (cardShellStatusUrl.value) urls.push(cardShellStatusUrl.value)
    return urls
  }),
  // H4：卡 manifest 里 InlineHtml 壳的 find_regex 触发器——消息源文命中
  // 才允许自动挂载 display 里的内联 HTML 文档（模型凭空输出的不算）。
  inlineShellTriggers: computed(() =>
    cardShellShells.value
      .filter((shell) => shell?.entry?.inline_html)
      .map((shell) => ({ label: shell.label || '', trigger: shell.trigger || '' })),
  ),
  // H3：消息内 .load 只对「卡 manifest 注册过的 URL」自动挂载；
  // 其余（如卡正则把模型输出改写成任意 .load）必须经用户确认。
  trustedMessageShellUrls: computed(() => {
    const urls = new Set()
    if (cardShellOpeningUrl.value) urls.add(cardShellOpeningUrl.value)
    if (cardShellStatusUrl.value) urls.add(cardShellStatusUrl.value)
    for (const shell of cardShellShells.value) {
      const url = shell?.entry?.remote_url?.url
      if (url) urls.add(url)
    }
    for (const url of cardShellRemoteUrls.value) {
      if (url) urls.add(url)
    }
    return [...urls]
  }),
})

// ProposeVariableUpdate（M4）：卡 JS 发起的 var_write 不再直写一等变量。
// bridge session token 对壳内任意脚本可读，消息本身证明不了来源可信；
// 写入先入提案队列，用户在确认条上显式 应用/拒绝。用户点击的 MVU 交互
// （dispatchMvuInteraction）保持直写——点击本身就是确认。
const shellVarProposals = ref([])
const shellVarApplyBusy = ref(false)

function onShellVarWrite(payload) {
  const before = shellVarProposals.value
  shellVarProposals.value = upsertShellVariableProposal(before, payload)
  if (shellVarProposals.value !== before) {
    shellVarAudit.push({ key: payload?.key, ok: null, scope: 'proposed', error: null })
    shellVarAuditTick.value++
  }
}

async function persistShellProposal(proposal) {
  const campaignId = campaign.activeCampaign?.id || null
  // 与 MVU 交互分发（useMvuStatusPanel.dispatchMvuInteraction）同判据：
  // 优先「卡绑定实例恰一个」，回退全 campaign 单实例——两条写入路径的
  // instance 作用域目标必须一致，否则同一逻辑变量会分裂在两个作用域。
  const cardBoundIds = mvuStatusSections.value.map((s) => s.instanceId)
  const ids = Object.keys(campaign.instanceNameMap || {})
  const instanceId =
    (cardBoundIds.length === 1 ? cardBoundIds[0] : null) ||
    (ids.length === 1 ? ids[0] : null)
  const result = await persistShellVariableWrite({
    campaignId,
    instanceId,
    key: proposal.key,
    value: proposal.value,
    setCampaignVariable,
    setCharacterVariable,
    log: (level, message) => logAppendFrontend(level, message),
  })
  shellVarAudit.push({
    key: proposal.key,
    ok: result.ok,
    scope: result.scope,
    error: result.error || null,
  })
  shellVarAuditTick.value++
  return result
}

async function applyShellVarProposal(id) {
  if (shellVarApplyBusy.value) return
  const { proposal, rest } = takeShellVariableProposal(shellVarProposals.value, id)
  if (!proposal) return
  shellVarApplyBusy.value = true
  try {
    shellVarProposals.value = rest
    await persistShellProposal(proposal)
  } finally {
    shellVarApplyBusy.value = false
  }
}

function rejectShellVarProposal(id) {
  const { proposal, rest } = takeShellVariableProposal(shellVarProposals.value, id)
  if (!proposal) return
  shellVarProposals.value = rest
  shellVarAudit.push({ key: proposal.key, ok: false, scope: 'rejected', error: null })
  shellVarAuditTick.value++
}

async function applyAllShellVarProposals() {
  if (shellVarApplyBusy.value) return
  shellVarApplyBusy.value = true
  try {
    const pending = shellVarProposals.value
    shellVarProposals.value = []
    for (const proposal of pending) {
      await persistShellProposal(proposal)
    }
  } finally {
    shellVarApplyBusy.value = false
  }
}

function rejectAllShellVarProposals() {
  for (const proposal of shellVarProposals.value) {
    shellVarAudit.push({ key: proposal.key, ok: false, scope: 'rejected', error: null })
  }
  shellVarProposals.value = []
  shellVarAuditTick.value++
}

async function refreshCardShellManifest() {
  cardShellStatusUrl.value = null
  cardShellOpeningUrl.value = null
  cardShellOpeningGreetings.value = []
  cardShellLabel.value = ''
  cardShellShells.value = []
  cardShellRemoteUrls.value = []
  cardShellThCount.value = 0
  cardShellCharacterId.value = null
  // 换卡/换 Campaign 后旧壳的待确认变量提案不得跨上下文生效
  shellVarProposals.value = []
  const target = resolveCardShellManifestTarget({
    activeCampaign: campaign.activeCampaign,
    activeChar: campaign.activeChar,
    activeCharDetail: campaign.activeCharDetail,
  })
  if (!target) return
  let characterId = target.kind === 'character' ? target.characterId : null
  // Campaign owns the surface: resolve its card even if a legacy character
  // remains selected from the preceding workflow.
  if (target.kind === 'campaign-card') {
    try {
      const card = await getCard(target.cardId)
      characterId = card?.source_character_id || null
      cardShellOpeningGreetings.value = buildGreetingOptionsFromDetail(card)
    } catch (e) {
      console.error('getCard for shell manifest:', e)
    }
  } else if (campaign.activeCharDetail) {
    cardShellOpeningGreetings.value = buildGreetingOptionsFromDetail(campaign.activeCharDetail)
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
    cardShellRemoteUrls.value = Array.isArray(m?.remote_urls) ? m.remote_urls : []
    cardShellThCount.value = m?.tavern_helper_count || 0
    cardShellCharacterId.value = characterId
  } catch (e) {
    console.error('getCardShellManifest:', e)
  } finally {
    cardShellLoading.value = false
  }
}

// ─── 功能面板 refs + 事件桥 ───
// CampaignPanel ref：MetaPanel mvu-applied 后调用其 refreshActiveDetailTab（App.vue:56-61 链）
const campaignPanelRef = ref(null)
const showCharDetail = ref(false)

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
    showCharDetail.value = false
    campaign.activeChar = null
    campaign.activeCharDetail = null
    writing.messages = []
    campaign.currentConversationId = null
    writing.selectedGreetingIndex = 0
    disarmCardShellOpening()
    broadcastChatChanged('character_cleared')
    return
  }
  campaign.activeChar = char
  campaign.currentConversationId = null
  armCardShellOpening()
  await loadCharDetail(char.id)
  if (campaign.activeCharDetail && char._card) {
    campaign.activeCharDetail = {
      ...campaign.activeCharDetail,
      _card: char._card,
    }
  }
  await refreshCardShellManifest()
  broadcastPluginEvent(ST_EVENT_TYPES.CHARACTER_LOADED, {
    characterId: char.id,
    name: char.name,
  })
  greeting.applySelectedOpeningMessage()
  showCharDetail.value = Boolean(campaign.activeCharDetail)
}

function enterSelectedCharacterWriting() {
  showCharDetail.value = false
  ui.viewWrite()
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

/**
 * Destiny home「开始旅程」ends by switching swipe_id and calling
 * reloadCurrentChat. Apply that selection into StoryForge UI and close the
 * setup-only opening surface.
 */
async function onOpeningShellApplied(payload) {
  const options = selectOpeningGreetingOptions({
    writingMode: writing.writingMode,
    storeOptions: writing.greetingOptions,
    cardGreetings: cardShellOpeningGreetings.value,
  })
  const selection = resolveOpeningChatSelection(payload, options)
  if (selection.greetingIndex != null && writing.writingMode === 'legacy') {
    greeting.selectGreeting(selection.greetingIndex)
  } else if (selection.content) {
    // Campaign conversations already own message history. Prefer rewriting the
    // first assistant opening in-place so the chosen scenario is visible now.
    const rewritten = rewriteOpeningMessages(writing.messages || [], selection.content)
    if (rewritten) {
      writing.messages = rewritten
      // Campaign 会话历史由后端持有：本地改写必须同步落库，否则下一轮
      // start_writing 仍基于旧开场生成，applyConversation 会静默回退。
      if (writing.writingMode === 'campaign' && campaign.activeCampaign?.id) {
        try {
          await applyCampaignOpening(campaign.activeCampaign.id, selection.content)
        } catch (e) {
          console.error('applyCampaignOpening:', e)
          logAppendFrontend('warn', `开场选择落库失败: ${e}`)
        }
      }
    } else if ((writing.messages || []).length === 0 && writing.writingMode === 'legacy') {
      writing.messages = [greeting.buildOpeningMessage(selection.content)]
    }
    if (selection.greetingIndex != null) {
      writing.selectedGreetingIndex = selection.greetingIndex
    }
    broadcastChatChanged('opening_shell_applied', {
      greetingIndex: selection.greetingIndex,
      swipe_id: payload?.swipe_id,
    })
  }
  disarmCardShellOpening()
}

// 3. conversation —— 会话装配 / 历史 / 打开 / 删除（注入范围外依赖）
const conversation = useConversation({
  loadInstanceNameMap,
  loadCharDetail,
  applySelectedOpeningMessage: greeting.applySelectedOpeningMessage,
  onConversationOpened: disarmCardShellOpening,
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
  handleRetryPostprocess,
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
  openingShellStarted: armCardShellOpening,
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
      handleRetryPostprocess,
      handleDeleteVariant,
      handleBranch,
      handleAddVariant,
      handleSwitchVariant,
    })

  // 9.5 MVU 原生状态面板（after-messages 槽）：ui_bindings 渲染 + interactions 分发
  const {
    mvuStatusSections,
    mvuInteractionMappings,
    mvuInteractionBusy,
    dispatchMvuInteraction,
  } = useMvuStatusPanel({ startWriting })

  // 10. history screen adapter —— design/history 纯展示层接线
  const { screenProps: historyScreenProps, screenEvents: historyScreenEvents } =
    useHistoryScreenAdapter({
      openConversation,
      handleDeleteConversation,
      openNewCampaign: openNewCampaignDialog,
    })

  async function continueActiveCampaignWriting() {
    const latestConversation = findLatestCampaignConversation(
      campaign.conversationHistory,
      campaign.activeCampaign?.id,
    )

    if (latestConversation) {
      await openConversation(latestConversation)
      return
    }

    armCardShellOpening()
    ui.viewWrite()
  }

  // 11. overview screen adapter —— design/overview 纯展示层接线
  const { screenProps: overviewScreenProps, screenEvents: overviewScreenEvents } =
    useOverviewScreenAdapter({
      openCampaign: () => { ui.openCampaignPanel() },
      viewHistory: () => ui.viewHistory(),
      openNewCampaign: openNewCampaignDialog,
      continueWriting: continueActiveCampaignWriting,
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
  <!-- V4 存储健康启动拦截：损坏未确认前盖住应用（写栅栏同时在后端拒绝写入） -->
  <StorageHealthGate />
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
        @open-campaign="ui.openCampaignPanel()"
        @open-char-list="ui.showCharList = true"
        @import="handleImport"
        @open-conn="ui.showConnConfig = true"
        @open-preset="ui.showPresetPanel = true"
        @open-plugin="ui.showPluginPanel = true"
        @open-agent-profile="ui.showAgentProfile = true"
        @open-meta="ui.showMetaPanel = true"
      >
        <template #runtime>
          <!-- L7-A：隐藏运行时只跑轻量逻辑脚本；重型可见应用归写作面 dock -->
          <TavernHelperRuntime
            v-if="cardShellThCount"
            :shells="cardShellSplit.light"
            :character-id="cardShellCharacterId"
            :show-status="true"
            :auto-run="true"
            placement="sidebar"
            @var-write="onShellVarWrite"
          />
        </template>
      </PrimarySidebar>
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
        :show-opening="showCardShellOpening"
        v-on="writingScreenEvents"
      >
        <template #opening>
          <CardShellHost
            v-if="showCardShellOpening"
            :url="cardShellOpeningUrl"
            :campaign-id="campaign.activeCampaign?.id || null"
            label="序章"
            :height="openingShellPresentation.height"
            :auto-height="openingShellPresentation.autoHeight"
            :opening-chat-seed="openingShellChatSeed"
            root-class="overflow-hidden rounded-xl border border-line bg-surface shadow-rise"
            @var-write="onShellVarWrite"
            @opening-applied="onOpeningShellApplied"
          />
        </template>
        <template #after-messages>
          <!-- L7-A：重型卡应用折叠 dock（默认收起，展开即挂载，一次一个活跃） -->
          <HeavyShellDock
            :shells="cardShellSplit.heavy"
            :character-id="cardShellCharacterId"
            @var-write="onShellVarWrite"
          />
          <ShellVariableProposalBar
            :proposals="shellVarProposals"
            :busy="shellVarApplyBusy"
            @apply="applyShellVarProposal"
            @reject="rejectShellVarProposal"
            @apply-all="applyAllShellVarProposals"
            @reject-all="rejectAllShellVarProposals"
          />
          <MvuStatusPanel
            :sections="mvuStatusSections"
            :interactions="mvuInteractionMappings"
            :busy="mvuInteractionBusy || writing.isWriting"
            @interact="dispatchMvuInteraction"
          />
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
      <CardShellFloatingStatus
        v-if="ui.currentView === 'write' && cardShellStatusUrl"
      >
        <CardShellHost
          :url="cardShellStatusUrl"
          :campaign-id="campaign.activeCampaign?.id || null"
          label="状态栏"
          height="100%"
          root-class="h-full"
          :show-status-line="false"
          @var-write="onShellVarWrite"
        />
      </CardShellFloatingStatus>

      <!-- ═══ 管理面板（每个用 ui.show* 控制；close 同时关面板并恢复侧栏） ═══ -->

      <!-- 角色卡列表（保留原位组件,未迁 v2） -->
      <CharacterList
        v-if="ui.showCharList"
        :active-id="campaign.activeChar?.id"
        @select="handleSelectChar"
        @close="ui.showCharList = false; ui.showSidebar = true"
      />

      <CharacterCardDetail
        v-if="showCharDetail && campaign.activeCharDetail"
        :character="campaign.activeCharDetail"
        @close="showCharDetail = false"
        @write="enterSelectedCharacterWriting"
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
        :initial-tab="ui.campaignPanelTab"
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
        :refresh-card-shell-manifest="refreshCardShellManifest"
        :opening-shell-started="armCardShellOpening"
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
