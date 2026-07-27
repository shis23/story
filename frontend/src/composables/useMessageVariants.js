// 迁移自 App.vue:822-1097 的变体操作群。
// 消费 writingStore: messages / isWriting / showPipeline / pipeline。
// 消费 campaignStore: activeCampaign / currentConversationId。
// 消费 uiStore: showHistory / activeCampaignOverview（handleBranch 切视图用）。
//
// 范围外依赖（注入）：
// - handlePipelineEvent(event)：来自 usePipeline composable（本次未迁移）。
// - applyConversation(conv)：App.vue:416-438，把后端 Conversation 应用到 messages。
// - broadcastPluginEvent(event, data)：插件事件广播。
// - messageEventPayload(messageId, extra)：App.vue:202-213，消息级事件载荷。
// - chatEventPayload(extra)：App.vue:191-200，会话级事件载荷。
// - loadInstanceNameMap()：刷新实例名映射（App.vue:42-54）。
// - loadConversationHistory()：刷新会话历史列表（App.vue:441-453）。
// - scrollToBottom()：消息容器自动滚动。
// - alertDialog(message)：错误弹窗。
// - startWriting(intent, skipLocalPush)：来自 useWriting（handleRerollUser 无 AI 消息时回退）。
// 未传时安全降级（startWriting 降级为 no-op 并告警；alertDialog 降级为 console.error）。

import { useWritingStore } from '../stores/writing.js'
import { useCampaignStore } from '../stores/campaign.js'
import { useUiStore } from '../stores/ui.js'
import {
  regenerate as apiRegenerate,
  editVariant as apiEditVariant,
  acceptVariant as apiAcceptVariant,
  getActiveTurnReceipt as apiGetActiveTurnReceipt,
  retryActiveTurnPostprocess as apiRetryActiveTurnPostprocess,
  deleteMessageFrom as apiDeleteMessageFrom,
  addVariant as apiAddVariant,
  switchVariant as apiSwitchVariant,
  forkCampaign,
  setActiveCampaign,
  getActiveCampaign,
  getConversation,
} from '../tauri-api.js'
import { ST_EVENT_TYPES } from '../plugin-bridge.js'
import { makeForkCampaignName } from '../utils/forkCampaignName.js'
import { assistantRoleLabel } from '../utils/roleLabel.js'

/**
 * @param {{
 *   handlePipelineEvent?: (event: object) => void,
 *   applyConversation?: (conv: object) => void,
 *   broadcastPluginEvent?: (event: string, data?: object) => void,
 *   messageEventPayload?: (messageId: string, extra?: object) => object,
 *   chatEventPayload?: (extra?: object) => object,
 *   loadInstanceNameMap?: () => Promise<void> | void,
 *   loadConversationHistory?: () => Promise<void> | void,
 *   scrollToBottom?: () => void,
 *   alertDialog?: (message: string) => Promise<void> | void,
 *   askForceAccept?: (message: string, options?: object) => Promise<boolean> | boolean,
 *   startWriting?: (intent: string, skipLocalPush?: boolean) => Promise<void>,
 *   beginPromptHookGeneration?: () => number,
 *   regenerateApi?: (request: object, onEvent?: (event: object) => void) => Promise<unknown>,
 *   acceptVariantApi?: (conversationId: string, nodeId: string, forceAccept?: boolean, selectedMutationIndices?: number[] | null) => Promise<unknown>,
 *   getActiveTurnReceiptApi?: (campaignId: string, nodeId: string) => Promise<object | null>,
 *   retryActiveTurnPostprocessApi?: (campaignId: string, nodeId: string) => Promise<object | null>,
 *   deleteMessageFromApi?: (conversationId: string, nodeId: string) => Promise<unknown>,
 *   getConversationApi?: (conversationId: string) => Promise<object | null>,
 * }} [options]
 */
