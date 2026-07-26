/**
 * Shell variable write proposals (ProposeVariableUpdate for card shells).
 *
 * Card JS can post `var_write` with any key/value — the bridge session token
 * lives in the shell document, so the message itself proves nothing. Writing
 * first-class campaign/instance variables directly from that channel equals a
 * prompt injection for the next turn. Instead, shell-initiated writes become
 * pending proposals; only an explicit user action (apply) persists them via
 * persistShellVariableWrite. User-initiated MVU interaction buttons keep the
 * direct path — the click *is* the confirmation.
 *
 * Pure list helpers: callers own the array (e.g. inside a Vue ref).
 */

let proposalSeq = 0

/** test hook: deterministic ids */
export function resetShellVariableProposalSeq() {
  proposalSeq = 0
}

/**
 * Add or refresh a pending proposal. One pending entry per key: a shell
 * spamming writes to the same key collapses to the latest value instead of
 * flooding the confirm surface.
 *
 * @param {Array<object>} list current proposals (not mutated)
 * @param {{ key?: unknown, value?: unknown }} payload from the var_write bridge
 * @param {{ limit?: number, source?: string }} [options]
 * @returns {Array<object>} next proposals list
 */
export function upsertShellVariableProposal(list, payload, options = {}) {
  const limit = options.limit ?? 50
  const key = String(payload?.key || '').trim()
  const current = Array.isArray(list) ? list : []
  if (!key) return current
  const entry = {
    id: `svp_${++proposalSeq}`,
    key,
    value: payload?.value,
    source: options.source || 'card-shell',
  }
  const next = current.filter((p) => p.key !== key)
  next.push(entry)
  // Oldest proposals drop first when a shell floods distinct keys.
  return next.length > limit ? next.slice(next.length - limit) : next
}

/**
 * @param {Array<object>} list
 * @param {string} id
 * @returns {{ proposal: object | null, rest: Array<object> }}
 */
export function takeShellVariableProposal(list, id) {
  const current = Array.isArray(list) ? list : []
  const proposal = current.find((p) => p.id === id) || null
  if (!proposal) return { proposal: null, rest: current }
  return { proposal, rest: current.filter((p) => p.id !== id) }
}

/** Human-readable value preview for the confirm surface. */
export function previewShellVariableValue(value, maxLength = 60) {
  let text
  if (value === undefined || value === null) {
    text = 'null'
  } else if (typeof value === 'string') {
    text = value
  } else {
    try {
      text = JSON.stringify(value)
    } catch {
      text = String(value)
    }
  }
  if (text.length > maxLength) return `${text.slice(0, maxLength - 1)}…`
  return text
}
