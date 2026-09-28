import {
  canModifyPrompt,
  DEFAULT_PLUGIN_HOOK_TIMEOUT_MS,
} from '../plugin-bridge.js'

const PROMPT_HOOK_AUDIT_LIMIT = 100

// M-21b：整条 hook 链的墙钟总预算（每次 emitPromptHookEventAndWaitForPlugins 调用）。
// 单插件 5s 超时挡不住 N 个挂死插件串行阻塞写作前流程（最坏 5s × N × 2 事件），
// 超预算后不再调用后续插件的 host，并按剩余额度收紧单插件超时。
// options.totalBudgetMs === null 显式关闭（测试/应急旁路，与 timeoutMs 同约定）。
export const DEFAULT_PROMPT_HOOK_CHAIN_BUDGET_MS = 15000

// M-21a：后端 ChatMessage.role 的取值域（crates/domain/src/llm.rs ChatRole，
// serde rename_all = "lowercase"）。
const PROMPT_HOOK_MESSAGE_ROLES = new Set(['system', 'user', 'assistant', 'tool'])

function nowMs() {
  return typeof performance !== 'undefined' && typeof performance.now === 'function'
    ? performance.now()
    : Date.now()
}

function hashString(value) {
  let text = ''
  try {
    text = String(value ?? '')
  } catch {
    text = '<unstringifiable>'
  }
  let hash = 2166136261
  for (let i = 0; i < text.length; i += 1) {
    hash ^= text.charCodeAt(i)
    hash = Math.imul(hash, 16777619)
  }
  return (hash >>> 0).toString(16).padStart(8, '0')
}

const SENSITIVE_AUDIT_KEY_PATTERN = /(api[_-]?key|authorization|password|secret|token|private[_-]?memory|credential)/i

function isSensitiveAuditKey(key) {
  return SENSITIVE_AUDIT_KEY_PATTERN.test(String(key || ''))
}

function withTimeout(promise, timeoutMs, onTimeout) {
  if (!Number.isFinite(timeoutMs) || timeoutMs < 0) {
    return Promise.resolve(promise)
  }
  let timer = null
  return new Promise((resolve, reject) => {
    let settled = false
    const finish = (fn, value) => {
      if (settled) return
      settled = true
      if (timer) clearTimeout(timer)
      fn(value)
    }
    timer = setTimeout(() => {
      try {
        onTimeout?.()
      } catch {
        // Timeout side effects must not fail closed.
      }
      const error = new Error(`Plugin hook timed out after ${timeoutMs}ms`)
      error.code = 'PROMPT_HOOK_TIMEOUT'
      finish(reject, error)
    }, timeoutMs)
    Promise.resolve(promise).then(
      (value) => finish(resolve, value),
      (error) => finish(reject, error),
    )
  })
}

export function createPromptHookCancelledError() {
  const error = new Error('Plugin hook generation cancelled')
  error.name = 'AbortError'
  error.code = 'PROMPT_HOOK_CANCELLED'
  return error
}

export function isPromptHookCancelledError(error) {
  return error?.code === 'PROMPT_HOOK_CANCELLED'
}

function withAbort(promise, signal) {
  if (!signal) return Promise.resolve(promise)
  if (signal.aborted) return Promise.reject(createPromptHookCancelledError())

  return new Promise((resolve, reject) => {
    let settled = false
    const finish = (callback, value) => {
      if (settled) return
      settled = true
      signal.removeEventListener('abort', onAbort)
      callback(value)
    }
    const onAbort = () => finish(reject, createPromptHookCancelledError())
    signal.addEventListener('abort', onAbort, { once: true })
    Promise.resolve(promise).then(
      (value) => finish(resolve, value),
      (error) => finish(reject, error),
    )
  })
}

function safeStableStringify(value) {
  try {
    return JSON.stringify(value)
  } catch {
    return JSON.stringify({ type: 'unserializable', hash: hashString(value) })
  }
}

function summaryContainsCircular(summary) {
  if (!summary || typeof summary !== 'object') return false
  if (summary.circular) return true
  if (Array.isArray(summary)) return summary.some(summaryContainsCircular)
  return Object.values(summary).some(summaryContainsCircular)
}

