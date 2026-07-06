<script setup>
import { ref, reactive, computed, onMounted, nextTick } from 'vue'
import AppSidebar from './components/AppSidebar.vue'
import BaseOverlay from './components/base/BaseOverlay.vue'
import ChatMessage from './components/ChatMessage.vue'
import Composer from './components/Composer.vue'
import StreamingMessage from './components/StreamingMessage.vue'
import DebugDrawer from './components/DebugDrawer.vue'
import CharacterList from './components/CharacterList.vue'
import ConnectionConfig from './components/ConnectionConfig.vue'
import CampaignPanel from './components/CampaignPanel.vue'
import PresetPanel from './components/PresetPanel.vue'
import PluginPanel from './components/PluginPanel.vue'
import MetaPanel from './components/MetaPanel.vue'
import MvuJsRuntime from './components/MvuJsRuntime.vue'
import { alertDialog, confirmDialog } from './components/base/BaseDialog.js'
import { importCharacter, getCharacter, getVersion, startWriting as apiStartWriting, cancelWriting as apiCancelWriting, regenerate as apiRegenerate, getActiveConnection, editVariant as apiEditVariant, acceptVariant as apiAcceptVariant, softDeleteVariant as apiSoftDeleteVariant, deleteMessageFrom as apiDeleteMessageFrom, addVariant as apiAddVariant, switchVariant as apiSwitchVariant, listConversations, deleteConversation, getConversation, logAppendFrontend, getActiveCampaign, listCards, getCard, createCampaign, forkCampaign, setActiveCampaign, listInstances, listPlugins, extractCharacters } from './tauri-api.js'
import { ST_EVENT_TYPES } from './plugin-bridge.js'
import { findLastAssistantConversationNode } from './utils/conversationNodes.js'

const powerMode = ref(false)
const messages = ref([])
const showPipeline = ref(false)
const appVersion = ref('...')

// 当前活跃的角色卡
const activeChar = ref(null)
const activeCharDetail = ref(null)
const importError = ref('')
const showCharList = ref(false)
const showCampaignPanel = ref(false)
const showMetaPanel = ref(false)
const activeCampaign = ref(null)
const campaignPanelRef = ref(null) // template ref for CampaignPanel refresh

// ─── 实例名映射（instance_id → display_name，供 Pipeline trace 显示） ───
const instanceNameMap = ref({})
async function loadInstanceNameMap() {
  if (!activeCampaign.value) { instanceNameMap.value = {}; return }
  try {
    const insts = await listInstances(activeCampaign.value.id)
    const map = {}
    for (const inst of insts) {
      if (inst.id) map[inst.id] = inst.name || inst.character_name || ''
    }
    instanceNameMap.value = map
  } catch (e) {
    console.error('加载实例名映射失败:', e)
  }
}

// ─── MVU Apply 成功后刷新 Campaign 变量 tab ───
function handleMvuApplied() {
  if (campaignPanelRef.value?.refreshActiveDetailTab) {
    campaignPanelRef.value.refreshActiveDetailTab()
  }
}

const showPresetPanel = ref(false)
const showPluginPanel = ref(false)
const showDebugDrawer = ref(false) // 移动端调试抽屉（桌面常驻无需此开关）
const showSidebar = ref(false) // 移动端左导航抽屉

// 中间栏视图：'write' 写作 | 'history' 会话历史 | 'overview' Campaign 概览
// 由 showHistory/activeCampaignOverview 派生，统一成 currentView 供左导航高亮
const currentView = computed(() => {
  if (showHistory.value && activeCampaign.value && activeCampaignOverview.value) return 'overview'
  if (showHistory.value) return 'history'
  return 'write'
})

// 中间栏标题
const pageTitle = computed(() => {
  if (currentView.value === 'overview') return activeCampaign.value?.name || 'Campaign'
  if (currentView.value === 'history') return '会话历史'
  if (writingMode.value === 'campaign') return activeCampaign.value?.name || 'Campaign 写作'
  if (writingMode.value === 'legacy') return activeChar.value?.name || '写作'
  return 'StoryForge'
})

// 流式消息的角色标签
const streamingRoleLabel = computed(() =>
  writingMode.value === 'campaign'
    ? (activeCampaign.value?.name || 'AI')
    : (activeChar.value?.name || 'AI')
)

function viewHistory() { showHistory.value = true; activeCampaignOverview.value = false }
function viewOverview() { showHistory.value = true; activeCampaignOverview.value = true }
function viewWrite() { showHistory.value = false }

// 高玩开关 = 模态总闸：开启时进入专业调试模态（桌面右栏自动出现，移动端弹抽屉）
function onTogglePower() {
  powerMode.value = !powerMode.value
  if (powerMode.value) showDebugDrawer.value = true
}

const sidebarPlugins = ref([])
const showSidebarPlugins = ref(false)
const pluginPipelineEvents = ref([])
let pluginPipelineEventSeq = 0
const MAX_PLUGIN_PIPELINE_EVENTS = 100

// 加载侧栏插件
async function loadSidebarPlugins() {
  try {
    const all = await listPlugins()
    sidebarPlugins.value = (all || []).filter(p => p.enabled && p.ui_slots?.includes('SidebarPanel'))
  } catch (e) {
    console.error('加载侧栏插件失败:', e)
    logAppendFrontend('error', `loadSidebarPlugins: ${e}`).catch(() => {})
  }
}

function pushPluginEventRecord(record) {
  pluginPipelineEventSeq += 1
  const nextEvents = [
    ...pluginPipelineEvents.value,
    { id: pluginPipelineEventSeq, ...record },
  ]
  pluginPipelineEvents.value = nextEvents.slice(-MAX_PLUGIN_PIPELINE_EVENTS)
}

function broadcastPluginPipelineEvent(event) {
  if (!event?.event_type) return
  pushPluginEventRecord({ event })
}

function broadcastPluginEvent(event, data = {}) {
  if (!event) return
  pushPluginEventRecord({ event, data })
}

function activeVariantForMessage(message) {
  return message?.variants?.[message.active_variant] || message?.variants?.[0] || null
}

function chatEventPayload(extra = {}) {
  return {
    conversationId: currentConversationId.value,
    messageCount: messages.value.length,
    writingMode: writingMode.value,
    campaignId: activeCampaign.value?.id || null,
    characterId: activeChar.value?.id || null,
    ...extra,
  }
}

function messageEventPayload(messageId, extra = {}) {
  const message = messages.value.find((m) => m.id === messageId)
  const variant = activeVariantForMessage(message)
  return chatEventPayload({
    messageId,
    role: message?.role || null,
    variantId: variant?.id || null,
    content: variant?.content || '',
    displayContent: variant?.display_content || variant?.content || '',
    ...extra,
  })
}

function broadcastChatChanged(reason, extra = {}) {
  broadcastPluginEvent(ST_EVENT_TYPES.CHAT_CHANGED, chatEventPayload({ reason, ...extra }))
}

