// Plugin Bridge composable — 迁移自 App.vue:129-282。
// 负责:plugin host ref 管理、插件事件广播、prompt hook 编排、
// chat/message 事件 payload 构造。
//
// 消费:usePluginStore(事件 feed / host refs / hook 插件列表 / slot 注册表 / 审计记录)、
// useWritingStore(messages / currentConversationId 读取)、useCampaignStore(writingMode / campaign / char 读取)。
// 只 import 不修改:tauri-api.js、plugin-bridge.js、utils/promptHooks.js。

import {
  DEFAULT_PLUGIN_HOOK_TIMEOUT_MS,
  ST_EVENT_TYPES,
} from '../plugin-bridge.js'
import { logAppendFrontend, pluginPromptHookResult } from '../tauri-api.js'
import {
  appendPromptHookAuditRecord,
  createPromptHookCancelledError,
  emitPromptHookEventAndWaitForPlugins,
  resolveHookedIntent,
  resolveHookedMessages,
} from '../utils/promptHooks.js'
import { sanitizePromptHookAuditRecord } from '../utils/promptHookAudit.js'
import { usePluginStore } from '../stores/plugin.js'
import { useWritingStore } from '../stores/writing.js'
import { useCampaignStore } from '../stores/campaign.js'

export function usePluginBridge() {
  const plugin = usePluginStore()
  const writing = useWritingStore()
  const campaign = useCampaignStore()

  // Production defaults: per-plugin timeout is always on; cancellation is scoped
  // to one write/reroll generation so late events cannot revive old hooks.
  let promptHookTimeoutMs = DEFAULT_PLUGIN_HOOK_TIMEOUT_MS
  let nextPromptHookGenerationId = 0
  let activePromptHookGeneration = null

  function setPromptHookTimeoutMs(timeoutMs) {
    const previous = promptHookTimeoutMs
    if (timeoutMs === undefined) {
      promptHookTimeoutMs = DEFAULT_PLUGIN_HOOK_TIMEOUT_MS
      return previous
    }
    if (timeoutMs === null) {
      promptHookTimeoutMs = null
      return previous
    }
    const next = Number(timeoutMs)
    promptHookTimeoutMs = Number.isFinite(next) ? Math.max(0, next) : DEFAULT_PLUGIN_HOOK_TIMEOUT_MS
    return previous
  }

  function beginPromptHookGeneration() {
    if (activePromptHookGeneration && !activePromptHookGeneration.controller.signal.aborted) {
      activePromptHookGeneration.controller.abort()
    }
    activePromptHookGeneration = {
      id: ++nextPromptHookGenerationId,
      controller: new AbortController(),
    }
    return activePromptHookGeneration.id
  }

  function ensurePromptHookGeneration() {
    if (!activePromptHookGeneration) beginPromptHookGeneration()
    return activePromptHookGeneration
  }

  function cancelPromptHooks() {
    const generation = ensurePromptHookGeneration()
    if (!generation.controller.signal.aborted) generation.controller.abort()
    return generation.id
  }

  function isPromptHooksCancelled() {
    return Boolean(activePromptHookGeneration?.controller.signal.aborted)
  }

  // ─── host ref 管理(App.vue:131-138)──────────────────────────────────
  // setHookPluginHostRef 在 App.vue 模板里作为 :ref 回调被调用,需要保持引用稳定。
  // pluginStore.setHookPluginHostRef 已实现 Map set/delete,这里直接转发。
  function setHookPluginHostRef(pluginId, el) {
    if (!pluginId) return
    plugin.setHookPluginHostRef(pluginId, el)
  }

  // onHookPluginSlotMount(App.vue:140-157):插件 UI slot 挂载事件 → 写 hookPluginSlots
  function onHookPluginSlotMount(mount) {
    const pluginId = mount?.pluginId
    const slot = mount?.slot
    if (!pluginId || !slot) return

    const html = typeof mount.html === 'string' ? mount.html : ''
    const pluginSlots = { ...(plugin.hookPluginSlots[pluginId] || {}) }
    if (html) {
      pluginSlots[slot] = html
    } else {
      delete pluginSlots[slot]
    }

    plugin.hookPluginSlots = {
      ...plugin.hookPluginSlots,
      [pluginId]: pluginSlots,
    }
  }

  // ─── 事件 feed 广播(App.vue:159-176)──────────────────────────────────
  // pushPluginEventRecord 给记录分配自增序号,裁剪到 MAX_PLUGIN_PIPELINE_EVENTS。
  function pushPluginEventRecord(record) {
    const seq = plugin.nextPipelineEventSeq()
    const nextEvents = [
      ...plugin.pluginPipelineEvents,
      { id: seq, ...record },
    ]
    plugin.pluginPipelineEvents = nextEvents.slice(-plugin.MAX_PLUGIN_PIPELINE_EVENTS)
  }

  function broadcastPluginPipelineEvent(event) {
    if (!event?.event_type) return
    pushPluginEventRecord({ event })
  }

  function broadcastPluginEvent(event, data = {}) {
    if (!event) return
    pushPluginEventRecord({ event, data })
  }

  // ─── prompt hook 审计(App.vue:178-185)──────────────────────────────
  function recordPromptHookAudit(record) {
    const safeRecord = sanitizePromptHookAuditRecord(record)
    plugin.promptHookAuditRecords = appendPromptHookAuditRecord(
      plugin.promptHookAuditRecords,
      safeRecord,
      plugin.MAX_PROMPT_HOOK_AUDIT_RECORDS,
    )
    logAppendFrontend('info', `prompt_hook_audit ${JSON.stringify(safeRecord)}`).catch(() => {})
  }

  // ─── payload 构造(App.vue:187-213)──────────────────────────────────
  function activeVariantForMessage(message) {
    return message?.variants?.[message.active_variant] || message?.variants?.[0] || null
  }

  function chatEventPayload(extra = {}) {
    return {
      conversationId: campaign.currentConversationId,
      messageCount: writing.messages.length,
      writingMode: writing.writingMode,
      campaignId: campaign.activeCampaign?.id || null,
      characterId: campaign.activeChar?.id || null,
      ...extra,
    }
  }

  function messageEventPayload(messageId, extra = {}) {
    const message = writing.messages.find((m) => m.id === messageId)
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

  // ─── 事件广播 + 等待(App.vue:215-237)───────────────────────────────
  // emitPluginEventAndWait 遍历所有 hook 插件的 host ref,串行调用其 emitPluginEventAndWait。
  async function emitPluginEventAndWait(event, data = {}) {
    let payload = data
    for (const hp of plugin.hookPlugins || []) {
      const host = plugin.getHookPluginHostRef(hp.id)
      if (host?.emitPluginEventAndWait) {
        payload = await host.emitPluginEventAndWait(event, payload)
      }
    }
    return payload
  }

  // emitPromptHookEventAndWait 委托 utils/promptHooks.js,传入 stage / timeout / cancel / onAudit。
  async function emitPromptHookEventAndWait(
    event,
    data = {},
    stage = '',
    generation = ensurePromptHookGeneration(),
  ) {
    if (generation.controller.signal.aborted) throw createPromptHookCancelledError()
    return await emitPromptHookEventAndWaitForPlugins(
      plugin.hookPlugins,
      plugin.hookPluginHostRefs,
      event,
      data,
      {
        stage,
        timeoutMs: promptHookTimeoutMs,
        signal: generation.controller.signal,
        onAudit: recordPromptHookAudit,
      },
    )
  }

  // ─── prompt hook 编排(App.vue:239-278)─────────────────────────────
  // runPromptHookEvents:写作前对用户意图依次触发两个 GENERATE hook,返回最终 intent。
  async function runPromptHookEvents(intent) {
    beginPromptHookGeneration()
    const generation = activePromptHookGeneration
    let payload = {
      intent,
      prompt: intent,
      messages: writing.messages.map((message) => ({
        id: message.id,
        role: message.role,
        content: activeVariantForMessage(message)?.content || '',
        displayContent: activeVariantForMessage(message)?.display_content || activeVariantForMessage(message)?.content || '',
      })),
      ...chatEventPayload(),
    }

    payload = await emitPromptHookEventAndWait(
      ST_EVENT_TYPES.GENERATE_BEFORE_COMBINE_PROMPTS,
      payload,
      'frontend_intent',
      generation,
    )
    payload = await emitPromptHookEventAndWait(
      ST_EVENT_TYPES.CHAT_COMPLETION_PROMPT_READY,
      payload,
      'frontend_intent',
      generation,
    )

    return resolveHookedIntent(payload, intent)
  }

  // handlePromptHookRequest:后端发来的 prompt_hook_request 事件处理。
  // 触发 hook → 把结果(或错误)回传后端 pluginPromptHookResult。
  async function handlePromptHookRequest(data = {}) {
    const requestId = data.request_id || data.requestId
    const originalMessages = Array.isArray(data.messages) ? data.messages : []
    if (!requestId) return

    if (!activePromptHookGeneration) beginPromptHookGeneration()
    const generation = activePromptHookGeneration
    try {
      if (generation.controller.signal.aborted) throw createPromptHookCancelledError()
      const payload = await emitPromptHookEventAndWait(ST_EVENT_TYPES.CHAT_COMPLETION_PROMPT_READY, {
        ...chatEventPayload({
          promptHookStage: 'llm_messages',
          role: data.role || null,
          round: data.round || 0,
          model: data.model || '',
        }),
        messages: originalMessages,
      }, 'llm_messages', generation)
      const messagesForBackend = resolveHookedMessages(payload, originalMessages)
      await pluginPromptHookResult(requestId, messagesForBackend, null)
    } catch (err) {
      await pluginPromptHookResult(requestId, originalMessages, err?.message || String(err))
    }
  }

  // ─── chat 变更广播(App.vue:280-282)─────────────────────────────────
  function broadcastChatChanged(reason, extra = {}) {
    broadcastPluginEvent(ST_EVENT_TYPES.CHAT_CHANGED, chatEventPayload({ reason, ...extra }))
  }

  return {
    // host ref 管理
    setHookPluginHostRef,
    onHookPluginSlotMount,
    // 事件 feed 广播
    pushPluginEventRecord,
    broadcastPluginPipelineEvent,
    broadcastPluginEvent,
    broadcastChatChanged,
    // prompt hook 审计
    recordPromptHookAudit,
    // payload 构造
    activeVariantForMessage,
    chatEventPayload,
    messageEventPayload,
    // 事件广播 + 等待
    emitPluginEventAndWait,
    emitPromptHookEventAndWait,
    // prompt hook 编排
    runPromptHookEvents,
    handlePromptHookRequest,
    // production timeout/cancel wiring
    setPromptHookTimeoutMs,
    beginPromptHookGeneration,
    cancelPromptHooks,
    isPromptHooksCancelled,
  }
}