function summarizeHookValue(value, seen = new WeakSet()) {
  const valueType = typeof value
  if (valueType === 'string') {
    return { type: 'string', length: value.length, hash: hashString(value) }
  }
  if (valueType === 'bigint') {
    return { type: 'bigint', hash: hashString(value.toString()) }
  }
  if (valueType === 'symbol' || valueType === 'function') {
    return { type: valueType }
  }
  if (Array.isArray(value)) {
    if (seen.has(value)) {
      return { type: 'array', circular: true }
    }
    seen.add(value)
    const children = value.map((item) => summarizeHookValue(item, seen))
    return {
      type: 'array',
      length: value.length,
      circular: children.some(summaryContainsCircular) || undefined,
      hash: hashString(safeStableStringify(children)),
    }
  }
  if (value && typeof value === 'object') {
    if (seen.has(value)) {
      return { type: 'object', circular: true }
    }
    seen.add(value)
    let keys = []
    try {
      keys = Object.keys(value).sort()
    } catch (error) {
      return {
        type: 'object',
        unreadable: true,
        error: summarizeHookError(error),
      }
    }
    const childSummaries = keys.map((key) => {
      try {
        if (isSensitiveAuditKey(key)) {
          return [key, { type: 'sensitive', redacted: true }]
        }
        return [key, summarizeHookValue(value[key], seen)]
      } catch (error) {
        return [key, { type: 'unreadable', error: summarizeHookError(error) }]
      }
    })
    return {
      type: 'object',
      keys: keys.map((key) => (isSensitiveAuditKey(key) ? `<redacted:${hashString(key)}>` : key)),
      circular: childSummaries.some(([, child]) => summaryContainsCircular(child)) || undefined,
      hash: hashString(safeStableStringify(childSummaries.map(([key, child]) => (
        isSensitiveAuditKey(key) ? [`<redacted:${hashString(key)}>`, child] : [key, child]
      )))),
    }
  }
  if (valueType === 'number') {
    return { type: 'number', value: Number.isFinite(value) ? value : String(value) }
  }
  if (valueType === 'boolean' || value == null) {
    return { type: valueType, value }
  }
  return { type: valueType }
}

export function summarizePromptHookPayload(payload = {}) {
  if (!payload || typeof payload !== 'object') return summarizeHookValue(payload)
  const summary = {}
  let keys = []
  try {
    keys = Object.keys(payload).sort()
  } catch (error) {
    return { __payload: { type: 'object', unreadable: true, error: summarizeHookError(error) } }
  }
  for (const key of keys) {
    const summaryKey = isSensitiveAuditKey(key) ? `<redacted:${hashString(key)}>` : key
    try {
      if (isSensitiveAuditKey(key)) {
        summary[summaryKey] = { type: 'sensitive', redacted: true }
        continue
      }
      summary[summaryKey] = summarizeHookValue(payload[key])
    } catch (error) {
      summary[summaryKey] = { type: 'unreadable', error: summarizeHookError(error) }
    }
  }
  return summary
}

function changedKeysFromSummaries(beforeSummary = {}, afterSummary = {}) {
  const keys = new Set([
    ...Object.keys(beforeSummary || {}),
    ...Object.keys(afterSummary || {}),
  ])
  return Array.from(keys)
    .filter((key) => safeStableStringify(beforeSummary?.[key]) !== safeStableStringify(afterSummary?.[key]))
    .sort()
}

export function promptHookChangedKeys(beforePayload = {}, afterPayload = {}) {
  return changedKeysFromSummaries(
    summarizePromptHookPayload(beforePayload),
    summarizePromptHookPayload(afterPayload),
  )
}

export function appendPromptHookAuditRecord(records, record, limit = PROMPT_HOOK_AUDIT_LIMIT) {
  const nextRecords = [
    ...(Array.isArray(records) ? records : []),
    record,
  ]
  return nextRecords.slice(-limit)
}

function emitAudit(options, record) {
  try {
    options?.onAudit?.(record)
  } catch {
    // Audit reporting should not make prompt hooks fail closed.
  }
}

function summarizeHookError(error) {
  if (!error) return null
  let name = 'Error'
  let message = ''
  try {
    name = error?.name || 'Error'
  } catch {
    name = 'Error'
  }
  try {
    message = String(error?.message || error)
  } catch {
    message = '<unreadable error>'
  }
  return {
    name,
    messageLength: message.length,
    messageHash: hashString(message),
  }
}

