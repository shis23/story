// 迁移自 App.vue:696-809 startWriting、812-819 cancelWriting。
// 消费 writingStore: messages / isWriting / showPipeline / pipeline / activeConnection /
//                   writingMode / selectedGreeting。
// 消费 campaignStore: activeChar / activeCampaign / currentConversationId。
// 消费 uiStore: showConnConfig（无活跃连接时弹连接配置）。
//
// 范围外依赖（注入）：
// - handlePipelineEvent(event)：来自 usePipeline composable（本次未迁移）。
//   为避免循环依赖，作为 options 注入；startWriting 亦可接收单次覆盖参数。
// - runPromptHookEvents(intent)：App.vue:239-256，prompt hook 预处理。
// - applyConversation(conv)：App.vue:416-438，把后端 Conversation 应用到 messages。
// - broadcastPluginEvent(event, data)：插件事件广播。
// - messageEventPayload(messageId, extra)：App.vue:202-213，构造消息级事件载荷。
// - scrollToBottom()：消息容器自动滚动。
// - loadInstanceNameMap()：刷新实例名映射（App.vue:42-54）。
// - alertDialog(message)：错误弹窗。
// 未传时安全降级（runPromptHookEvents 默认原样返回 intent；其余 no-op）。

import { useWritingStore } from '../stores/writing.js'
import { useCampaignStore } from '../stores/campaign.js'
import { useUiStore } from '../stores/ui.js'
import {
  startWriting as apiStartWriting,
  cancelWriting as apiCancelWriting,
  getConversation,
} from '../tauri-api.js'
import { ST_EVENT_TYPES } from '../plugin-bridge.js'
import {
  createPromptHookCancelledError,
  isPromptHookCancelledError,
} from '../utils/promptHooks.js'
import { assistantRoleLabel } from '../utils/roleLabel.js'

/**
 * @param {{
 *   handlePipelineEvent?: (event: object) => void,
 *   runPromptHookEvents?: (intent: string) => Promise<string>,
 *   cancelPromptHooks?: () => void,
 *   isPromptHooksCancelled?: () => boolean,
 *   startWritingApi?: (...args: any[]) => Promise<object>,
 *   getConversationApi?: (conversationId: string) => Promise<object | null>,
 *   cancelWritingApi?: () => Promise<unknown>,
 *   applyConversation?: (conv: object) => void,
 *   broadcastPluginEvent?: (event: string, data?: object) => void,
 *   messageEventPayload?: (messageId: string, extra?: object) => object,
 *   scrollToBottom?: () => void,
 *   loadInstanceNameMap?: () => Promise<void> | void,
 *   alertDialog?: (message: string) => Promise<void> | void,
 * }} [options]
 */