// 流水线状态
const pipeline = reactive({
  state: 'idle',
  stateLabel: '',
  director: { status: 'idle', detail: '', output: '' },
  subagents: [],
  editor: { status: 'idle', detail: '', output: '' },
  postprocess: { status: 'idle', detail: '', knowledge: 0, variable: 0, task: 0, reason: '' },
})

// 消息列表容器 ref（自动滚动用）
const messagesContainer = ref(null)

// 滚动到底部（新消息/写作完成后调用）
function scrollToBottom() {
  nextTick(() => {
    if (messagesContainer.value) {
      messagesContainer.value.scrollTop = messagesContainer.value.scrollHeight
    }
  })
}

// 写作模式：campaign = Campaign 写作，legacy = ST 单卡兼容，none = 无法写作
const writingMode = computed(() => {
  if (activeCampaign.value) return 'campaign'
  if (activeChar.value) return 'legacy'
  return 'none'
})

// 获取 AI 消息的 role_label（Campaign 模式用 Campaign 名，legacy 用角色名）
function getAssistantRoleLabel() {
  if (writingMode.value === 'campaign') return activeCampaign.value?.name || 'AI'
  return activeChar.value?.name || 'AI'
}

// 是否有流水线运行中（用于禁用重 roll / 显示停止按钮）
const isWriting = ref(false)
// 当前对话 ID（重 roll 需要）
const currentConversationId = ref(null)
const lastConversationNode = computed(() =>
  findLastAssistantConversationNode(messages.value, currentConversationId.value)
)
const selectedGreetingIndex = ref(0)
function buildGreetingOptionsFromDetail(detail) {
  if (!detail) return []

  const options = []
  const seen = new Set()
  const addOption = (label, content) => {
    if (!content || !content.trim() || seen.has(content)) return
    seen.add(content)
    options.push({ label, content })
  }

  addOption('默认', detail.first_mes)
  const alternates = detail.alternate_greetings || []
  alternates.forEach((content, index) => {
    addOption(`备选 ${index + 1}`, content)
  })
  return options
}
const greetingOptions = computed(() => buildGreetingOptionsFromDetail(activeCharDetail.value))
const selectedGreeting = computed(() => greetingOptions.value[selectedGreetingIndex.value] || null)
const canChooseGreeting = computed(() =>
  writingMode.value === 'legacy'
  && !currentConversationId.value
  && greetingOptions.value.length > 1
)
// 会话历史列表
const conversationHistory = ref([])
const showHistory = ref(true)
// 有 active campaign 时首屏显示 campaign 概览（而非会话历史）
const activeCampaignOverview = ref(true)
// 连接配置弹层
const showConnConfig = ref(false)
// 当前活跃连接（顶栏显示用）
const activeConnection = ref(null)
// AgentConfigCard 引用（连接变更后刷新）—— 现由 DebugDrawer 持有，转发调用
const debugDrawerRef = ref(null)
const agentConfigRef = computed(() => debugDrawerRef.value)

function normalizeGreetingSelection() {
  if (selectedGreetingIndex.value >= greetingOptions.value.length) {
    selectedGreetingIndex.value = 0
  }
}

function buildOpeningMessage(content) {
  return {
    id: 'm1',
    role: 'assistant',
    role_label: activeCharDetail.value?.name || getAssistantRoleLabel(),
    active_variant: 0,
    variants: [{
      id: 'v1',
      content,
      display_content: content,
      status: 'final',
      provenance: null,
    }],
  }
}

function applySelectedOpeningMessage() {
  normalizeGreetingSelection()
  if (writingMode.value !== 'legacy') {
    messages.value = []
    broadcastChatChanged('opening_message_cleared')
    return
  }
  const content = selectedGreeting.value?.content
  messages.value = content ? [buildOpeningMessage(content)] : []
  broadcastChatChanged('opening_message_selected', { greetingIndex: selectedGreetingIndex.value })
  scrollToBottom()
}

function selectGreeting(index) {
  selectedGreetingIndex.value = index
  applySelectedOpeningMessage()
}

// 获取版本 + 加载会话历史列表 + 拦截 console
onMounted(async () => {
  try { appVersion.value = await getVersion() } catch (e) { console.error('getVersion:', e) }
  await refreshActiveConnection()
  await loadConversationHistory()
  try {
    activeCampaign.value = await getActiveCampaign()
    await loadInstanceNameMap()
  } catch (e) { console.error('getActiveCampaign:', e) }
  await loadSidebarPlugins()
  setupConsoleForwarding()
  broadcastPluginEvent(ST_EVENT_TYPES.APP_READY, chatEventPayload({ version: appVersion.value }))
})

// 拦截 console.log/warn/error，转发到后端 LogStore
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

// 把后端 Conversation 应用到前端 messages（设 id + 转换 nodes → messages）
// 复用点：恢复对话、重 roll 后刷新、删除后刷新（单一事实源，避免前端臆测 variant 数组）
function applyConversation(conv) {
  currentConversationId.value = conv.id
  messages.value = conv.nodes
    // 隐藏「所有 variant 都 Discarded」的 node（删除后该消息整体消失）
    .filter((node) => node.variants.some((v) => v.status !== 'Discarded'))
    .map((node) => {
      const active = node.variants[node.active_variant] || node.variants[0]
      return {
        id: node.id,
        role: active.role === 'User' ? 'user' : 'assistant',
        role_label: active.role === 'User' ? '我' : 'AI',
        active_variant: node.active_variant,
        variants: node.variants.map((v) => ({
          id: v.id,
          content: v.content,
          display_content: v.display_content ?? v.content,
          status: v.status === 'Final' ? 'final' : v.status === 'Discarded' ? 'discarded' : 'draft',
          provenance: v.provenance,
        })),
      }
    })
  broadcastChatChanged('conversation_applied', { conversationId: conv.id })
}

// 加载会话历史列表
async function loadConversationHistory() {
  try {
    const convList = await listConversations()
    if (!convList || convList.length === 0) {
      conversationHistory.value = []
      return
    }
    convList.sort((a, b) => b.updated_at.localeCompare(a.updated_at))
    conversationHistory.value = convList
  } catch (e) {
    console.error('加载会话历史失败:', e)
  }
}

// 删除会话记录
async function handleDeleteConversation(conv, event) {
  if (event) event.stopPropagation()
  const ok = await confirmDialog(`确定删除该会话？会话 ${conv.id?.slice(0, 8)} 的所有消息将被清除。`, { title: '删除确认' })
  if (!ok) return
  try {
    await deleteConversation(conv.id)
    // 若删的是当前会话，清空当前对话
    if (conv.id === currentConversationId.value) {
      messages.value = []
      currentConversationId.value = null
      broadcastChatChanged('conversation_deleted', { conversationId: conv.id })
    }
    await loadConversationHistory()
  } catch (e) {
    console.error('删除会话失败:', e)
    await alertDialog('删除会话失败: ' + e)
  }
}

