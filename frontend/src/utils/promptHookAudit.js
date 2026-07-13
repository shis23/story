/**
 * Prompt hook 审计记录导出。
 *
 * 每个审计记录应由 `buildAuditRecord` 脱敏（只存 type/length/hash，不存原始文本）。
 * 导出函数仍会二次消毒传入 records，防止调用方把原始字段混入环缓冲后被原样写出。
 */

const SENSITIVE_KEY_PATTERN = /(api[_-]?key|authorization|password|secret|token|private[_-]?memory|credential|prompt|messages|content|message|stack|raw)/i
const FORBIDDEN_VALUE_PATTERN = /(SF_SECRET_|(?:^|\W)sk-[A-Za-z0-9_-]{4,}|api[_-]?key|authorization|bearer\s+|password|credential)/i
const SAFE_FIELD_PATTERN = /^[A-Za-z][A-Za-z0-9_.:-]{0,95}$/
const SAFE_PLUGIN_LABEL_PATTERN = /^[\p{L}\p{N} ._:/-]{1,96}$/u
const SAFE_EVENT_PATTERN = /^[A-Z][A-Z0-9_]{0,95}$/
const SAFE_STAGE_PATTERN = /^[a-z][a-z0-9_]{0,63}$/
const SAFE_STATUSES = new Set([
  '',
  'ok',
  'no_change',
  'timeout',
  'cancelled',
  'error',
  'audit_error',
  'missing_host',
])
const SAFE_SUMMARY_TYPES = new Set([
  'array',
  'bigint',
  'boolean',
  'function',
  'number',
  'object',
  'sensitive',
  'string',
  'symbol',
  'undefined',
  'unreadable',
])

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

function redactLabel(value, fallback = '') {
  const text = String(value ?? fallback)
  return `<redacted:${hashString(text)}>`
}

function sanitizeFieldName(value) {
  const text = String(value ?? '')
  if (!SAFE_FIELD_PATTERN.test(text) || FORBIDDEN_VALUE_PATTERN.test(text)) {
    return redactLabel(text)
  }
  return text
}

function sanitizePluginLabel(value) {
  const text = String(value ?? '')
  if (!SAFE_PLUGIN_LABEL_PATTERN.test(text) || FORBIDDEN_VALUE_PATTERN.test(text)) {
    return redactLabel(text)
  }
  return text
}

function sanitizeEvent(value) {
  const text = String(value ?? '')
  return SAFE_EVENT_PATTERN.test(text) && !FORBIDDEN_VALUE_PATTERN.test(text)
    ? text
    : redactLabel(text)
}

function sanitizeStage(value) {
  const text = String(value ?? '')
  return (!text || (SAFE_STAGE_PATTERN.test(text) && !FORBIDDEN_VALUE_PATTERN.test(text)))
    ? text
    : redactLabel(text)
}

function sanitizeStatus(value) {
  const text = String(value ?? '')
  return SAFE_STATUSES.has(text) ? text : 'audit_error'
}

function sanitizeHash(value) {
  const text = String(value ?? '')
  return /^[a-f0-9]{8,64}$/i.test(text) ? text : hashString(text)
}

function sanitizeLength(value) {
  const number = Number(value)
  if (!Number.isSafeInteger(number) || number < 0) return 0
  return number
}

function summarizeExportValue(value, seen = new WeakSet()) {
  const valueType = typeof value
  if (valueType === 'string') {
    return { type: 'string', length: value.length, hash: hashString(value) }
  }
  if (valueType === 'bigint') {
    return { type: 'bigint', hash: hashString(value.toString()) }
  }
  if (valueType === 'number') {
    return { type: 'number', value: Number.isFinite(value) ? value : null }
  }
  if (valueType === 'boolean' || value == null) {
    return { type: valueType, value }
  }
  if (valueType === 'symbol' || valueType === 'function') {
    return { type: valueType }
  }
  if (Array.isArray(value)) {
    if (seen.has(value)) return { type: 'array', circular: true }
    seen.add(value)
    return {
      type: 'array',
      length: value.length,
      hash: hashString(String(value.length)),
    }
  }
  if (value && valueType === 'object') {
    if (seen.has(value)) return { type: 'object', circular: true }
    seen.add(value)
    const out = {}
    let keys = []
    try {
      keys = Object.keys(value)
    } catch {
      return { type: 'object', unreadable: true }
    }
    for (const key of keys) {
      const safeKey = sanitizeFieldName(key)
      if (SENSITIVE_KEY_PATTERN.test(key) && key !== 'messageHash' && key !== 'messageLength') {
        // Keep structured summary fields such as inputSummary.prompt.{type,length,hash}
        // only when they already look like safe summaries.
        const child = value[key]
        if (
          child
          && typeof child === 'object'
          && !Array.isArray(child)
          && ('type' in child)
          && !('message' in child)
          && !('content' in child)
        ) {
          out[safeKey] = sanitizeSummaryObject(child)
        } else {
          out[`<redacted:${hashString(key)}>`] = { type: 'sensitive', redacted: true }
        }
        continue
      }
      try {
        out[safeKey] = summarizeExportValue(value[key], seen)
      } catch {
        out[safeKey] = { type: 'unreadable' }
      }
    }
    return out
  }
  return { type: valueType }
}

