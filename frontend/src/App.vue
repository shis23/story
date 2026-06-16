<script setup>
import { ref, reactive, onMounted, nextTick } from 'vue'
import AppHeader from './components/AppHeader.vue'
import ChatMessage from './components/ChatMessage.vue'
import Composer from './components/Composer.vue'
import PipelinePanel from './components/PipelinePanel.vue'
import AgentConfigCard from './components/AgentConfigCard.vue'
import CharacterDetail from './components/CharacterDetail.vue'
import CharacterList from './components/CharacterList.vue'
import LogPanel from './components/LogPanel.vue'
import ConnectionConfig from './components/ConnectionConfig.vue'
import CampaignPanel from './components/CampaignPanel.vue'
import PresetPanel from './components/PresetPanel.vue'
import PluginPanel from './components/PluginPanel.vue'
import PluginHost from './components/PluginHost.vue'
import MetaPanel from './components/MetaPanel.vue'
import { importCharacter, getCharacter, getVersion, startWriting as apiStartWriting, cancelWriting as apiCancelWriting, regenerate as apiRegenerate, getActiveConnection, editVariant as apiEditVariant, acceptVariant as apiAcceptVariant, softDeleteVariant as apiSoftDeleteVariant, addVariant as apiAddVariant, switchVariant as apiSwitchVariant, listConversations, getConversation, logAppendFrontend, getActiveCampaign, listPlugins, extractCharacters } from './tauri-api.js'

const powerMode = ref(false)
const messages = ref([])
const showPipeline = ref(false)
const appVersion = ref('...')

// 当前活跃的角色卡
const activeChar = ref(null)
const activeCharDetail = ref(null)
const importError = ref('')
const showCharDetail = ref(false)
const showCharList = ref(false)
const showCampaignPanel = ref(false)
const showMetaPanel = ref(false)
const activeCampaign = ref(null)
const showPresetPanel = ref(false)
const showPluginPanel = ref(false)
const sidebarPlugins = ref([])
const showSidebarPlugins = ref(false)

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

// 流水线状态
const pipeline = reactive({
  state: 'idle',
  stateLabel: '',
  director: { status: 'idle', detail: '', output: '' },
  subagents: [],
  editor: { status: 'idle', detail: '', output: '' },
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

// 是否有流水线运行中（用于禁用重 roll / 显示停止按钮）
const isWriting = ref(false)
// 当前对话 ID（重 roll 需要）
const currentConversationId = ref(null)
// 连接配置弹层
const showConnConfig = ref(false)
// 当前活跃连接（顶栏显示用）
const activeConnection = ref(null)
// AgentConfigCard 引用（连接变更后刷新）
const agentConfigRef = ref(null)

// 获取版本 + 加载活跃连接 + 恢复最近对话 + 拦截 console
onMounted(async () => {
  try { appVersion.value = await getVersion() } catch (e) { console.error('getVersion:', e) }
  await refreshActiveConnection()
  await loadRecentConversation()
  try { activeCampaign.value = await getActiveCampaign() } catch (e) { console.error('getActiveCampaign:', e) }
  await loadSidebarPlugins()
  setupConsoleForwarding()
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
          status: v.status === 'Final' ? 'final' : v.status === 'Discarded' ? 'discarded' : 'draft',
          provenance: v.provenance,
        })),
      }
    })
}

// 从后端恢复最近一次对话
async function loadRecentConversation() {
  try {
    const convList = await listConversations()
    if (!convList || convList.length === 0) return

    // 按 updated_at 降序，取最近的对话
    convList.sort((a, b) => b.updated_at.localeCompare(a.updated_at))
    const latest = convList[0]

    const conv = await getConversation(latest.id)
    if (!conv || !conv.nodes || conv.nodes.length === 0) return

    applyConversation(conv)

    // 如果有关联角色卡，加载角色信息
    if (conv.character_id) {
      try {
        const chars = await import('./tauri-api.js').then(m => m.listCharacters())
        const char = chars.find((c) => c.id === conv.character_id)
        if (char) {
          activeChar.value = char
          await loadCharDetail(char.id)
          // 用角色名更新 assistant 消息的 role_label
          messages.value.forEach((m) => {
            if (m.role === 'assistant') m.role_label = char.name
          })
        }
      } catch (e) { console.error('加载关联角色卡失败:', e) }
    }
  } catch (e) {
    console.error('恢复对话失败:', e)
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
    // 加载详情
    await loadCharDetail(result.id)

    // 用角色的开场白替换消息列表
    if (activeCharDetail.value?.first_mes) {
      messages.value = [{
        id: 'm1',
        role: 'assistant',
        role_label: activeCharDetail.value.name,
        active_variant: 0,
        variants: [{
          id: 'v1',
          content: activeCharDetail.value.first_mes,
          status: 'final',
          provenance: null,
        }],
      }]
    }
  } catch (err) {
    importError.value = String(err)
  }
}