// 打开一个会话
async function openConversation(convSummary) {
  try {
    const conv = await getConversation(convSummary.id)
    if (!conv) return

    applyConversation(conv)
    currentConversationId.value = convSummary.id
    showHistory.value = false

    // 一 Campaign 一对话：切到该会话的 Campaign
    if (convSummary.campaign_id) {
      try {
        await setActiveCampaign(convSummary.campaign_id)
        activeCampaign.value = await getActiveCampaign()
        await loadInstanceNameMap()
        // role_label 用 Campaign 名
        messages.value.forEach((m) => {
          if (m.role === 'assistant') m.role_label = activeCampaign.value?.name || 'AI'
        })
      } catch (e) { console.error('切换 Campaign 失败:', e) }
    } else if (conv.character_id) {
      // legacy 会话（无 Campaign 绑定）：加载关联角色卡
      try {
        const chars = await import('./tauri-api.js').then(m => m.listCharacters())
        const char = chars.find((c) => c.id === conv.character_id)
        if (char) {
          activeChar.value = char
          await loadCharDetail(char.id)
          messages.value.forEach((m) => {
            if (m.role === 'assistant') m.role_label = char.name
          })
        }
      } catch (e) { console.error('加载关联角色卡失败:', e) }
    }
    broadcastPluginEvent(ST_EVENT_TYPES.CHAT_LOADED, chatEventPayload({
      conversationId: currentConversationId.value,
      campaignId: convSummary.campaign_id || null,
      characterId: conv.character_id || null,
    }))
  } catch (e) {
    console.error('打开对话失败:', e)
  }
}

// 新建对话（从历史列表点「新对话」）
function startNewConversation() {
  messages.value = []
  currentConversationId.value = null
  showHistory.value = false
  applySelectedOpeningMessage()
  broadcastPluginEvent(ST_EVENT_TYPES.CHAT_LOADED, chatEventPayload({ reason: 'new_conversation' }))
}

// ─── 新建 Campaign（一 Campaign 一对话：建 Campaign 自动建对话+开场白） ───
const showNewCampaignForm = ref(false)
const newCampaignCards = ref([])
const newCampaignCardId = ref(null)
const newCampaignCardDetail = ref(null)
const newCampaignName = ref('')
const newCampaignGreetingIndex = ref(0)
const creatingCampaign = ref(false)
const newCampaignGreetingOptions = computed(() => buildGreetingOptionsFromDetail(newCampaignCardDetail.value))
const selectedNewCampaignGreeting = computed(() => newCampaignGreetingOptions.value[newCampaignGreetingIndex.value] || null)

function normalizeNewCampaignGreetingSelection() {
  if (newCampaignGreetingIndex.value >= newCampaignGreetingOptions.value.length) {
    newCampaignGreetingIndex.value = 0
  }
}

async function loadNewCampaignCardDetail() {
  newCampaignCardDetail.value = null
  newCampaignGreetingIndex.value = 0
  if (!newCampaignCardId.value) return
  try {
    newCampaignCardDetail.value = await getCard(newCampaignCardId.value)
    normalizeNewCampaignGreetingSelection()
  } catch (e) {
    console.error('加载 Campaign 开场白失败:', e)
  }
}

async function openNewCampaignDialog() {
  showNewCampaignForm.value = true
  newCampaignName.value = ''
  newCampaignCardDetail.value = null
  newCampaignGreetingIndex.value = 0
  try {
    newCampaignCards.value = await listCards()
    // 默认选第一张已识别的卡
    const firstExtracted = newCampaignCards.value.find(c => c.extracted) || newCampaignCards.value[0]
    newCampaignCardId.value = firstExtracted?.id || null
    await loadNewCampaignCardDetail()
  } catch (e) { console.error('加载角色卡列表失败:', e) }
}

async function handleCreateCampaign() {
  if (!newCampaignCardId.value || !newCampaignName.value.trim()) return
  creatingCampaign.value = true
  try {
    const result = await createCampaign(
      newCampaignCardId.value,
      newCampaignName.value.trim(),
      selectedNewCampaignGreeting.value?.content || null,
    )
    await setActiveCampaign(result.id)
    activeCampaign.value = await getActiveCampaign()
    await loadInstanceNameMap()
    showNewCampaignForm.value = false
    // 加载该 Campaign 绑定的对话（create_campaign 已自动建+开场白）
    if (result.conversation_id) {
      const conv = await getConversation(result.conversation_id)
      if (conv) {
        applyConversation(conv)
        // role_label 用 Campaign 名
        messages.value.forEach((m) => {
          if (m.role === 'assistant') m.role_label = activeCampaign.value?.name || 'AI'
        })
        broadcastPluginEvent(ST_EVENT_TYPES.CHAT_LOADED, chatEventPayload({
          reason: 'campaign_created',
          conversationId: result.conversation_id,
          campaignId: result.id,
        }))
      }
    }
    showHistory.value = false // 切到写作视图
    await loadConversationHistory()
  } catch (e) {
    console.error('创建 Campaign 失败:', e)
    await alertDialog('创建 Campaign 失败: ' + e)
  } finally {
    creatingCampaign.value = false
  }
}

// 刷新活跃连接状态（连接配置变更后调用）
async function refreshActiveConnection() {
  try {
    activeConnection.value = await getActiveConnection()
  } catch (e) {
    console.error('加载活跃连接失败:', e)
  }
  // 同步刷新 AgentConfigCard 的连接列表
  agentConfigRef.value?.loadConnections?.()
}

// 导入角色卡
async function handleImport() {
  importError.value = ''
  try {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const filePath = await open({
      multiple: false,
      filters: [{ name: '角色卡', extensions: ['png', 'json'] }],
    })
    if (!filePath) return

    const { readFile } = await import('@tauri-apps/plugin-fs')
    const data = await readFile(filePath)
    const result = await importCharacter(data)

    // 自动触发角色识别（写入 CampaignStore.cards.json，供 Campaign 面板使用）
    // 失败不阻塞导入主流程：识别失败时 Campaign 面板可手动重试
    extractCharacters(result.id).catch((e) => {
      console.error('角色识别失败（不影响导入，可在 Campaign 面板重试）:', e)
    })

    // 导入成功，设为当前活跃角色
    activeChar.value = result
    currentConversationId.value = null
    // 加载详情
    await loadCharDetail(result.id)
    broadcastPluginEvent(ST_EVENT_TYPES.CHARACTER_LOADED, {
      characterId: result.id,
      name: result.name,
    })

    // 用角色的开场白替换消息列表
    applySelectedOpeningMessage()
  } catch (err) {
    importError.value = String(err)
  }
}