function buildAuditRecord(plugin, event, stage, status, startedAt, beforePayload, afterPayload, error = null, context = {}) {
  try {
    const finishedAt = nowMs()
    const safeAfterPayload = afterPayload === undefined ? beforePayload : afterPayload
    const inputSummary = summarizePromptHookPayload(beforePayload)
    const outputSummary = summarizePromptHookPayload(safeAfterPayload)
    return {
      kind: 'prompt_hook',
      pluginId: plugin?.id || '',
      pluginName: plugin?.manifest?.name || plugin?.name || plugin?.id || '',
      event,
      stage: stage || '',
      status,
      durationMs: Math.max(0, Math.round(finishedAt - startedAt)),
      changedKeys: changedKeysFromSummaries(inputSummary, outputSummary),
      inputSummary,
      outputSummary,
      error: summarizeHookError(error),
      correlationId: context.correlationId || null,
      generationId: context.generationId || null,
    }
  } catch (auditError) {
    return {
      kind: 'prompt_hook',
      pluginId: plugin?.id || '',
      pluginName: plugin?.id || '',
      event,
      stage: stage || '',
      status: 'audit_error',
      durationMs: 0,
      changedKeys: [],
      inputSummary: {},
      outputSummary: {},
      error: summarizeHookError(auditError),
      correlationId: context.correlationId || null,
      generationId: context.generationId || null,
    }
  }
}

const PROMPT_HOOK_FAIL_OPEN_STATUSES = new Set(['error', 'timeout', 'budget_exceeded', 'missing_host', 'revoked', 'no_change', 'ok', 'invalid_mutation', 'chain_budget_exceeded'])

/**
 * Machine-readable fail policy for prompt hook outcomes.
 *
 * Mutating pre-generation hooks fail-open on plugin error / timeout / budget /
 * missing host (the turn continues with the fallback payload), but fail-closed
 * on cancellation (the whole turn must abort). This makes the classification
 * explicit and testable instead of an implicit runtime invariant.
 *
 * M-29：本函数是失败策略的唯一来源——emitPromptHookEventAndWaitForPlugins 的
 * catch 分支直接用它决定「继续链还是中止 turn」，不再内联同一判断，避免
 * 「文档化策略表」与运行时代码漂移。
 */
export function classifyPromptHookFailurePolicy(stage, operationType, status) {
  const surface = `${stage || 'unknown_stage'}:${operationType || 'unknown_operation'}`
  if (status === 'cancelled') {
    return { failOpen: false, reason: 'cancellation_aborts_turn', surface }
  }
  if (status === 'audit_error') {
    return { failOpen: true, reason: 'audit_error_is_non_fatal', surface }
  }
  if (PROMPT_HOOK_FAIL_OPEN_STATUSES.has(status)) {
    return { failOpen: true, reason: `${status}_continues_chain`, surface }
  }
  // Unknown status: conservatively fail-open for non-mutating surfaces but
  // surface an explicit reason so callers can detect the gap.
  return { failOpen: true, reason: 'unknown_status_defaulted_fail_open', surface }
}

function payloadByteSize(payload) {
  let serialized = ''
  try {
    serialized = JSON.stringify(payload ?? {})
  } catch {
    // Unserializable payload: estimate via the summarized hash material.
    serialized = safeStableStringify(payload)
  }
  const text = typeof serialized === 'string' ? serialized : String(serialized ?? '')
  if (typeof TextEncoder !== 'undefined') {
    return new TextEncoder().encode(text).byteLength
  }
  // WebView fallback for older runtimes without TextEncoder. Count UTF-8 bytes
  // directly rather than treating UTF-16 code units as bytes.
  let bytes = 0
  for (let index = 0; index < text.length; index += 1) {
    const code = text.charCodeAt(index)
    if (code < 0x80) bytes += 1
    else if (code < 0x800) bytes += 2
    else if (code >= 0xd800 && code <= 0xdbff && index + 1 < text.length) {
      const next = text.charCodeAt(index + 1)
      if (next >= 0xdc00 && next <= 0xdfff) {
        bytes += 4
        index += 1
      } else {
        bytes += 3
      }
    } else bytes += 3
  }
  return bytes
}