// 加载角色详情（含世界书）
async function loadCharDetail(id) {
  try {
    activeCharDetail.value = await getCharacter(id)
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
    return
  }
  activeChar.value = char
  await loadCharDetail(char.id)

  // 用角色的开场白替换消息列表
  if (activeCharDetail.value?.first_mes) {
    messages.value = [{
      id: 'm1',
      role: 'assistant',
      role_label: activeCharDetail.value.name,
      active_variant: 0,
      variants: [{
        id: 'v1',
        content: activeCharDetail.value.first_mes,
        status: 'final',
        provenance: null,
      }],
    }]
  }
}

// 写作流水线（调用后端真实流水线，通过 Channel 接收事件）
async function startWriting(intent) {
  // 写作前检查：必须有活跃连接
  if (!activeConnection.value) {
    showConnConfig.value = true
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

  // 先把用户意图加入消息列表（聊天界面应该看到自己发了什么）
  const userMsgId = `user-${Date.now()}`
  messages.value.push({
    id: userMsgId,
    role: 'user',
    role_label: '我',
    active_variant: 0,
    variants: [{
      id: `uv-${Date.now()}`,
      content: intent,
      status: 'final',
      provenance: null,
    }],
  })
  scrollToBottom()

  try {
    const result = await apiStartWriting(intent, activeChar.value?.id, (event) => {
      // 实时更新流水线状态
      handlePipelineEvent(event)
    })

    // 后端返回 { text, conversation_id, node_id }
    const text = result.text
    const msgId = result.node_id || `msg-${Date.now()}`
    currentConversationId.value = result.conversation_id

    // 流水线完成，将成文追加到消息列表
    messages.value.push({
      id: msgId,
      role: 'assistant',
      role_label: activeChar.value?.name || 'AI',
      active_variant: 0,
      variants: [{
        id: `v-${Date.now()}`,
        content: text,
        status: 'final',
        provenance: null,
      }],
    })

    pipeline.state = 'done'
    pipeline.stateLabel = '已完成'
    scrollToBottom()
  } catch (err) {
    // 失败回滚：移除已 push 的用户消息（无对应 AI 回复，残留会误导重试）
    messages.value = messages.value.filter((m) => m.id !== userMsgId)
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
    alert('无对话上下文，无法重 roll')
    return
  }

  showPipeline.value = true
  isWriting.value = true
  pipeline.state = 'running'
  pipeline.stateLabel = `重 roll（${kind === 'all' ? '整体' : kind}）`
  pipeline.director = { status: 'idle', detail: '', output: '' }
  pipeline.subagents = []
  pipeline.editor = { status: 'idle', detail: '', output: '' }

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
      // 保留用户当前的角色卡 role_label 覆盖（applyConversation 重置为 AI/我）
      if (activeChar.value) {
        messages.value.forEach((m) => {
          if (m.role === 'assistant') m.role_label = activeChar.value.name
        })
      }
    }

    pipeline.state = 'done'
    pipeline.stateLabel = '重 roll 完成'
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
    // 同步更新本地消息
    const msg = messages.value.find((m) => m.id === nodeId)
    if (msg) {
      const variant = msg.variants[msg.active_variant]
      if (variant) variant.content = newContent
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
    }
  } catch (e) {
    console.error('采纳失败:', e)
  }
}

// 软删除变体（→ Discarded），删除后重新拉取对话刷新（单一事实源：
// 后端 soft_delete 会自动切到最近非 Discarded variant，前端需同步）
async function handleDeleteVariant({ nodeId }) {
  if (!currentConversationId.value) return
  try {
    await apiSoftDeleteVariant(currentConversationId.value, nodeId)
    const refreshed = await getConversation(currentConversationId.value)
    if (refreshed) {
      applyConversation(refreshed)
      if (activeChar.value) {
        messages.value.forEach((m) => {
          if (m.role === 'assistant') m.role_label = activeChar.value.name
        })
      }
    }
  } catch (e) {
    console.error('删除失败:', e)
  }
}