function sanitizeSummaryObject(summary) {
  if (!summary || typeof summary !== 'object' || Array.isArray(summary)) {
    return summarizeExportValue(summary)
  }
  const out = {}
  for (const [key, value] of Object.entries(summary)) {
    if (key === 'type') {
      const type = String(value ?? '')
      out.type = SAFE_SUMMARY_TYPES.has(type) ? type : 'unreadable'
      continue
    }
    if (key === 'length' || key === 'messageLength') {
      out[key] = sanitizeLength(value)
      continue
    }
    if (key === 'hash' || key === 'messageHash') {
      out[key] = sanitizeHash(value)
      continue
    }
    if (key === 'circular' || key === 'redacted' || key === 'unreadable') {
      out[key] = Boolean(value)
      continue
    }
    if (key === 'value') {
      if (typeof value === 'boolean' || value == null || (typeof value === 'number' && Number.isFinite(value))) {
        out.value = value
      } else {
        out.valueSummary = summarizeExportValue(value)
      }
      continue
    }
    if (key === 'keys') {
      out.keyCount = Array.isArray(value) ? value.length : 0
      continue
    }
    const safeKey = sanitizeFieldName(key)
    if (SENSITIVE_KEY_PATTERN.test(key)) {
      if (
        value
        && typeof value === 'object'
        && !Array.isArray(value)
        && ('type' in value)
      ) {
        out[safeKey] = sanitizeSummaryObject(value)
      } else if (typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') {
        // Drop raw secret-bearing values that were incorrectly placed under summary keys.
        out[`<redacted:${hashString(key)}>`] = { type: 'sensitive', redacted: true }
      } else {
        out[safeKey] = sanitizeSummaryObject(value)
      }
      continue
    }
    if (value && typeof value === 'object' && !Array.isArray(value) && ('type' in value || Array.isArray(value.keys))) {
      out[safeKey] = sanitizeSummaryObject(value)
    } else if (typeof value === 'string') {
      out[safeKey] = { type: 'string', length: value.length, hash: hashString(value) }
    } else {
      out[safeKey] = summarizeExportValue(value)
    }
  }
  return out
}

function sanitizeError(error) {
  if (!error) return null
  if (typeof error !== 'object') {
    const message = String(error)
    return {
      name: 'Error',
      messageLength: message.length,
      messageHash: hashString(message),
    }
  }
  const rawName = String(error.name || 'Error')
  const name = /^[A-Za-z][A-Za-z0-9]{0,63}$/.test(rawName) && !FORBIDDEN_VALUE_PATTERN.test(rawName)
    ? rawName
    : 'Error'
  let message = ''
  try {
    message = String(error.message || '')
  } catch {
    message = '<unreadable error>'
  }
  return {
    name,
    messageLength: Number.isFinite(error.messageLength) ? sanitizeLength(error.messageLength) : message.length,
    messageHash: typeof error.messageHash === 'string' && error.messageHash
      ? sanitizeHash(error.messageHash)
      : hashString(message),
  }
}

/**
 * Sanitize one audit record for export. Strips raw bodies/secrets even if the
 * caller injected hostile fields into the ring buffer.
 *
 * @param {object} record
 * @returns {object}
 */
export function sanitizePromptHookAuditRecord(record) {
  if (!record || typeof record !== 'object') {
    return {
      kind: 'prompt_hook',
      status: 'audit_error',
      error: sanitizeError(record),
    }
  }

  return {
    kind: 'prompt_hook',
    pluginId: sanitizeFieldName(record.pluginId || ''),
    pluginName: sanitizePluginLabel(record.pluginName || ''),
    event: sanitizeEvent(record.event || ''),
    stage: sanitizeStage(record.stage || ''),
    status: sanitizeStatus(record.status || ''),
    durationMs: Number.isFinite(record.durationMs) && record.durationMs >= 0
      ? Math.round(record.durationMs)
      : 0,
    changedKeys: Array.isArray(record.changedKeys)
      ? record.changedKeys.slice(0, 128).map((key) => sanitizeFieldName(key))
      : [],
    inputSummary: sanitizeSummaryObject(record.inputSummary || {}),
    outputSummary: sanitizeSummaryObject(record.outputSummary || {}),
    error: sanitizeError(record.error),
  }
}

/**
 * 将 prompt hook 审计记录导出为可保存的 JSON 字符串。
 *
 * @param {Array<object>} records - 审计记录数组，每条由 `buildAuditRecord` 生成
 * @returns {string} 格式化的 JSON 字符串
 */
export function exportPromptHookAudit(records) {
  const safeRecords = Array.isArray(records)
    ? records.map((record) => sanitizePromptHookAuditRecord(record))
    : []
  return JSON.stringify(
    {
      exportedAt: new Date().toISOString(),
      kind: 'prompt_hook_audit_export',
      schema: {
        identity: ['pluginId', 'pluginName', 'event', 'stage'],
        timing: ['durationMs'],
        outcome: ['status', 'changedKeys', 'error'],
        redaction: ['inputSummary', 'outputSummary'],
        guarantees: [
          'no_full_prompt_bodies',
          'no_private_memory_text',
          'no_api_keys_or_secrets',
          'export_sanitizes_input_records',
        ],
      },
      totalRecords: safeRecords.length,
      records: safeRecords,
    },
    null,
    2,
  )
}

/**
 * 从 exportPromptHookAudit 的输出中解析回结构化数据。
 * 用于测试验证。
 *
 * @param {string} jsonStr
 * @returns {{ exportedAt: string, totalRecords: number, records: Array }}
 */
export function parsePromptHookAuditExport(jsonStr) {
  return JSON.parse(jsonStr)
}
