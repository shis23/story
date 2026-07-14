import {
  canModifyPrompt,
  DEFAULT_PLUGIN_HOOK_TIMEOUT_MS,
} from '../plugin-bridge.js'

const PROMPT_HOOK_AUDIT_LIMIT = 100

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

const PROMPT_HOOK_FAIL_OPEN_STATUSES = new Set(['error', 'timeout', 'budget_exceeded', 'missing_host', 'revoked', 'no_change', 'ok'])

/**
 * Machine-readable fail policy for prompt hook outcomes.
 *
 * Mutating pre-generation hooks fail-open on plugin error / timeout / budget /
 * missing host (the turn continues with the fallback payload), but fail-closed
 * on cancellation (the whole turn must abort). This makes the classification
 * explicit and testable instead of an implicit runtime invariant.
 */
export function classifyPromptHookFailurePolicy(stage, operationType, status) {
  if (status === 'cancelled') {
    return { failOpen: false, reason: 'cancellation_aborts_turn' }
  }
  if (status === 'audit_error') {
    return { failOpen: true, reason: 'audit_error_is_non_fatal' }
  }
  if (PROMPT_HOOK_FAIL_OPEN_STATUSES.has(status)) {
    return { failOpen: true, reason: `${status}_continues_chain` }
  }
  // Unknown status: conservatively fail-open for non-mutating surfaces but
  // surface an explicit reason so callers can detect the gap.
  return { failOpen: true, reason: 'unknown_status_defaulted_fail_open' }
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

    const beforePayload = payload
    const startedAt = nowMs()
    try {
      throwIfAborted()
      const pending = host.emitPluginEventAndWait(event, payload)
      const abortable = withAbort(pending, signal)
      const nextPayload = timeoutMs === null
        ? await abortable
        : await withTimeout(abortable, timeoutMs)
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
        payload = nextPayload
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
          cancelled ? 'cancelled' : timedOut ? 'timeout' : 'error',
          startedAt,
          beforePayload,
          payload,
          error,
          auditContext,
        ),
      )
      if (cancelled) throw error
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

export function resolveHookedMessages(payload, fallbackMessages) {
  return Array.isArray(payload?.messages) ? payload.messages : fallbackMessages
}