// 添加新变体（分支）
async function handleAddVariant({ nodeId }) {
  if (!currentConversationId.value) return
  try {
    const newIndex = await apiAddVariant(currentConversationId.value, nodeId, '', null)
    const msg = messages.value.find((m) => m.id === nodeId)
    if (msg) {
      msg.variants.push({
        id: `v-${Date.now()}`,
        content: '',
        status: 'draft',
        provenance: null,
      })
      msg.active_variant = newIndex
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
  } catch (e) {
    console.error('切换变体失败:', e)
  }
}

// 处理流水线事件（更新 UI）
function handlePipelineEvent(event) {
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
      pipeline.stateLabel = '子 Agent 并行表演'
      break
    case 'subagent_started':
      pipeline.subagents[event.data.index] = {
        id: event.data.character_id,
        name: event.data.character_id,
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
      break
    case 'editor_progress':
      // 累积编剧流式输出
      if (pipeline.editor.status !== 'running') {
        pipeline.editor = { status: 'running', detail: '生成中', output: '' }
      }
      pipeline.editor.output += event.data.delta || ''
      break
    case 'draft_ready':
      pipeline.editor = { status: 'done', detail: '成文完成' }
      pipeline.stateLabel = '已产出'
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
  <div class="max-w-2xl mx-auto h-screen bg-bg flex flex-col" style="max-width: 480px;">
    <!-- 顶栏 -->
    <AppHeader
      :power-mode="powerMode"
      :active-char-name="activeChar?.name"
      @toggle-power="powerMode = !powerMode"
      @open-campaign="showCampaignPanel = true"
      @open-meta="showMetaPanel = true"
    >
      <template #actions>
        <button
          @click="showConnConfig = true"
          class="px-2.5 py-1.5 rounded-full text-xs font-medium transition-all shrink-0 flex items-center gap-1"
          :class="activeConnection
            ? 'bg-green-500/10 text-green-700 hover:bg-green-500/20'
            : 'bg-warn/10 text-warn hover:bg-warn/20'"
          :title="activeConnection ? `活跃连接：${activeConnection.name}` : '未配置连接'"
        >
          {{ activeConnection ? '⚡' : '⚠️' }}
          <span v-if="activeConnection" class="hidden xs:inline">{{ activeConnection.model }}</span>
          <span v-else class="hidden xs:inline">配置</span>
        </button>
        <button
          @click="showCharList = true"
          class="px-2.5 py-1.5 rounded-full text-xs font-medium bg-bg text-ink-soft hover:bg-line transition-all shrink-0"
          title="角色卡列表"
        >
          📋
        </button>
        <button
          @click="showPresetPanel = true"
          class="px-2.5 py-1.5 rounded-full text-xs font-medium bg-bg text-ink-soft hover:bg-line transition-all shrink-0"
          title="预设管理"
        >
          📑
        </button>
        <button
          @click="showPluginPanel = true"
          class="px-2.5 py-1.5 rounded-full text-xs font-medium bg-bg text-ink-soft hover:bg-line transition-all shrink-0"
          title="插件管理"
        >
          🔌
        </button>
        <button
          @click="handleImport"
          class="px-3 py-1.5 rounded-full text-xs font-medium bg-accent-soft text-accent hover:bg-accent hover:text-white transition-all shrink-0"
        >
          📥 导入
        </button>
      </template>
    </AppHeader>

    <!-- 主滚动区 -->
    <main class="flex-1 overflow-y-auto">
      <!-- 当前角色卡（可点击查看详情） -->
      <div
        v-if="activeChar"
        @click="showCharDetail = true"
        class="mx-4 mt-3 p-3 bg-accent-soft/50 rounded-xl border border-accent-border cursor-pointer hover:bg-accent-soft transition-colors"
      >
        <div class="flex items-center gap-2">
          <div class="w-8 h-8 rounded-full bg-accent text-white flex items-center justify-center text-sm shrink-0">
            {{ activeChar.name?.charAt(0) || '?' }}
          </div>
          <div class="flex-1 min-w-0">
            <div class="text-sm font-medium text-accent truncate">{{ activeChar.name }}</div>
            <div class="text-[11px] text-ink-soft">
              ST {{ activeChar.spec_version }} · {{ activeChar.world_info_count }} 条世界书
              <span v-if="activeChar.has_renderable_assets"> · 🎨</span>
            </div>
          </div>
          <span class="text-xs text-ink-soft/60">详情 →</span>
        </div>
      </div>

      <!-- 未选择角色时的提示 -->
      <div v-else class="mx-4 mt-3 p-4 bg-bg rounded-xl border border-dashed border-line text-center">
        <div class="text-ink-soft text-sm">还没有选择角色卡</div>
        <div class="text-xs text-ink-soft/60 mt-1">点 📥 导入或 📋 选择</div>
      </div>

      <!-- 导入错误 -->
      <div v-if="importError" class="mx-4 mt-3 p-3 bg-err/10 rounded-xl border border-err/30">
        <div class="text-sm text-err">❌ 导入失败</div>
        <div class="text-xs text-ink-soft mt-1">{{ importError }}</div>
      </div>

      <!-- 流水线面板 -->
      <PipelinePanel v-if="showPipeline" :pipeline="pipeline">
        <template #extra>
          <button
            v-if="isWriting"
            @click="cancelWriting"
            class="mt-2 w-full px-3 py-1.5 text-xs rounded-lg bg-err/10 text-err border border-err/30 hover:bg-err/20"
          >
            ⏹ 停止生成
          </button>
        </template>
      </PipelinePanel>

      <!-- 高玩模式：Agent 配置卡片 -->
      <AgentConfigCard
        v-if="powerMode"
        ref="agentConfigRef"
        @open-connection-config="showConnConfig = true"
      />

      <!-- 高玩模式：日志面板 -->
      <div v-if="powerMode" class="mx-4 mt-3">
        <LogPanel />
      </div>

      <!-- 对话消息列表 -->
      <div ref="messagesContainer" class="flex-1 divide-y divide-line/50 overflow-y-auto">
        <ChatMessage
          v-for="m in messages"
          :key="m.id"
          :message="m"
          :conversation-id="currentConversationId"
          :busy="isWriting"
  @reroll="handleReroll"
  @edit-variant="handleEditVariant"
  @accept-variant="handleAcceptVariant"
  @delete-variant="handleDeleteVariant"
  @add-variant="handleAddVariant"
  @switch-variant="handleSwitchVariant"
/>
      </div>

      <div class="h-4"></div>
    </main>

    <!-- 插件侧栏面板 -->
    <div v-if="sidebarPlugins.length > 0" class="border-t border-line/50">
      <button
        @click="showSidebarPlugins = !showSidebarPlugins"
        class="w-full px-4 py-1.5 text-xs text-ink-soft hover:bg-line/30 flex items-center gap-1"
      >
        <span class="transition-transform" :class="showSidebarPlugins ? 'rotate-90' : ''">▸</span>
        🧩 插件面板 ({{ sidebarPlugins.length }})
      </button>
      <div v-if="showSidebarPlugins" class="px-4 pb-2 space-y-2">
        <PluginHost
          v-for="p in sidebarPlugins"
          :key="p.id"
          :plugin="p"
          height="150px"
        />
      </div>
    </div>

    <!-- 底部输入栏 -->
    <Composer @start-writing="startWriting" />

    <!-- 版本号 -->
    <div class="text-center text-[10px] text-ink-soft/40 pb-1">v{{ appVersion }}</div>

    <!-- 角色详情弹层 -->
    <CharacterDetail
      v-if="showCharDetail && activeCharDetail"
      :character="activeCharDetail"
      @close="showCharDetail = false"
    />

    <!-- 角色列表弹层 -->
    <CharacterList
      v-if="showCharList"
      :active-id="activeChar?.id"
      @select="handleSelectChar"
      @close="showCharList = false"
    />

    <!-- LLM 连接配置弹层 -->
    <ConnectionConfig
      v-if="showConnConfig"
      @close="showConnConfig = false"
      @changed="refreshActiveConnection"
    />

    <!-- Campaign 管理弹层 -->
    <CampaignPanel
      v-if="showCampaignPanel"
      @close="showCampaignPanel = false"
      @campaign-changed="(c) => activeCampaign = c"
    />

    <!-- Meta 配置助手弹层（P3 新增） -->
    <MetaPanel
      v-if="showMetaPanel"
      @close="showMetaPanel = false"
    />

    <!-- 预设管理弹层 -->
    <PresetPanel
      v-if="showPresetPanel"
      @close="showPresetPanel = false"
    />

    <!-- 插件管理弹层 -->
    <PluginPanel
      v-if="showPluginPanel"
      @close="showPluginPanel = false; loadSidebarPlugins()"
    />
  </div>
</template>
