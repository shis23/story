export function safeFileName(name) {
  return String(name)
    .replace(/[\\/:*?"<>|]+/g, '-')
    .trim()
    .slice(0, 80) || 'agent-profile'
}

export function parseWhitelist(raw) {
  if (raw === null || raw === undefined) return null

  if (Array.isArray(raw)) {
    return raw.map((item) => String(item).trim()).filter(Boolean)
  }

  const text = String(raw).trim()
  if (text === '') return []

  return text
    .split(/[,\r\n]+/)
    .map((item) => item.trim())
    .filter(Boolean)
}

export function cleanForSave(cfg) {
  const out = { ...cfg }
  const cleanedConfigs = {}

  for (const role of Object.keys(cfg.agent_configs || {})) {
    const c = cfg.agent_configs[role] || {}
    const model = normalizeModel(c.model_override)
    const rounds = normalizeToolRounds(c.max_tool_rounds)
    const whitelist = c.tool_whitelistRaw !== undefined
      ? parseWhitelist(c.tool_whitelistRaw)
      : parseWhitelist(c.tool_whitelist)

    if (!model && rounds === null && whitelist === null) continue

    cleanedConfigs[role] = {
      model_override: model || null,
      max_tool_rounds: rounds,
      tool_whitelist: whitelist,
    }
  }

  out.agent_configs = cleanedConfigs
  out.max_concurrent_subagents = Number(out.max_concurrent_subagents) || 1
  delete out.tool_whitelistRaw
  return out
}

function normalizeModel(value) {
  if (value === null || value === undefined) return ''
  return String(value).trim()
}

function normalizeToolRounds(value) {
  if (value === '' || value === null || value === undefined) return null

  const rounds = Number(value)
  return Number.isFinite(rounds) ? rounds : null
}
