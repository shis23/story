/**
 * Build the ST-compatible first chat message the Destiny home shell expects
 * when the user finishes setup and clicks「开始旅程」.
 *
 * The card writes `chat[0].swipe_id` / `chat[0].swipes`, then calls
 * `SillyTavern.saveChat()` + `SillyTavern.reloadCurrentChat()`. Without a real
 * swipes array the button either no-ops or logs "environment not detected".
 */

/**
 * @param {Array<{ label?: string, content?: string } | string> | null | undefined} greetingOptions
 * @returns {string[]}
 */
export function buildOpeningChatSwipes(greetingOptions = []) {
  if (!Array.isArray(greetingOptions)) return []
  const swipes = []
  const seen = new Set()
  for (const option of greetingOptions) {
    const content = typeof option === 'string' ? option : option?.content
    const text = String(content || '').trim()
    if (!text || seen.has(text)) continue
    seen.add(text)
    swipes.push(String(content))
  }
  return swipes
}

/**
 * Seed value for `SillyTavern.chat[0]`.
 *
 * `swipe_id` starts at `-1` so the first selected scenario is always treated as
 * a change (the card only reloads when `swipe_id !== target`).
 *
 * @param {Array<{ label?: string, content?: string } | string> | null | undefined} greetingOptions
 * @param {{ selectedIndex?: number, name?: string }} [options]
 */
export function buildOpeningChatSeed(greetingOptions = [], options = {}) {
  const swipes = buildOpeningChatSwipes(greetingOptions)
  const name = String(options.name || 'Assistant')
  const selected = Number(options.selectedIndex)
  const fallbackMes = swipes[Number.isFinite(selected) && selected >= 0 ? selected : 0] || ''
  return {
    name,
    is_user: false,
    is_name: true,
    mes: fallbackMes,
    message: fallbackMes,
    // Force the first「开始旅程」click to take the swipe-switch branch.
    swipe_id: -1,
    swipes: swipes.length ? swipes : [fallbackMes || ''],
  }
}

/**
 * Rewrite the single opening assistant message in place with the chosen
 * scenario text. Returns the new messages array, or `null` when the
 * conversation is no longer in a rewritable opening state (not exactly one
 * assistant message) — callers must not touch history in that case.
 *
 * @param {Array<object> | null | undefined} messages
 * @param {string} content
 * @returns {Array<object> | null}
 */
export function rewriteOpeningMessages(messages, content) {
  const list = Array.isArray(messages) ? messages : []
  if (list.length !== 1 || list[0]?.role !== 'assistant') return null
  const first = list[0]
  const variants = Array.isArray(first.variants) ? first.variants : []
  if (variants.length) {
    const active = first.active_variant ?? 0
    const nextVariants = variants.map((variant, index) => (
      index === active
        ? { ...variant, content, display_content: content }
        : variant
    ))
    return [{ ...first, variants: nextVariants }]
  }
  return [{ ...first, content, display_content: content }]
}

/**
 * Map a shell swipe selection onto StoryForge greeting options / message text.
 *
 * @param {{ swipe_id?: number|null, mes?: string|null }} payload
 * @param {Array<{ label?: string, content?: string } | string> | null | undefined} greetingOptions
 */
export function resolveOpeningChatSelection(payload = {}, greetingOptions = []) {
  const swipes = buildOpeningChatSwipes(greetingOptions)
  const rawSwipe = payload?.swipe_id
  const swipeId = rawSwipe === null || rawSwipe === undefined || rawSwipe === ''
    ? null
    : Number(rawSwipe)
  const mes = typeof payload?.mes === 'string' ? payload.mes : ''

  if (Number.isFinite(swipeId) && swipeId >= 0 && swipeId < swipes.length) {
    return {
      greetingIndex: swipeId,
      content: swipes[swipeId],
    }
  }

  if (mes) {
    const byContent = swipes.findIndex((swipe) => swipe === mes)
    if (byContent >= 0) {
      return { greetingIndex: byContent, content: swipes[byContent] }
    }
    return { greetingIndex: null, content: mes }
  }

  return { greetingIndex: null, content: swipes[0] || '' }
}