export function useMessageVariants(options = {}) {
  const writingStore = useWritingStore()
  const campaignStore = useCampaignStore()
  const uiStore = useUiStore()

  const handlePipelineEvent = options.handlePipelineEvent || (() => {})
  const applyConversation = options.applyConversation || (() => {})
  const broadcastPluginEvent = options.broadcastPluginEvent || (() => {})
  const messageEventPayload = options.messageEventPayload || (() => ({}))
  const chatEventPayload = options.chatEventPayload || (() => ({}))
  const loadInstanceNameMap = options.loadInstanceNameMap || (() => {})
  const loadConversationHistory = options.loadConversationHistory || (() => {})
  const scrollToBottom = options.scrollToBottom || (() => {})
  const alertDialog = options.alertDialog || ((msg) => { console.error('alertDialog(未注入):', msg) })
  const startWriting = options.startWriting || (() => { console.error('useMessageVariants: startWriting 未注入') })
  const beginPromptHookGeneration = options.beginPromptHookGeneration || (() => 0)
  const regenerateApi = options.regenerateApi || apiRegenerate
  const acceptVariantApi = options.acceptVariantApi || apiAcceptVariant
  const getActiveTurnReceiptApi = options.getActiveTurnReceiptApi || apiGetActiveTurnReceipt
  const retryActiveTurnPostprocessApi =
    options.retryActiveTurnPostprocessApi || apiRetryActiveTurnPostprocess
  const deleteMessageFromApi = options.deleteMessageFromApi || apiDeleteMessageFrom
  const getConversationApi = options.getConversationApi || getConversation
  const askForceAccept = options.askForceAccept || (async (message, dialogOptions) => {
    const { ask } = await import('@tauri-apps/plugin-dialog')
    return ask(message, dialogOptions)
  })
  // Backend Accept is deliberately idempotent, but terminal plugin fan-out is
  // an external side effect. Coalesce in-flight requests and remember a
  // successfully published variant so double-clicks/replayed commands cannot
  // emit MESSAGE_RECEIVED twice.
  const pendingAccepts = new Map()
  const publishedTerminalAccepts = new Set()
  const MAX_PUBLISHED_TERMINAL_ACCEPTS = 512

  function acceptVariantIdentity(nodeId) {
    const message = writingStore.messages.find((item) => item?.id === nodeId)
    const variant = message?.variants?.[message.active_variant] || message?.variants?.[0] || null
    return {
      variant,
      key: `${campaignStore.currentConversationId || ''}\u001f${nodeId || ''}\u001f${variant?.id || 'active-variant'}`,
    }
  }

  function rememberPublishedTerminalAccept(key) {
    publishedTerminalAccepts.add(key)
    while (publishedTerminalAccepts.size > MAX_PUBLISHED_TERMINAL_ACCEPTS) {
      const oldest = publishedTerminalAccepts.values().next().value
      if (oldest === undefined) break
      publishedTerminalAccepts.delete(oldest)
    }
  }

  // 来源 App.vue:314-317 getAssistantRoleLabel
  function getAssistantRoleLabel() {
    return assistantRoleLabel(
      writingStore.writingMode,
      campaignStore.activeCampaign?.name,
      campaignStore.activeChar?.name,
    )
  }

  // 来源 App.vue:822-883 handleReroll
  async function handleReroll({ messageId, kind, hint }) {
    // 找到消息
    const msg = writingStore.messages.find((m) => m.id === messageId)
    if (!msg) return
    if (!campaignStore.currentConversationId) {
      await alertDialog('无对话上下文，无法重 roll')
      return
    }

    writingStore.showPipeline = true
    writingStore.isWriting = true
    writingStore.pipeline.state = 'running'
    writingStore.pipeline.stateLabel = `重 roll（${kind === 'all' ? '整体' : kind}）`
    writingStore.pipeline.director = { status: 'idle', detail: '', output: '' }
    writingStore.pipeline.subagents = []
    writingStore.pipeline.editor = { status: 'idle', detail: '', output: '', role: null }
    writingStore.pipeline.summary = { status: 'idle', detail: '', charCount: 0 }
    writingStore.pipeline.postprocess = { status: 'idle', detail: '', knowledge: 0, variable: 0, task: 0, reason: '' }
    writingStore.messages = writingStore.messages.filter((m) => m.id !== 'editor-streaming')

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
      beginPromptHookGeneration()
      const result = await regenerateApi({
        conversationId: campaignStore.currentConversationId,
        nodeId: messageId,
        targets,
        generationMode: campaignStore.activeCampaign ? writingStore.generationMode : null,
        hint,
      }, (event) => handlePipelineEvent(event))

      // 重拉对话刷新 UI（单一事实源）：后端按「最后一条 → 原地替换（旧 variant 降级
      // Discarded 可切回）/ 中间 → 开分支（保留旧版）」落库。前端不臆测 variant 数组，
      // 直接以后端真实状态为准。
      const refreshed = await getConversationApi(campaignStore.currentConversationId)
      if (refreshed) {
        applyConversation(refreshed)
        // 保留 role_label 覆盖（applyConversation 重置为 AI/我）
        writingStore.messages.forEach((m) => {
          if (m.role === 'assistant') m.role_label = getAssistantRoleLabel()
        })
        broadcastPluginEvent(ST_EVENT_TYPES.MESSAGE_UPDATED, messageEventPayload(messageId, { reason: 'reroll', kind }))
      }

      writingStore.pipeline.state = 'done'
      writingStore.pipeline.stateLabel = '重 roll 完成'
      writingStore.showPipeline = false // 收起 StreamingMessage
      loadInstanceNameMap()
      // result 含最终成文，但 UI 已由 refreshed 驱动，无需单独消费
      void result
    } catch (err) {
      writingStore.pipeline.state = 'error'
      writingStore.pipeline.stateLabel = `重 roll 失败: ${err}`
    } finally {
      writingStore.isWriting = false
    }
  }

  // 来源 App.vue:888-903 handleEditVariant
  async function handleEditVariant({ nodeId, newContent }) {
    if (!campaignStore.currentConversationId) return
    try {
      await apiEditVariant(campaignStore.currentConversationId, nodeId, newContent)
      const refreshed = await getConversationApi(campaignStore.currentConversationId)
      if (refreshed) {
        applyConversation(refreshed)
        writingStore.messages.forEach((m) => {
          if (m.role === 'assistant') m.role_label = getAssistantRoleLabel()
        })
        broadcastPluginEvent(ST_EVENT_TYPES.MESSAGE_UPDATED, messageEventPayload(nodeId, { reason: 'edit' }))
      }
    } catch (e) {
      console.error('编辑失败:', e)
    }
  }

  // 来源 App.vue:906-919 handleAcceptVariant
  // Quality Error 默认拦截；用户确认后 forceAccept → Degraded
  async function handleAcceptVariant({
    nodeId,
    forceAccept = false,
    selectedMutationIndices = null,
    skipReceipt = false,
  } = {}) {
    if (!campaignStore.currentConversationId) return
    const { variant, key } = acceptVariantIdentity(nodeId)
    if (variant?.status === 'final' || publishedTerminalAccepts.has(key)) return
    if (
      campaignStore.activeCampaign?.id &&
      !skipReceipt &&
      selectedMutationIndices === null
    ) {
      try {
        const receipt = await getActiveTurnReceiptApi(campaignStore.activeCampaign.id, nodeId)
        if (receipt) {
          writingStore.openTurnReceipt(nodeId, receipt)
          return
        }
      } catch (error) {
        console.error('读取采纳小票失败:', error)
        return
      }
    }
    // Keep the normal quality-gate attempt distinct from its later force retry.
    // Concurrent attempts of the same flavor share one promise; a direct force
    // click and a dialog-confirmed force retry also converge on the force key.
    const pendingKey = `${key}\u001f${forceAccept ? 'force' : 'normal'}`
    const existing = pendingAccepts.get(pendingKey)
    if (existing) return existing

    const task = (async () => {
      try {
        await acceptVariantApi(
          campaignStore.currentConversationId,
          nodeId,
          forceAccept,
          selectedMutationIndices,
        )
        const msg = writingStore.messages.find((m) => m.id === nodeId)
        if (msg) {
          const acceptedVariant = msg.variants[msg.active_variant]
          if (acceptedVariant) acceptedVariant.status = 'final'
        }
        // A successful Accept is the only frontend source of terminal message
        // fan-out. Mark before broadcasting so any synchronous/re-entrant
        // caller observes the terminal state too.
        rememberPublishedTerminalAccept(key)
        writingStore.clearTurnReceipt()
        broadcastPluginEvent(ST_EVENT_TYPES.MESSAGE_RECEIVED, messageEventPayload(nodeId, {
          reason: 'accept_variant',
          terminalTurnCommit: true,
          turnStatus: forceAccept ? 'Degraded' : 'Committed',
          attemptStatus: 'Final',
          variantStatus: 'Final',
          forceAccept: Boolean(forceAccept),
        }))
      } catch (e) {
        const msg = String(e?.message || e || '')
        if (!forceAccept && /质量门禁|force_accept|Error 级/i.test(msg)) {
          try {
            const ok = await askForceAccept(
              '质量门禁发现 Error 级问题。强制采纳将标记本轮为 Degraded，是否继续？',
              { title: '强制采纳确认', kind: 'warning' },
            )
            if (ok) {
              return handleAcceptVariant({
                nodeId,
                forceAccept: true,
                selectedMutationIndices: null,
                skipReceipt: true,
              })
            }
          } catch (dialogErr) {
            console.error('强制采纳确认失败:', dialogErr)
          }
        }
        console.error('采纳失败:', e)
      }
    })()
    pendingAccepts.set(pendingKey, task)
    try {
      return await task
    } finally {
      pendingAccepts.delete(pendingKey)
    }
  }

  async function handleRetryPostprocess({ nodeId } = {}) {
    const campaignId = campaignStore.activeCampaign?.id
    if (!campaignId || !nodeId) return
    if (writingStore.pendingReceipt?.nodeId === nodeId) {
      writingStore.pendingReceipt.retrying = true
      writingStore.pendingReceipt.retry_error = null
    }
    try {
      const receipt = await retryActiveTurnPostprocessApi(campaignId, nodeId)
      if (receipt) writingStore.openTurnReceipt(nodeId, receipt)
    } catch (error) {
      console.error('重试记账失败:', error)
      if (writingStore.pendingReceipt?.nodeId === nodeId) {
        writingStore.pendingReceipt.retrying = false
        writingStore.pendingReceipt.retry_error = String(error?.message || error || '')
      }
    }
  }

  // 来源 App.vue:922-947 handleDeleteVariant
  async function handleDeleteVariant({ nodeId }) {
    if (!campaignStore.currentConversationId) return
    try {
      await deleteMessageFromApi(campaignStore.currentConversationId, nodeId)
      // 重新拉取对话刷新（truncate 后该消息及之后都消失）
      const refreshed = await getConversationApi(campaignStore.currentConversationId)
      if (refreshed) {
        applyConversation(refreshed)
        writingStore.messages.forEach((m) => {
          if (m.role === 'assistant') m.role_label = getAssistantRoleLabel()
        })
        broadcastPluginEvent(ST_EVENT_TYPES.MESSAGE_DELETED, chatEventPayload({ messageId: nodeId }))
      }
      // 清流水线状态（删除 = 回到这条之前的状态，上次写作的导演/子Agent/编剧输出作废）
      writingStore.pipeline.state = 'idle'
      writingStore.pipeline.stateLabel = ''
      writingStore.pipeline.director = { status: 'idle', detail: '', output: '' }
      writingStore.pipeline.subagents = []
      writingStore.pipeline.editor = { status: 'idle', detail: '', output: '', role: null }
      writingStore.pipeline.summary = { status: 'idle', detail: '', charCount: 0 }
      writingStore.pipeline.postprocess = { status: 'idle', detail: '', knowledge: 0, variable: 0, task: 0, reason: '' }
      writingStore.showPipeline = false
    } catch (e) {
      console.error('删除失败:', e)
      await alertDialog('删除失败: ' + e)
    }
  }

  // 来源 App.vue:951-1005 handleRerollUser
  async function handleRerollUser({ messageId }) {
    if (writingStore.isWriting) return
    const userMsg = writingStore.messages.find((m) => m.id === messageId)
    if (!userMsg) return
    const intent = userMsg.variants[userMsg.active_variant]?.content
    if (!intent) return
    // 找到紧随其后的 AI 消息（regenerate 的目标）
    const userIndex = writingStore.messages.findIndex((m) => m.id === messageId)
    const aiMsg = writingStore.messages.slice(userIndex + 1).find((m) => m.role === 'assistant')
    // 没有 AI 消息（已被删除）→ 直接用 intent 重新写作（跳过本地 push，u3 已在列表中）
    if (!aiMsg) {
      await startWriting(intent, true)
      return
    }

    writingStore.showPipeline = true
    writingStore.isWriting = true
    writingStore.pipeline.state = 'running'
    writingStore.pipeline.stateLabel = '重 roll（整体）'
    writingStore.pipeline.director = { status: 'idle', detail: '', output: '' }
    writingStore.pipeline.subagents = []
    writingStore.pipeline.editor = { status: 'idle', detail: '', output: '', role: null }
    writingStore.pipeline.summary = { status: 'idle', detail: '', charCount: 0 }
    writingStore.pipeline.postprocess = { status: 'idle', detail: '', knowledge: 0, variable: 0, task: 0, reason: '' }
    writingStore.messages = writingStore.messages.filter((m) => m.id !== 'editor-streaming')

    try {
      beginPromptHookGeneration()
      await regenerateApi({
        conversationId: campaignStore.currentConversationId,
        nodeId: aiMsg.id,
        targets: [],       // 空 = 整体重 roll
        generationMode: campaignStore.activeCampaign ? writingStore.generationMode : null,
        hint: intent,       // user 的意图作为 hint 注入导演+编剧
      }, (event) => handlePipelineEvent(event))

      // 重拉对话刷新（regenerate 新增 variant：最后一条→旧降级Discarded+新active，中间→开分支）
      const refreshed = await getConversationApi(campaignStore.currentConversationId)
      if (refreshed) {
        applyConversation(refreshed)
        writingStore.messages.forEach((m) => {
          if (m.role === 'assistant') m.role_label = getAssistantRoleLabel()
        })
        broadcastPluginEvent(ST_EVENT_TYPES.MESSAGE_UPDATED, messageEventPayload(aiMsg.id, { reason: 'reroll_user' }))
      }
      writingStore.messages = writingStore.messages.filter((m) => m.id !== 'editor-streaming')
      writingStore.pipeline.state = 'done'
      writingStore.pipeline.stateLabel = '重 roll 完成'
      writingStore.showPipeline = false // 收起 StreamingMessage
      loadInstanceNameMap()
      scrollToBottom()
    } catch (err) {
      writingStore.pipeline.state = 'error'
      writingStore.pipeline.stateLabel = `重 roll 失败: ${err}`
    } finally {
      writingStore.isWriting = false
    }
  }

  // 来源 App.vue:1019-1056 handleBranch
  async function handleBranch({ nodeId }) {
    if (writingStore.isWriting) return
    if (!campaignStore.activeCampaign || !campaignStore.currentConversationId) {
      await alertDialog('请先打开 Campaign 对话，再创建分支。')
      return
    }

    try {
      const result = await forkCampaign(
        campaignStore.activeCampaign.id,
        nodeId,
        makeForkCampaignName(campaignStore.activeCampaign?.name),
      )
      await setActiveCampaign(result.id)
      campaignStore.activeCampaign = await getActiveCampaign()
      await loadInstanceNameMap()

      if (result.conversation_id) {
        const conv = await getConversationApi(result.conversation_id)
        if (conv) {
          applyConversation(conv)
          writingStore.messages.forEach((m) => {
            if (m.role === 'assistant') m.role_label = getAssistantRoleLabel()
          })
        }
      }

      uiStore.showHistory = false
      uiStore.activeCampaignOverview = false
      await loadConversationHistory()
      broadcastPluginEvent(ST_EVENT_TYPES.CHAT_LOADED, chatEventPayload({
        reason: 'campaign_forked',
        campaignId: result.id,
        conversationId: result.conversation_id || campaignStore.currentConversationId,
        forkNodeId: nodeId,
        sourceCampaignId: result.fork_from?.[0] || null,
      }))
    } catch (e) {
      console.error('创建分支失败:', e)
      await alertDialog('创建分支失败: ' + e)
    }
  }

  // 来源 App.vue:1058-1080 handleAddVariant
  async function handleAddVariant({ nodeId }) {
    if (!campaignStore.currentConversationId) return
    try {
      const newIndex = await apiAddVariant(campaignStore.currentConversationId, nodeId, '', null)
      const msg = writingStore.messages.find((m) => m.id === nodeId)
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

  // 来源 App.vue:1083-1097 handleSwitchVariant
  async function handleSwitchVariant({ messageId, index }) {
    if (!campaignStore.currentConversationId) return
    const msg = writingStore.messages.find((m) => m.id === messageId)
    if (!msg) return
    try {
      await apiSwitchVariant(campaignStore.currentConversationId, messageId, index)
      msg.active_variant = index
      broadcastPluginEvent(ST_EVENT_TYPES.MESSAGE_SWIPED, messageEventPayload(messageId, {
        reason: 'switch_variant',
        index,
      }))
    } catch (e) {
      console.error('切换变体失败:', e)
    }
  }

  return {
    handleReroll,
    handleEditVariant,
    handleAcceptVariant,
    handleRetryPostprocess,
    handleDeleteVariant,
    handleRerollUser,
    handleBranch,
    handleAddVariant,
    handleSwitchVariant,
  }
}
