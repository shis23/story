/**
 * An opening shell is an initial-setup surface, not part of the permanent
 * reading flow. It may coexist with the selected greeting, but never with an
 * established conversation or an in-progress turn.
 */
export function shouldShowOpeningShell({
  openingUrl,
  openingArmed = false,
  messageCount = 0,
  isWriting = false,
} = {}) {
  return Boolean(openingUrl) && openingArmed && messageCount <= 1 && !isWriting
}

/** Keep automatic shell height safe without turning it back into a tiny pane. */
export function normalizeShellHeight(value, { min = 160, max = 10_000 } = {}) {
  const height = Math.ceil(Number(value))
  if (!Number.isFinite(height) || height <= 0) return null
  return Math.min(Math.max(height, min), max)
}