// 加载角色详情（含世界书）
async function loadCharDetail(id) {
  try {
    activeCharDetail.value = await getCharacter(id)
    selectedGreetingIndex.value = 0
    normalizeGreetingSelection()
  } catch (e) {
    console.error('加载详情失败:', e)
  }
}

// 从列表选择角色
async function handleSelectChar(char) {
  showCharList.value = false
  if (!char) {
    activeChar.value = null
    activeCharDetail.value = null
    messages.value = []
    currentConversationId.value = null
    selectedGreetingIndex.value = 0
    broadcastChatChanged('character_cleared')
    return
  }
  activeChar.value = char
  currentConversationId.value = null
  await loadCharDetail(char.id)
  broadcastPluginEvent(ST_EVENT_TYPES.CHARACTER_LOADED, {
    characterId: char.id,
    name: char.name,
  })

  // 用角色的开场白替换消息列表
  applySelectedOpeningMessage()
}

// 写作流水线（调用后端真实流水线，通过 Channel 接收事件）
async function startWriting(intent, skipLocalPush = false) {
  // 写作前检查：必须有活跃连接
  if (!activeConnection.value) {
    showConnConfig.value = true
    return
  }
  // 三态检查：有 Campaign → Campaign 写作；无 Campaign 有角色 → legacy；都没有 → 阻止
  if (writingMode.value === 'none') {
    await alertDialog('请先导入角色卡或打开一个 Campaign，再开始写作。')
    return
  }
  // 防止并发写入
  if (isWriting.value) return
  showPipeline.value = true
  isWriting.value = true
  pipeline.state = 'running'
  pipeline.stateLabel = '准备中'
  pipeline.director = { status: 'idle', detail: '', output: '' }
  pipeline.subagents = []
  pipeline.editor = { status: 'idle', detail: '', output: '' }
  pipeline.postprocess = { status: 'idle', detail: '', knowledge: 0, variable: 0, task: 0, reason: '' }
  // 清除编剧流式消息占位（上次写作残留）
  messages.value = messages.value.filter((m) => m.id !== 'editor-streaming')

  // 本地 push user 消息（即时反馈，skipLocalPush 时跳过——用户消息已在列表中）
  let userMsgId = null
  if (!skipLocalPush) {
    userMsgId = `user-${Date.now()}`
    messages.value.push({
      id: userMsgId,
      role: 'user',
      role_label: '我',
      active_variant: 0,
      variants: [{
        id: `uv-${Date.now()}`,
        content: intent,
        display_content: intent,
        status: 'final',
        provenance: null,
      }],
    })
    broadcastPluginEvent(ST_EVENT_TYPES.MESSAGE_SENT, messageEventPayload(userMsgId, { content: intent }))
    scrollToBottom()
  }

  try {
    // Campaign 模式不传 characterId（后端从 active campaign 装配 runtime）；
    // legacy 模式传 activeChar.id 保持旧命令兼容
    const charIdForWriting = writingMode.value === 'campaign' ? null : activeChar.value?.id
    const openingMessage = writingMode.value === 'legacy' && !currentConversationId.value
      ? selectedGreeting.value?.content || null
      : null
    const result = await apiStartWriting(intent, charIdForWriting, (event) => {
      handlePipelineEvent(event)
    }, currentConversationId.value, openingMessage)

    // 后端已存开场白、user 意图和 AI 成文；重拉会话以拿到展示态 regex 内容。
    const text = result.text
    const msgId = result.node_id || `msg-${Date.now()}`
    currentConversationId.value = result.conversation_id

    // 替换编剧流式占位。
    const streamingIdx = messages.value.findIndex((m) => m.id === 'editor-streaming')
    if (streamingIdx >= 0) {
      messages.value.splice(streamingIdx, 1)
    }
    const refreshed = await getConversation(currentConversationId.value)
    if (refreshed) {
      applyConversation(refreshed)
      messages.value.forEach((m) => {
        if (m.role === 'assistant') m.role_label = getAssistantRoleLabel()
      })
      broadcastPluginEvent(ST_EVENT_TYPES.MESSAGE_RECEIVED, messageEventPayload(msgId, {
        reason: 'writing_complete',
        content: text,
      }))
    } else {
      messages.value.push({
        id: msgId,
        role: 'assistant',
        role_label: getAssistantRoleLabel(),
        active_variant: 0,
        variants: [{
          id: `v-${Date.now()}`,
          content: text,
          display_content: text,
          status: 'final',
          provenance: null,
        }],
      })
      broadcastPluginEvent(ST_EVENT_TYPES.MESSAGE_RECEIVED, messageEventPayload(msgId, {
        reason: 'writing_complete',
        content: text,
      }))
    }

    pipeline.state = 'done'
    pipeline.stateLabel = '已完成'
    showPipeline.value = false // 写作完成，收起 StreamingMessage（成文消息已 push）
    loadInstanceNameMap() // 刷新实例名映射（可能新增临时实例）
    scrollToBottom()
  } catch (err) {
    // 失败回滚：移除已 push 的用户消息（无对应 AI 回复，残留会误导重试）
    if (userMsgId) {
      messages.value = messages.value.filter((m) => m.id !== userMsgId)
    }
    messages.value = messages.value.filter((m) => m.id !== 'editor-streaming')
    pipeline.state = 'error'
    pipeline.stateLabel = `失败: ${err}`
  } finally {
    isWriting.value = false
  }
}

// 取消当前写作
async function cancelWriting() {
  try {
    await apiCancelWriting()
    pipeline.stateLabel = '正在停止…'
  } catch (e) {
    console.error('取消失败:', e)
  }
}

// 重 roll（整体/只重编剧/只重某子 Agent，可附 hint）
async function handleReroll({ messageId, kind, hint }) {
  // 找到消息
  const msg = messages.value.find((m) => m.id === messageId)
  if (!msg) return
  if (!currentConversationId.value) {
    await alertDialog('无对话上下文，无法重 roll')
    return
  }

  showPipeline.value = true
  isWriting.value = true
  pipeline.state = 'running'
  pipeline.stateLabel = `重 roll（${kind === 'all' ? '整体' : kind}）`
  pipeline.director = { status: 'idle', detail: '', output: '' }
  pipeline.subagents = []
  pipeline.editor = { status: 'idle', detail: '', output: '' }
  pipeline.postprocess = { status: 'idle', detail: '', knowledge: 0, variable: 0, task: 0, reason: '' }
  messages.value = messages.value.filter((m) => m.id !== 'editor-streaming')

  // 构造 targets
  let targets = []
  if (kind === 'director' || kind === 'all') {
    targets = [] // 空表示整体重 roll
  } else if (kind === 'editor') {
    targets = [{ kind: 'editor' }]
  } else if (kind.startsWith('subagent:')) {
    targets = [{ kind }]
  }

  try {
    const result = await apiRegenerate({
      conversationId: currentConversationId.value,
      nodeId: messageId,
      targets,
      hint,
    }, (event) => handlePipelineEvent(event))

    // 重拉对话刷新 UI（单一事实源）：后端按「最后一条 → 原地替换 / 中间 → 开分支」
    // 落库，前端不臆测 variant 数组，直接以后端真实状态为准。
    const refreshed = await getConversation(currentConversationId.value)
    if (refreshed) {
      applyConversation(refreshed)
      // 保留 role_label 覆盖（applyConversation 重置为 AI/我）
      messages.value.forEach((m) => {
        if (m.role === 'assistant') m.role_label = getAssistantRoleLabel()
      })
      broadcastPluginEvent(ST_EVENT_TYPES.MESSAGE_UPDATED, messageEventPayload(messageId, { reason: 'reroll', kind }))
    }

    pipeline.state = 'done'
    pipeline.stateLabel = '重 roll 完成'
    showPipeline.value = false // 收起 StreamingMessage
    loadInstanceNameMap()
    // result 含最终成文，但 UI 已由 refreshed 驱动，无需单独消费
    void result
  } catch (err) {
    pipeline.state = 'error'
    pipeline.stateLabel = `重 roll 失败: ${err}`
  } finally {
    isWriting.value = false
  }
}

