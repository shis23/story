const KNOWLEDGE_SOURCE_LABELS = {
  witnessed: '👁 亲眼',
  told_by_other: '💬 被告知',
  inferred: '🔮 推断',
  backstory: '📖 背景',
}

export function routingText(routing) {
  if (!routing) return ''
  if (routing.Native || routing.kind === 'native') return '原生'
  const reason = routing.webview_reason || (routing.Hybrid && routing.Hybrid.webview_reason) || ''
  return `混合（${reason}）`
}

export function formatDiffValue(val) {
  if (val === null || val === undefined) return '（无）'
  if (typeof val === 'string') return val
  if (typeof val === 'object') return JSON.stringify(val, null, 2)
  return String(val)
}

export function knowledgeSourceText(source) {
  return KNOWLEDGE_SOURCE_LABELS[source] || source
}

export function shortId(id) {
  return id ? id.slice(0, 8) : ''
}

export function instanceLabel(k) {
  return k.character_name || (k.character_id ? `实例 ${shortId(k.character_id)}` : '未知角色')
}

export function sourceLabel(k) {
  return k.source_character_name || (k.source_character_id ? `实例 ${shortId(k.source_character_id)}` : '')
}

export function propagationText(k) {
  if (k.propagation === 'private') return '🔒 封口'
  if (k.propagation?.startsWith?.('group:')) return `限制 ${k.propagation.slice(6)}`
  return ''
}

export function relayChainText(k) {
  return k.relay_chain_text || (k.source_knowledge_id ? `上游 ${shortId(k.source_knowledge_id)}` : '')
}

export function inferVarType(value) {
  if (typeof value === 'boolean') return 'bool'
  if (typeof value === 'number') return Number.isInteger(value) ? 'int' : 'float'
  if (Array.isArray(value) || (typeof value === 'object' && value !== null)) return 'json'
  return 'string'
}

export function formatJsonValue(value) {
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}