export async function emitPromptHookEventAndWaitForPlugins(plugins, hostRefs, event, data = {}, options = {}) {
  let payload = data
  // Default to DEFAULT_PLUGIN_HOOK_TIMEOUT_MS when callers omit timeoutMs.
  // Explicit null disables timeout (tests / emergency bypass only).
  let timeoutMs = null
  if (options.timeoutMs === null) {
    timeoutMs = null
  } else if (Number.isFinite(options.timeoutMs)) {
    timeoutMs = Math.max(0, options.timeoutMs)
  } else {
    timeoutMs = DEFAULT_PLUGIN_HOOK_TIMEOUT_MS
  }
  const isCancelled = typeof options.isCancelled === 'function' ? options.isCancelled : null
  const signal = options.signal || null
  // M-21b：整链墙钟预算（null = 关闭）。每次调用独立计时。
  const totalBudgetMs = options.totalBudgetMs === null
    ? null
    : Number.isFinite(options.totalBudgetMs)
      ? Math.max(0, options.totalBudgetMs)
      : DEFAULT_PROMPT_HOOK_CHAIN_BUDGET_MS
  const chainStartedAt = nowMs()
  // M-21b：被预算夹住的超时一旦触发，剩余预算即视为耗尽（确定性判定，不依赖
  // Date.now() 毫秒取整；见下方 timeoutClampedByBudget）。
  let chainBudgetExhausted = false
  // M-21a：链起点的 messages 是「原 system 消息必须保留」的比对基线（后端下发的
  // 原始提示词骨架）。前端 intent 阶段可能没有 system 消息，此时该校验自然跳过。
  const baselineMessages = Array.isArray(data?.messages) ? data.messages : null
  const getPluginPermissions = typeof options.getPluginPermissions === 'function'
    ? options.getPluginPermissions
    : null
  const maxPayloadBytes = Number.isFinite(options.maxPayloadBytesPerPlugin)
    ? Math.max(0, options.maxPayloadBytesPerPlugin)
    : null
  // Correlation / generation ids stamped on every async audit record so late
  // responses and overlapping generations stay traceable without raw payloads.
  const auditContext = {
    correlationId: options.correlationId || null,
    generationId: options.generationId || null,
  }

  const throwIfAborted = () => {
    if (signal?.aborted) {
      throw createPromptHookCancelledError()
    }
  }

  for (const plugin of plugins || []) {
    // Declaration-time permission gate: plugins that never declared
    // ModifyPrompt are silently skipped (no audit row, preserves legacy
    // behavior for read-only plugins in the hook list).
    if (!canModifyPrompt(plugin)) continue

    // Runtime permission re-check: when a resolver is provided, a plugin that
    // declared ModifyPrompt but lost it at runtime (revoked / disabled) is
    // audited as 'revoked' and skipped without running its hook.
    if (getPluginPermissions) {
      const runtimePlugin = { ...plugin, permissions: getPluginPermissions(plugin) || [] }
      if (!canModifyPrompt(runtimePlugin)) {
        emitAudit(options, buildAuditRecord(plugin, event, options.stage, 'revoked', nowMs(), payload, payload, null, auditContext))
        continue
      }
    }

    if (signal?.aborted) {
      emitAudit(options, buildAuditRecord(plugin, event, options.stage, 'cancelled', nowMs(), payload, payload, null, auditContext))
      throw createPromptHookCancelledError()
    }
    if (isCancelled?.()) {
      emitAudit(options, buildAuditRecord(plugin, event, options.stage, 'cancelled', nowMs(), payload, payload, null, auditContext))
      continue
    }

    const host = hostRefs?.get?.(plugin.id)
    if (!host?.emitPluginEventAndWait) {
      emitAudit(options, buildAuditRecord(plugin, event, options.stage, 'missing_host', nowMs(), payload, payload, null, auditContext))
      continue
    }

    // M-21b：预算耗尽后不再调用任何 host；未耗尽时也要把单插件超时收紧到剩余
    // 额度，否则链尾的一个挂死插件仍会让总时长越过预算。
    const remainingBudgetMs = totalBudgetMs === null
      ? null
      : totalBudgetMs - (nowMs() - chainStartedAt)
    if (chainBudgetExhausted || (remainingBudgetMs !== null && remainingBudgetMs <= 0)) {
      emitAudit(options, buildAuditRecord(plugin, event, options.stage, 'chain_budget_exceeded', nowMs(), payload, payload, null, auditContext))
      continue
    }
    const effectiveTimeoutMs = remainingBudgetMs === null
      ? timeoutMs
      : timeoutMs === null
        ? remainingBudgetMs
        : Math.min(timeoutMs, remainingBudgetMs)
    // M-21b：这次调用吃的是"预算本身"（被预算夹住的超时）⇒ 一旦它超时，剩余预算
    // 必然已被耗尽。显式记下来，避免只靠 `Date.now()` 毫秒取整判断（定时器可能提前
    // <1ms 触发，导致 remaining=1ms 时又白调一个插件，行为不稳定）。
    const timeoutClampedByBudget = remainingBudgetMs !== null && effectiveTimeoutMs === remainingBudgetMs

    const beforePayload = payload
    const startedAt = nowMs()
    try {
      throwIfAborted()
      const pending = host.emitPluginEventAndWait(event, payload)
      const abortable = withAbort(pending, signal)
      const nextPayload = effectiveTimeoutMs === null
        ? await abortable
        : await withTimeout(abortable, effectiveTimeoutMs)
      throwIfAborted()
      if (nextPayload !== undefined) {
        // Per-plugin payload size budget: discard an oversized mutation and
        // continue the chain on the pre-mutation payload (fail-open).
        if (maxPayloadBytes !== null && payloadByteSize(nextPayload) > maxPayloadBytes) {
          emitAudit(options, buildAuditRecord(
            plugin,
            event,
            options.stage,
            'budget_exceeded',
            startedAt,
            beforePayload,
            payload,
            null,
            auditContext,
          ))
          continue
        }
        // M-21a：messages 结构校验（角色白名单 / 非空 / 保留原 system 消息）。
        // 非法变更审计为 invalid_mutation。若该插件只动了 messages（其它字段没变），
        // 整份忽略、链沿用原 payload 引用（旧语义）；若它同时改了 intent/prompt 等
        // 其它字段，则**只回退 messages**，不丢弃整份 payload —— 否则首轮
        // `messages: []` 的 intent hook 阶段（常态）里插件改 intent 会静默全丢。
        let sanitizedNextPayload = nextPayload
        let messagesRolledBack = false
        if (nextPayload && typeof nextPayload === 'object' && nextPayload.messages !== undefined) {
          const verdict = validateHookedMessages(nextPayload.messages, { originalMessages: baselineMessages })
          if (!verdict.ok) {
            emitAudit(options, buildAuditRecord(
              plugin,
              event,
              options.stage,
              'invalid_mutation',
              startedAt,
              beforePayload,
              payload,
              null,
              auditContext,
            ))
            const restoredMessages = { ...nextPayload }
            if (payload && typeof payload === 'object' && 'messages' in payload) {
              restoredMessages.messages = payload.messages
            } else {
              delete restoredMessages.messages
            }
            // 纯 messages 非法（其余字段未变）⇒ 保持旧语义：整份丢弃，只留上面那条审计。
            if (JSON.stringify(payload ?? null) === JSON.stringify(restoredMessages ?? null)) {
              continue
            }
            sanitizedNextPayload = restoredMessages
            messagesRolledBack = true
          }
        }
        payload = sanitizedNextPayload
        // M-08：记录"实际改写过 payload"的插件 id，供调用方向后端声明改写者
        // （pluginPromptHookResult 的 modifierPluginIds）。
        if (Array.isArray(options.appliedPluginIds) && !options.appliedPluginIds.includes(plugin.id)) {
          options.appliedPluginIds.push(plugin.id)
        }
        if (messagesRolledBack) {
          // 本 phase 已发过 invalid_mutation（说明 messages 被回退），不再重复发 ok；
          // 其余字段的改写已生效，appliedPluginIds 也已记录。
          continue
        }
      }
      emitAudit(options, buildAuditRecord(
        plugin,
        event,
        options.stage,
        nextPayload === undefined ? 'no_change' : 'ok',
        startedAt,
        beforePayload,
        payload,
        null,
        auditContext,
      ))
    } catch (error) {
      const cancelled = isPromptHookCancelledError(error)
      const timedOut = error?.code === 'PROMPT_HOOK_TIMEOUT'
        || /timed out/i.test(String(error?.message || error || ''))
      const status = cancelled ? 'cancelled' : timedOut ? 'timeout' : 'error'
      // 这次超时吃的就是全部剩余预算 ⇒ 后续插件不再被调用（M-21b 确定性判定）。
      if (timeoutClampedByBudget && timedOut) chainBudgetExhausted = true
      try {
        options?.onError?.(error, plugin)
      } catch {
        // Error reporting should not make prompt hooks fail closed.
      }
      emitAudit(
        options,
        buildAuditRecord(
          plugin,
          event,
          options.stage,
          status,
          startedAt,
          beforePayload,
          payload,
          error,
          auditContext,
        ),
      )
      // M-29：失败策略的唯一来源是 classifyPromptHookFailurePolicy（上方的文档化
      // 策略表）。此处只翻译「是否 fail-open」，不再内联同一判断。
      if (!classifyPromptHookFailurePolicy(options.stage, 'prompt_hook', status).failOpen) throw error
    }
  }

  return payload
}