export function useWriting(options = {}) {
  const writingStore = useWritingStore()
  const campaignStore = useCampaignStore()
  const uiStore = useUiStore()

  const handlePipelineEvent = options.handlePipelineEvent || (() => {})
  const runPromptHookEvents = options.runPromptHookEvents || ((intent) => Promise.resolve(intent))
  const isPromptHooksCancelled = options.isPromptHooksCancelled || (() => false)
  const startWritingApi = options.startWritingApi || apiStartWriting
  const getConversationApi = options.getConversationApi || getConversation
  const cancelWritingApi = options.cancelWritingApi || apiCancelWriting
  const applyConversation = options.applyConversation || (() => {})
  const broadcastPluginEvent = options.broadcastPluginEvent || (() => {})
  const messageEventPayload = options.messageEventPayload || (() => ({}))
  const scrollToBottom = options.scrollToBottom || (() => {})
  const loadInstanceNameMap = options.loadInstanceNameMap || (() => {})
  const alertDialog = options.alertDialog || ((msg) => { console.error('alertDialog(未注入):', msg) })

  // 来源 App.vue:314-317 getAssistantRoleLabel
  function getAssistantRoleLabel() {
    return assistantRoleLabel(
      writingStore.writingMode,
      campaignStore.activeCampaign?.name,
      campaignStore.activeChar?.name,
    )
  }

  // 来源 App.vue:696-809 startWriting
  // onPipelineEventOverride：单次覆盖 handlePipelineEvent（供调用方临时替换）。
  async function startWriting(intent, skipLocalPush = false, onPipelineEventOverride) {
    const onPipelineEvent = typeof onPipelineEventOverride === 'function' ? onPipelineEventOverride : handlePipelineEvent

    // 写作前检查：必须有活跃连接
    if (!writingStore.activeConnection) {
      uiStore.showConnConfig = true
      return
    }
    // 三态检查：有 Campaign → Campaign 写作；无 Campaign 有角色 → legacy；都没有 → 阻止
    if (writingStore.writingMode === 'none') {
      await alertDialog('请先导入角色卡或打开一个 Campaign，再开始写作。')
      return
    }
    // 防止并发写入
    if (writingStore.isWriting) return
    writingStore.showPipeline = true
    writingStore.isWriting = true
    writingStore.pipeline.state = 'running'
    writingStore.pipeline.stateLabel = '准备中'
    writingStore.pipeline.director = { status: 'idle', detail: '', output: '' }
    writingStore.pipeline.subagents = []
    writingStore.pipeline.editor = { status: 'idle', detail: '', output: '' }
    writingStore.pipeline.postprocess = { status: 'idle', detail: '', knowledge: 0, variable: 0, task: 0, reason: '' }
    writingStore.pipeline.quality = null
    // 清除编剧流式消息占位（上次写作残留）
    writingStore.messages = writingStore.messages.filter((m) => m.id !== 'editor-streaming')

    // 本地 push user 消息（即时反馈，skipLocalPush 时跳过——用户消息已在列表中）
    let userMsgId = null
    if (!skipLocalPush) {
      userMsgId = `user-${Date.now()}`
      writingStore.messages.push({
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
      const charIdForWriting = writingStore.writingMode === 'campaign' ? null : campaignStore.activeChar?.id
      const hookedIntent = await runPromptHookEvents(intent)
      if (isPromptHooksCancelled()) throw createPromptHookCancelledError()
      const openingMessage = writingStore.writingMode === 'legacy' && !campaignStore.currentConversationId
        ? writingStore.selectedGreeting?.content || null
        : null
      const result = await startWritingApi(hookedIntent, charIdForWriting, (event) => {
        onPipelineEvent(event)
      }, campaignStore.currentConversationId, openingMessage, writingStore.generationMode)

      // 后端已存开场白、user 意图和 AI 成文；重拉会话以拿到展示态 regex 内容。
      const text = result.text
      const msgId = result.node_id || `msg-${Date.now()}`
      campaignStore.currentConversationId = result.conversation_id

      // 替换编剧流式占位。
      const streamingIdx = writingStore.messages.findIndex((m) => m.id === 'editor-streaming')
      if (streamingIdx >= 0) {
        writingStore.messages.splice(streamingIdx, 1)
      }
      const refreshed = await getConversationApi(campaignStore.currentConversationId)
      if (refreshed) {
        applyConversation(refreshed)
        writingStore.messages.forEach((m) => {
          if (m.role === 'assistant') m.role_label = getAssistantRoleLabel()
        })
      } else {
        writingStore.messages.push({
          id: msgId,
          role: 'assistant',
          role_label: getAssistantRoleLabel(),
          active_variant: 0,
          variants: [{
            id: `v-${Date.now()}`,
            content: text,
            display_content: text,
            // The pipeline has produced a Draft only. It remains mutable and
            // may be discarded until the explicit Accept command succeeds.
            status: 'draft',
            provenance: null,
          }],
        })
      }
      // A produced draft is deliberately not a received/committed message.
      // Only useMessageVariants.handleAcceptVariant may publish the terminal
      // MESSAGE_RECEIVED fan-out after the backend has accepted this variant.
      broadcastPluginEvent(ST_EVENT_TYPES.GENERATION_ENDED, messageEventPayload(msgId, {
        reason: 'draft_ready',
        draft: true,
        variantStatus: 'Draft',
      }))

      writingStore.pipeline.state = 'done'
      writingStore.pipeline.stateLabel = '已完成'
      writingStore.showPipeline = false // 写作完成，收起 StreamingMessage（成文消息已 push）
      loadInstanceNameMap() // 刷新实例名映射（可能新增临时实例）
      scrollToBottom()
    } catch (err) {
      // 失败回滚：移除已 push 的用户消息（无对应 AI 回复，残留会误导重试）
      if (userMsgId) {
        writingStore.messages = writingStore.messages.filter((m) => m.id !== userMsgId)
      }
      writingStore.messages = writingStore.messages.filter((m) => m.id !== 'editor-streaming')
      const cancelled = isPromptHookCancelledError(err)
      if (cancelled) writingStore.showPipeline = false
      writingStore.pipeline.state = cancelled ? 'idle' : 'error'
      writingStore.pipeline.stateLabel = cancelled ? '已停止' : `失败: ${err}`
    } finally {
      writingStore.isWriting = false
    }
  }

  // 来源 App.vue:812-819 cancelWriting
  async function cancelWriting() {
    try {
      // Cooperative cancel for in-flight frontend prompt hooks (if wired).
      try {
        options.cancelPromptHooks?.()
      } catch (hookCancelError) {
        console.error('取消 prompt hook 失败:', hookCancelError)
      }
      await cancelWritingApi()
      writingStore.pipeline.stateLabel = '正在停止…'
    } catch (e) {
      console.error('取消失败:', e)
    }
  }

  return { startWriting, cancelWriting }
}
