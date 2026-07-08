import { canModifyPrompt } from '../plugin-bridge.js'

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
        return [key, summarizeHookValue(value[key], seen)]
      } catch (error) {
        return [key, { type: 'unreadable', error: summarizeHookError(error) }]
      }
    })
    return {
      type: 'object',
      keys,
      circular: childSummaries.some(([, child]) => summaryContainsCircular(child)) || undefined,
      hash: hashString(safeStableStringify(childSummaries)),
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
    try {
      summary[key] = summarizeHookValue(payload[key])
    } catch (error) {
      summary[key] = { type: 'unreadable', error: summarizeHookError(error) }
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

function buildAuditRecord(plugin, event, stage, status, startedAt, beforePayload, afterPayload, error = null) {
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
    }
  }
}

export async function emitPromptHookEventAndWaitForPlugins(plugins, hostRefs, event, data = {}, options = {}) {
  let payload = data

  for (const plugin of plugins || []) {
    if (!canModifyPrompt(plugin)) continue

    const host = hostRefs?.get?.(plugin.id)
    if (!host?.emitPluginEventAndWait) {
      emitAudit(options, buildAuditRecord(plugin, event, options.stage, 'missing_host', nowMs(), payload, payload))
      continue
    }

    const beforePayload = payload
    const startedAt = nowMs()
    try {
      const nextPayload = await host.emitPluginEventAndWait(event, payload)
      if (nextPayload !== undefined) {
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
      ))
    } catch (error) {
      try {
        options?.onError?.(error, plugin)
      } catch {
        // Error reporting should not make prompt hooks fail closed.
      }
      emitAudit(options, buildAuditRecord(plugin, event, options.stage, 'error', startedAt, beforePayload, payload, error))
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
