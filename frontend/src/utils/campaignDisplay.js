const KNOWLEDGE_SOURCE_LABELS = {
  witnessed: '👁 亲眼',
  told_by_other: '💬 被告知',
  inferred: '🔮 推断',
  backstory: '📖 背景',
}

const BUILTIN_VARIABLE_LABELS = {
  story_clock: '故事时间',
  weather: '天气',
  world_state: '世界大势',
  hp: '生命值',
  mp: '体力/精力',
  state: '状态',
  location: '位置',
  mood: '情绪',
  relationship_to_player: '与玩家关系',
  inventory: '随身物品',
  danger_level: '危险等级',
  affection: '好感度',
  trust: '信任度',
  fatigue: '疲劳度',
  health: '健康',
  stamina: '体力',
  mana: '魔力',
  nearby: '是否在附近',
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

export function parseVariableInput(value, type) {
  if (type === 'bool') return value === true || value === 'true'
  if (type === 'int') {
    const parsed = Number.parseInt(value, 10)
    return Number.isNaN(parsed) ? value : parsed
  }
  if (type === 'float') {
    const parsed = Number.parseFloat(value)
    return Number.isNaN(parsed) ? value : parsed
  }
  if (type === 'json') {
    try {
      return JSON.parse(value)
    } catch {
      return value
    }
  }
  return value
}

export function variableDisplayName(key, schemaLabel = '') {
  const normalizedKey = String(key || '').trim()
  const normalizedLabel = String(schemaLabel || '').trim()
  if (normalizedLabel && normalizedLabel !== normalizedKey) return normalizedLabel
  const builtIn = BUILTIN_VARIABLE_LABELS[normalizedKey.toLowerCase()]
  if (builtIn) return builtIn
  if (/[\u3400-\u9fff]/u.test(normalizedKey)) return normalizedKey
  return normalizedKey.replace(/[._/-]+/g, ' ')
}
