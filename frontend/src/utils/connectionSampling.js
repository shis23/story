export function normalizeOptionalMaxTokens(value) {
  const raw = String(value ?? '').trim()
  if (!raw) return null

  const parsed = Number(raw)
  if (!Number.isSafeInteger(parsed) || parsed <= 0) {
    throw new Error('max_tokens must be a positive integer')
  }
  return parsed
}