// ─── 对话操作（编辑/删除/采纳/分支）──────────────────────────────────────

// 编辑变体内容
async function handleEditVariant({ nodeId, newContent }) {
  if (!currentConversationId.value) return
  try {
    await apiEditVariant(currentConversationId.value, nodeId, newContent)
    const refreshed = await getConversation(currentConversationId.value)
    if (refreshed) {
      applyConversation(refreshed)
      messages.value.forEach((m) => {
        if (m.role === 'assistant') m.role_label = getAssistantRoleLabel()
      })
      broadcastPluginEvent(ST_EVENT_TYPES.MESSAGE_UPDATED, messageEventPayload(nodeId, { reason: 'edit' }))
    }
  } catch (e) {
    console.error('编辑失败:', e)
  }
}

// 采纳变体（Draft → Final）
async function handleAcceptVariant({ nodeId }) {
  if (!currentConversationId.value) return
  try {
    await apiAcceptVariant(currentConversationId.value, nodeId)
    const msg = messages.value.find((m) => m.id === nodeId)
    if (msg) {
      const variant = msg.variants[msg.active_variant]
      if (variant) variant.status = 'final'
      broadcastPluginEvent(ST_EVENT_TYPES.MESSAGE_UPDATED, messageEventPayload(nodeId, { reason: 'accept_variant' }))
    }
  } catch (e) {
    console.error('采纳失败:', e)
  }
}

// 删除消息 = 删该条及其后所有（撤销从这条开始的写作）+ 清流水线状态
async function handleDeleteVariant({ nodeId }) {
  if (!currentConversationId.value) return
  try {
    await apiDeleteMessageFrom(currentConversationId.value, nodeId)
    // 重新拉取对话刷新（truncate 后该消息及之后都消失）
    const refreshed = await getConversation(currentConversationId.value)
    if (refreshed) {
      applyConversation(refreshed)
      messages.value.forEach((m) => {
        if (m.role === 'assistant') m.role_label = getAssistantRoleLabel()
      })
      broadcastPluginEvent(ST_EVENT_TYPES.MESSAGE_DELETED, chatEventPayload({ messageId: nodeId }))
    }
    // 清流水线状态（删除 = 回到这条之前的状态，上次写作的导演/子Agent/编剧输出作废）
    pipeline.state = 'idle'
    pipeline.stateLabel = ''
    pipeline.director = { status: 'idle', detail: '', output: '' }
    pipeline.subagents = []
    pipeline.editor = { status: 'idle', detail: '', output: '' }
    pipeline.postprocess = { status: 'idle', detail: '', knowledge: 0, variable: 0, task: 0, reason: '' }
    showPipeline.value = false
  } catch (e) {
    console.error('删除失败:', e)
    await alertDialog('删除失败: ' + e)
  }
}

// user 消息重 roll：找到下一条 AI 消息，用 user 的 intent 作为 hint 调 regenerate
// 如果 AI 消息已被删除，直接用 intent 重新写作（追加到当前对话）
async function handleRerollUser({ messageId }) {
  if (isWriting.value) return
  const userMsg = messages.value.find((m) => m.id === messageId)
  if (!userMsg) return
  const intent = userMsg.variants[userMsg.active_variant]?.content
  if (!intent) return
  // 找到紧随其后的 AI 消息（regenerate 的目标）
  const userIndex = messages.value.findIndex((m) => m.id === messageId)
  const aiMsg = messages.value.slice(userIndex + 1).find((m) => m.role === 'assistant')
  // 没有 AI 消息（已被删除）→ 直接用 intent 重新写作（跳过本地 push，u3 已在列表中）
  if (!aiMsg) {
    await startWriting(intent, true)
    return
  }

  showPipeline.value = true
  isWriting.value = true
  pipeline.state = 'running'
  pipeline.stateLabel = '重 roll（整体）'
  pipeline.director = { status: 'idle', detail: '', output: '' }
  pipeline.subagents = []
  pipeline.editor = { status: 'idle', detail: '', output: '' }
  pipeline.postprocess = { status: 'idle', detail: '', knowledge: 0, variable: 0, task: 0, reason: '' }
  messages.value = messages.value.filter((m) => m.id !== 'editor-streaming')

  try {
    await apiRegenerate({
      conversationId: currentConversationId.value,
      nodeId: aiMsg.id,
      targets: [],       // 空 = 整体重 roll
      hint: intent,       // user 的意图作为 hint 注入导演+编剧
    }, (event) => handlePipelineEvent(event))

    // 重拉对话刷新（regenerate 替换了 AI 消息的 variant）
    const refreshed = await getConversation(currentConversationId.value)
    if (refreshed) {
      applyConversation(refreshed)
      messages.value.forEach((m) => {
        if (m.role === 'assistant') m.role_label = getAssistantRoleLabel()
      })
      broadcastPluginEvent(ST_EVENT_TYPES.MESSAGE_UPDATED, messageEventPayload(aiMsg.id, { reason: 'reroll_user' }))
    }
    messages.value = messages.value.filter((m) => m.id !== 'editor-streaming')
    pipeline.state = 'done'
    pipeline.stateLabel = '重 roll 完成'
    showPipeline.value = false // 收起 StreamingMessage
    loadInstanceNameMap()
    scrollToBottom()
  } catch (err) {
    pipeline.state = 'error'
    pipeline.stateLabel = `重 roll 失败: ${err}`
  } finally {
    isWriting.value = false
  }
}

// 添加新变体（分支）
function makeForkCampaignName() {
  const baseName = activeCampaign.value?.name || 'Campaign'
  const stamp = new Date().toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  })
  return `${baseName} 分支 ${stamp}`
}