export function resolveHookedIntent(payload, fallbackIntent) {
  if (typeof payload?.intent === 'string') {
    return payload.intent
  }
  if (typeof payload?.prompt === 'string') {
    return payload.prompt
  }
  return fallbackIntent
}

/**
 * M-21a：prompt hook 返回的 messages 必须通过结构校验才会被采用。
 *
 * hook 返回值直接回传后端 plugin_prompt_hook_result（Rust `Vec<ChatMessage>`），
 * 因此角色白名单 + content:string 是反序列化硬要求；空数组等于清空提示词，
 * 丢掉原 system 消息等于替换系统指令——两者都必须拒绝。插件仍可新增自己的
 * system 消息（原始那条必须原样在列），只是不能抹掉后端下发的提示词骨架。
 *
 * @param {unknown} messages
 * @param {{originalMessages?: unknown}} [options]
 * @returns {{ok:true}|{ok:false, reason:string}}
 */
export function validateHookedMessages(messages, { originalMessages } = {}) {
  if (!Array.isArray(messages)) return { ok: false, reason: 'messages_not_an_array' }
  if (!messages.length) return { ok: false, reason: 'messages_empty' }
  for (let index = 0; index < messages.length; index += 1) {
    const message = messages[index]
    if (!message || typeof message !== 'object' || Array.isArray(message)) {
      return { ok: false, reason: `message_${index}_not_an_object` }
    }
    if (typeof message.role !== 'string' || !PROMPT_HOOK_MESSAGE_ROLES.has(message.role)) {
      return { ok: false, reason: `message_${index}_invalid_role` }
    }
    if (typeof message.content !== 'string') {
      return { ok: false, reason: `message_${index}_invalid_content` }
    }
  }
  const originalSystem = (Array.isArray(originalMessages) ? originalMessages : [])
    .find((message) => message && typeof message === 'object' && message.role === 'system')
  if (
    originalSystem
    && typeof originalSystem.content === 'string'
    && !messages.some((message) => message.role === 'system' && message.content === originalSystem.content)
  ) {
    return { ok: false, reason: 'system_message_not_preserved' }
  }
  return { ok: true }
}

function reportInvalidHookMutation(options, message) {
  try {
    if (typeof options?.onInvalid === 'function') {
      options.onInvalid(message)
      return
    }
    console.warn(`[promptHooks] ${message}`)
  } catch {
    // 报告失败不得让 turn fail closed（与 onError / onAudit 同约定）。
  }
}

export function resolveHookedMessages(payload, fallbackMessages, options = {}) {
  const fallback = Array.isArray(fallbackMessages) ? fallbackMessages : []
  if (!Array.isArray(payload?.messages)) return fallback
  const verdict = validateHookedMessages(payload.messages, { originalMessages: fallback })
  if (!verdict.ok) {
    reportInvalidHookMutation(
      options,
      `hooked messages rejected (${verdict.reason}); original messages used instead`,
    )
    return fallback
  }
  return payload.messages
}
