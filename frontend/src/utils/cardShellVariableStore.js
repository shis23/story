/** Campaign variable key containing the selector-compatible state for card shells. */
export const CARD_SHELL_VARIABLES_KEY = '__storyforge_card_shell_variables'

const SUPPORTED_BUCKETS = new Set(['message', 'character', 'local', 'chat', 'global', 'script', 'preset'])

function isRecord(value) {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value)
}

function cloneRecord(value) {
  if (!isRecord(value)) return {}
  try {
    return JSON.parse(JSON.stringify(value))
  } catch {
    return {}
  }
}

/**
 * Keep the selector's stable identity in the persisted key. StoryForge does
 * not have a native ST message-variable entity, so the values still live in
 * the active Campaign, but one message must never overwrite another message.
 */
export function selectorKey(selector) {
  const type = String(selector?.type || 'local').toLowerCase()
  const safeType = SUPPORTED_BUCKETS.has(type) ? type : 'local'
  const identity = safeSelectorIdentity(selector, safeType)
  return `${safeType}:${identity}`
}

function safeSelectorIdentity(selector, type) {
  const candidates = type === 'message'
    ? [selector?.message_id, selector?.messageId]
    : type === 'preset'
      ? [selector?.preset_id, selector?.presetId, selector?.name]
      : type === 'character'
        ? [selector?.character_id, selector?.characterId]
        : type === 'script'
          ? [selector?.script_id, selector?.scriptId]
          : []
  const raw = candidates.find((value) => value !== null && value !== undefined && String(value).trim())
  return raw === undefined ? 'current' : encodeURIComponent(String(raw).trim()).slice(0, 256)
}

function isStoredSelectorKey(key) {
  const [type, identity] = String(key || '').split(':', 2)
  return SUPPORTED_BUCKETS.has(type) && Boolean(identity) && identity.length <= 256
}

export function normalizeCardShellVariables(value) {
  if (!isRecord(value)) return {}
  return Object.fromEntries(
    Object.entries(value)
      .flatMap(([key, variables]) => {
        if (!isRecord(variables)) return []
        // Migrate the prior type-only shape without silently losing existing
        // current Campaign data.
        const normalizedKey = SUPPORTED_BUCKETS.has(key) ? `${key}:current` : key
        return isStoredSelectorKey(normalizedKey) ? [[normalizedKey, cloneRecord(variables)]] : []
      }),
  )
}

/** Read the persisted selector snapshot from a Campaign variable DTO list. */
export function readCardShellVariables(campaignVariables) {
  const item = Array.isArray(campaignVariables)
    ? campaignVariables.find((entry) => entry?.key === CARD_SHELL_VARIABLES_KEY)
    : null
  return normalizeCardShellVariables(item?.value)
}

/** Return the next snapshot while retaining all selector buckets unrelated to this write. */
export function updateCardShellVariables(snapshot, selector, variables) {
  const next = normalizeCardShellVariables(snapshot)
  next[selectorKey(selector)] = cloneRecord(variables)
  return next
}

/** Apply a shallow selector patch against the latest persisted snapshot. */
export function mergeCardShellVariables(snapshot, selector, variables) {
  const next = normalizeCardShellVariables(snapshot)
  const key = selectorKey(selector)
  next[key] = { ...cloneRecord(next[key]), ...cloneRecord(variables) }
  return next
}

// Visible opening/status shells are separate iframe hosts. Serialize their
// read-modify-write cycles so a drawing save cannot erase a theme save made by
// the other surface a moment earlier.
const campaignWriteTails = new Map()

export function enqueueCardShellVariableMutation(campaignId, mutation) {
  const key = String(campaignId || '')
  if (!key) return mutation()

  const previous = campaignWriteTails.get(key) || Promise.resolve()
  const task = previous.catch(() => undefined).then(mutation)
  const tail = task.finally(() => {
    if (campaignWriteTails.get(key) === tail) campaignWriteTails.delete(key)
  })
  campaignWriteTails.set(key, tail)
  return task
}