async function handleBranch({ nodeId }) {
  if (isWriting.value) return
  if (!activeCampaign.value || !currentConversationId.value) {
    await alertDialog('请先打开 Campaign 对话，再创建分支。')
    return
  }

  try {
    const result = await forkCampaign(activeCampaign.value.id, nodeId, makeForkCampaignName())
    await setActiveCampaign(result.id)
    activeCampaign.value = await getActiveCampaign()
    await loadInstanceNameMap()

    if (result.conversation_id) {
      const conv = await getConversation(result.conversation_id)
      if (conv) {
        applyConversation(conv)
        messages.value.forEach((m) => {
          if (m.role === 'assistant') m.role_label = getAssistantRoleLabel()
        })
      }
    }

    showHistory.value = false
    activeCampaignOverview.value = false
    await loadConversationHistory()
    broadcastPluginEvent(ST_EVENT_TYPES.CHAT_LOADED, chatEventPayload({
      reason: 'campaign_forked',
      campaignId: result.id,
      conversationId: result.conversation_id || currentConversationId.value,
      forkNodeId: nodeId,
      sourceCampaignId: result.fork_from?.[0] || null,
    }))
  } catch (e) {
    console.error('创建分支失败:', e)
    await alertDialog('创建分支失败: ' + e)
  }
}

async function handleAddVariant({ nodeId }) {
  if (!currentConversationId.value) return
  try {
    const newIndex = await apiAddVariant(currentConversationId.value, nodeId, '', null)
    const msg = messages.value.find((m) => m.id === nodeId)
    if (msg) {
      msg.variants.push({
        id: `v-${Date.now()}`,
        content: '',
        display_content: '',
        status: 'draft',
        provenance: null,
      })
      msg.active_variant = newIndex
      broadcastPluginEvent(ST_EVENT_TYPES.MESSAGE_SWIPED, messageEventPayload(nodeId, {
        reason: 'add_variant',
        index: newIndex,
      }))
    }
  } catch (e) {
    console.error('分支失败:', e)
  }
}

// 切换变体（左右箭头 ‹ ›）：持久化到后端 + 更新本地 active_variant
async function handleSwitchVariant({ messageId, index }) {
  if (!currentConversationId.value) return
  const msg = messages.value.find((m) => m.id === messageId)
  if (!msg) return
  try {
    await apiSwitchVariant(currentConversationId.value, messageId, index)
    msg.active_variant = index
    broadcastPluginEvent(ST_EVENT_TYPES.MESSAGE_SWIPED, messageEventPayload(messageId, {
      reason: 'switch_variant',
      index,
    }))
  } catch (e) {
    console.error('切换变体失败:', e)
  }
}

// 处理流水线事件（更新 UI）
function handlePipelineEvent(event) {
  broadcastPluginPipelineEvent(event)

  switch (event.event_type) {
    case 'director_started':
      pipeline.stateLabel = '导演规划中'
      pipeline.director = { status: 'running', detail: '解析意图 · 检索世界书', output: '' }
      break
    case 'director_progress':
      // 累积导演流式输出
      if (pipeline.director.status !== 'running') {
        pipeline.director = { status: 'running', detail: '导演思考中', output: '' }
      }
      pipeline.director.output += event.data.delta || ''
      break
    case 'director_done':
      pipeline.director = {
        status: 'done',
        detail: `规划完成 · 分配 ${event.data.subagent_count} 个角色`,
        output: pipeline.director.output, // 保留已累积的输出
      }
      // 预填充 subagents 数组（避免稀疏数组导致 Vue 响应式失效，M-15）
      pipeline.subagents = Array.from({ length: event.data.subagent_count }, () => ({
        id: '', name: '', emoji: '🎭', status: 'pending', progress: 0
      }))
      pipeline.stateLabel = '子 Agent 并行表演'
      break
    case 'subagent_started':
      pipeline.subagents[event.data.index] = {
        id: event.data.character_id,
        name: instanceNameMap.value[event.data.character_id] || event.data.character_id,
        emoji: event.data.emoji || '🎭',
        status: 'running',
        progress: 0,
        output: '',
        _expanded: false,
      }
      break
    case 'subagent_progress':
      if (pipeline.subagents[event.data.index]) {
        pipeline.subagents[event.data.index].progress = Math.min(
          100,
          (pipeline.subagents[event.data.index].progress || 0) + 15
        )
      }
      break
    case 'subagent_done':
      if (pipeline.subagents[event.data.index]) {
        pipeline.subagents[event.data.index].status = 'done'
        pipeline.subagents[event.data.index].progress = 100
        pipeline.subagents[event.data.index].output = event.data.full_text || ''
      }
      break
    case 'subagent_cancelled':
      if (pipeline.subagents[event.data.index]) {
        pipeline.subagents[event.data.index].status = 'cancelled'
      }
      break
    case 'editor_started':
      pipeline.stateLabel = '编剧合并'
      pipeline.editor = { status: 'running', detail: '合并 · 润色 · 成文', output: '' }
      // Editor 逐字流式由 StreamingMessage 读 pipeline.editor.output 渲染，不再插占位消息
      break
    case 'editor_progress':
      // 累积编剧流式输出到 pipeline（StreamingMessage 实时渲染）
      if (pipeline.editor.status !== 'running') {
        pipeline.editor = { status: 'running', detail: '生成中', output: '' }
      }
      pipeline.editor.output += event.data.delta || ''
      scrollToBottom()
      break
    case 'draft_ready':
      pipeline.editor = { status: 'done', detail: '成文完成' }
      pipeline.stateLabel = '已产出'
      // 成文由 applyConversation 推入正式消息；StreamingMessage 随 showPipeline=false 消失
      break
    case 'postprocess_started':
      pipeline.postprocess = { status: 'running', detail: '提取知识 · 更新变量 · 检测任务', knowledge: 0, variable: 0, task: 0, reason: '' }
      break
    case 'postprocess_done':
      pipeline.postprocess = {
        status: 'done',
        detail: `知识 ${event.data.knowledge_count || 0} · 变量 ${event.data.variable_count || 0} · 任务 ${event.data.task_count || 0}`,
        knowledge: event.data.knowledge_count || 0,
        variable: event.data.variable_count || 0,
        task: event.data.task_count || 0,
        reason: '',
      }
      break
    case 'postprocess_failed':
      pipeline.postprocess = { status: 'error', detail: '后处理失败', knowledge: 0, variable: 0, task: 0, reason: event.data.reason || '' }
      break
    case 'postprocess_skipped':
      pipeline.postprocess = { status: 'done', detail: '已跳过', knowledge: 0, variable: 0, task: 0, reason: event.data.reason || '' }
      break
    case 'summary_done':
      // 摘要是后处理子步骤，记到 postprocess detail
      if (pipeline.postprocess.status === 'running') {
        pipeline.postprocess.detail = `摘要 ${event.data.char_count || 0} 字 · 提取中`
      }
      break
    case 'error':
      pipeline.stateLabel = `错误: ${event.data.message}`
      break
    case 'state_changed':
      // 状态变化已在其他事件中处理
      break
  }
}
</script>

<template>
  <div class="h-screen flex flex-col bg-bg overflow-hidden">
    <!-- 三栏主体（左右栏改弹出，中间占满） -->
    <div class="flex-1 flex min-h-0">

      <!-- ═══ 中写作区（占满） ═══ -->
      <main class="flex-1 flex flex-col min-w-0">
        <!-- 顶栏：左☰ + 标题 + 状态 + 右🛠（玻璃） -->
        <header class="glass shrink-0 h-14 flex items-center gap-2 px-3 border-b border-line">
          <button
            class="w-11 h-11 flex items-center justify-center rounded-lg text-ink-soft hover:bg-surface-2 transition-colors"
            @click="showSidebar = true"
            aria-label="菜单"
          >
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M3 6h18M3 12h18M3 18h18"/></svg>
          </button>

          <div class="flex-1 min-w-0 text-center">
            <div class="font-semibold text-ink truncate text-sm">{{ pageTitle }}</div>
            <div class="text-[11px] text-ink-soft truncate">
              <template v-if="isWriting">写作中…</template>
              <template v-else-if="writingMode === 'campaign'">Campaign · {{ activeCampaign?.story_clock || '第 1 轮' }}</template>
              <template v-else-if="writingMode === 'legacy'">兼容模式</template>
              <template v-else>导入角色卡或打开 Campaign</template>
            </div>
          </div>

          <!-- 写作中状态点 -->
          <div v-if="isWriting" class="flex items-center gap-1.5 px-2.5 py-1 rounded-full bg-accent-soft">
            <span class="w-1.5 h-1.5 rounded-full bg-accent animate-pulse"></span>
            <span class="text-[11px] text-accent font-medium">生成中</span>
          </div>

          <!-- 调试入口（🛠 总显示） -->
          <button
            class="w-11 h-11 flex items-center justify-center rounded-lg text-ink-soft hover:bg-surface-2 transition-colors"
            @click="showDebugDrawer = true"
            aria-label="调试"
          >🛠</button>
        </header>

        <!-- 消息流 / 视图（滚动区） -->
        <div class="flex-1 overflow-y-auto">
          <!-- 导入错误条 -->
          <div v-if="importError" class="mx-auto max-w-2xl px-4 pt-3">
            <div class="p-3 rounded-xl bg-err/10 border border-err/30 text-sm text-err">❌ 导入失败 <span class="text-ink-soft text-xs block mt-1">{{ importError }}</span></div>
          </div>

          <!-- 写作进行时：停止键在 Composer 发送槽位，过程流式在消息列表末尾 StreamingMessage -->

          <!-- ══ 视图：Campaign 概览 ══ -->
          <div v-if="currentView === 'overview'" class="mx-auto max-w-2xl px-4 py-6">
            <h2 class="text-xl font-bold text-ink mb-1">📜 {{ activeCampaign?.name }}</h2>
            <p v-if="activeCampaign?.story_clock" class="text-xs text-ink-soft mb-1">故事时间：{{ activeCampaign.story_clock }}</p>
            <p v-if="activeCampaign?.created_at" class="text-xs text-ink-soft mb-5">创建于 {{ new Date(activeCampaign.created_at).toLocaleDateString() }}</p>
            <div class="flex flex-wrap gap-2 mb-6">
              <button @click="showCampaignPanel = true" class="min-h-[44px] px-4 rounded-lg bg-accent text-white text-sm font-medium shadow-glow-accent hover:opacity-90 transition-opacity">进入 Campaign 面板</button>
              <button @click="openNewCampaignDialog" class="min-h-[44px] px-4 rounded-lg bg-surface-2 text-ink text-sm hover:bg-line transition-colors">✚ 新建 Campaign</button>
              <button @click="viewHistory" class="min-h-[44px] px-4 rounded-lg bg-surface-2 text-ink-soft text-sm hover:bg-line transition-colors">📋 会话历史 ({{ conversationHistory.length }})</button>
            </div>
          </div>

          <!-- ══ 视图：会话历史 ══ -->
          <div v-else-if="currentView === 'history'" class="mx-auto max-w-2xl px-4 py-6">
            <div class="flex items-center justify-between mb-4">
              <h2 class="text-xl font-bold text-ink">会话历史</h2>
              <button @click="openNewCampaignDialog" class="min-h-[44px] px-4 rounded-lg bg-accent text-white text-sm shadow-glow-accent hover:opacity-90 transition-opacity">✚ 新建 Campaign</button>
            </div>
            <div v-if="conversationHistory.length === 0" class="text-center text-ink-soft py-16 text-sm">暂无会话。点「新建 Campaign」开始第一局故事。</div>
            <div v-else class="space-y-2 sf-stagger">
              <div
                v-for="(conv, i) in conversationHistory" :key="conv.id"
                :style="{ '--i': i }"
                @click="openConversation(conv)"
                class="p-3.5 rounded-xl bg-surface shadow-card hover:shadow-rise hover:-translate-y-px cursor-pointer transition-all duration-200 flex items-center gap-2"
              >
                <div class="flex-1 min-w-0">
                  <div class="text-sm text-ink font-medium truncate">{{ conv.card_name || '未知角色卡' }}</div>
                  <div class="text-xs text-ink-soft mt-1">{{ conv.message_count || 0 }} 条消息 · 创建于 {{ new Date(conv.created_at).toLocaleString() }}</div>
                </div>
                <button
                  @click="handleDeleteConversation(conv, $event)"
                  class="shrink-0 w-9 h-9 flex items-center justify-center rounded-lg text-ink-faint hover:text-err hover:bg-err/10 transition-colors"
                  title="删除会话"
                  aria-label="删除会话"
                >
                  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
                </button>
              </div>
            </div>
          </div>

          <!-- ══ 视图：写作（消息列表，阅读器化） ══ -->
          <div v-else ref="messagesContainer" class="mx-auto max-w-2xl px-4 sm:px-6">
            <div v-if="canChooseGreeting" class="pt-3 pb-2">
              <div class="flex items-center gap-2 overflow-x-auto">
                <span class="shrink-0 text-xs text-ink-soft">开场</span>
                <button
                  v-for="(option, i) in greetingOptions"
                  :key="option.label"
                  @click="selectGreeting(i)"
                  class="shrink-0 min-h-[36px] px-3 rounded-lg text-xs font-medium border transition-colors"
                  :class="selectedGreetingIndex === i ? 'bg-accent text-white border-accent shadow-glow-accent' : 'bg-surface text-ink-soft border-line hover:border-accent hover:text-accent'"
                >
                  {{ option.label }}
                </button>
              </div>
            </div>
            <div v-if="messages.length === 0" class="text-center text-ink-faint py-20">
              <div class="text-4xl mb-3 opacity-40">✦</div>
              <div class="text-sm">描述你要写的场景，开始第一轮</div>
            </div>
            <ChatMessage
              v-for="m in messages"
              :key="m.id"
              :message="m"
              :conversation-id="currentConversationId"
              :busy="isWriting"
              :can-branch="writingMode === 'campaign' && !!activeCampaign && !!currentConversationId"
              @reroll="handleReroll"
              @reroll-user="handleRerollUser"
              @edit-variant="handleEditVariant"
              @accept-variant="handleAcceptVariant"
              @delete-variant="handleDeleteVariant"
              @branch="handleBranch"
              @add-variant="handleAddVariant"
              @switch-variant="handleSwitchVariant"
            />
            <!-- 写作进行时：过程流式（Director/子Agent折叠 + Editor逐字），全在最后一条消息 -->
            <StreamingMessage
              v-if="showPipeline"
              :pipeline="pipeline"
              :role-label="streamingRoleLabel"
            />
            <div class="h-4"></div>
          </div>
        </div>

        <!-- Composer（底部玻璃；写作中发送键变停止键） -->
        <Composer
          @start-writing="startWriting"
          @cancel="cancelWriting"
          :writing="isWriting"
          :disabled="writingMode === 'none'"
          :placeholder="writingMode === 'none' ? '请先导入角色卡或打开 Campaign…' : ''"
        />
      </main>

    </div>

    <!-- ═══ 左导航抽屉（顶栏☰触发，左侧滑出） ═══ -->
    <BaseOverlay :model-value="showSidebar" size="sm" position="left" :show-header="false" :body-scroll="false" @update:model-value="showSidebar = $event" @close="showSidebar = false">
      <AppSidebar
        mobile
        :power-mode="powerMode"
        :writing-mode="writingMode"
        :active-campaign-name="activeCampaign?.name"
        :active-char-name="activeChar?.name"
        :active-connection="activeConnection"
        :view="currentView"
        :conversation-count="conversationHistory.length"
        @open-campaign="showCampaignPanel = true"
        @open-char-list="showCharList = true"
        @open-preset="showPresetPanel = true"
        @open-plugin="showPluginPanel = true"
        @open-conn="showConnConfig = true"
        @open-meta="showMetaPanel = true"
        @import="handleImport"
        @new-campaign="openNewCampaignDialog"
        @view-history="viewHistory"
        @view-overview="viewOverview"
        @toggle-power="onTogglePower"
        @close="showSidebar = false"
      />
    </BaseOverlay>

    <!-- ═══ 右调试抽屉（顶栏🛠触发，右侧滑出） ═══ -->
    <BaseOverlay :model-value="showDebugDrawer" size="md" position="right" :show-header="false" :body-scroll="false" @update:model-value="showDebugDrawer = $event" @close="showDebugDrawer = false">
      <DebugDrawer
        mobile
        :sidebar-plugins="sidebarPlugins"
        :plugin-events="pluginPipelineEvents"
        @open-connection-config="showConnConfig = true"
        @close="showDebugDrawer = false"
      />
    </BaseOverlay>

    <!-- ═══ 弹层（不变） ═══ -->
    <CharacterList v-if="showCharList" :active-id="activeChar?.id" @select="handleSelectChar" @close="showCharList = false; showSidebar = true" />
    <ConnectionConfig v-if="showConnConfig" @close="showConnConfig = false; showSidebar = true" @changed="refreshActiveConnection" />
    <CampaignPanel v-if="showCampaignPanel" ref="campaignPanelRef" @close="showCampaignPanel = false; showSidebar = true" @campaign-changed="(c) => { activeCampaign = c; loadInstanceNameMap() }" />
    <MetaPanel v-if="showMetaPanel" :active-campaign="activeCampaign" :last-conversation-node="lastConversationNode" @close="showMetaPanel = false; showSidebar = true" @mvu-applied="handleMvuApplied" />
    <PresetPanel v-if="showPresetPanel" @close="showPresetPanel = false; showSidebar = true" />
    <PluginPanel v-if="showPluginPanel" @close="showPluginPanel = false; showSidebar = true; loadSidebarPlugins()" />

    <!-- ═══ 新建 Campaign 表单（选卡+起名，建完自动开对话） ═══ -->
    <BaseOverlay :model-value="showNewCampaignForm" title="新建 Campaign" size="sm" position="center" @update:model-value="showNewCampaignForm = $event" @close="showNewCampaignForm = false">
      <div class="p-4 space-y-4">
        <div v-if="newCampaignCards.length === 0" class="text-center text-ink-soft text-sm py-6">
          还没有已导入的角色卡<br>
          <span class="text-xs text-ink-faint">先点「导入」添加角色卡并识别角色</span>
        </div>
        <template v-else>
          <div>
            <label class="text-xs text-ink-soft mb-1.5 block">选择角色卡</label>
            <select v-model="newCampaignCardId" @change="loadNewCampaignCardDetail" class="w-full min-h-[44px] px-3 text-sm rounded-lg border border-line bg-surface focus:outline-none focus:border-accent">
              <option v-for="c in newCampaignCards" :key="c.id" :value="c.id">{{ c.name }}{{ c.extracted ? '' : '（未识别）' }}</option>
            </select>
          </div>
          <div v-if="newCampaignGreetingOptions.length > 1">
            <label class="text-xs text-ink-soft mb-1.5 block">开场白</label>
            <select v-model="newCampaignGreetingIndex" class="w-full min-h-[44px] px-3 text-sm rounded-lg border border-line bg-surface focus:outline-none focus:border-accent">
              <option v-for="(option, i) in newCampaignGreetingOptions" :key="i" :value="i">{{ option.label }}</option>
            </select>
          </div>
          <div>
            <label class="text-xs text-ink-soft mb-1.5 block">Campaign 名称</label>
            <input v-model="newCampaignName" placeholder="如：第一周目" @keyup.enter="handleCreateCampaign"
              class="w-full min-h-[44px] px-3 text-sm rounded-lg border border-line bg-surface focus:outline-none focus:border-accent" />
          </div>
          <div class="flex gap-2 pt-1">
            <button @click="showNewCampaignForm = false" class="flex-1 min-h-[44px] rounded-lg text-sm bg-surface-2 text-ink-soft hover:bg-line transition-colors">取消</button>
            <button @click="handleCreateCampaign" :disabled="!newCampaignCardId || !newCampaignName.trim() || creatingCampaign"
              class="flex-1 min-h-[44px] rounded-lg text-sm bg-accent text-white shadow-glow-accent hover:opacity-90 disabled:opacity-40 transition-opacity">
              {{ creatingCampaign ? '创建中…' : '创建并开始' }}
            </button>
          </div>
        </template>
      </div>
    </BaseOverlay>

    <!-- W8 MVU JS Runtime 容器（隐藏） -->
    <MvuJsRuntime />
  </div>
</template>
