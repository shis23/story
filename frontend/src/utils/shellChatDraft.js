/**
 * Card shells add user messages through TavernHelper `createChatMessages`
 * (Destiny's custom start sends its generated character brief this way, then
 * `/trigger`). StoryForge only puts that text into the writing composer; the
 * user reads it and presses send, so a shell can never start a turn by itself.
 *
 * The composer registers itself while mounted (one at a time); card shells
 * mounted anywhere (opening slot, message content, docks) reach it here
 * without threading events through every parent.
 */

let target = null

/**
 * @param {(text: string) => 'drafted' | 'busy' | 'draft'} fill
 * @returns {() => void} unregister
 */
export function registerShellDraftTarget(fill) {
  target = fill
  return () => {
    if (target === fill) target = null
  }
}

/** Why a shell's text did not land in the composer (shown back to the card). */
export const SHELL_DRAFT_NOT_FILLED = Object.freeze({
  empty: '没有要填入的内容',
  'no-composer': '写作输入框不在当前页面',
  busy: '正在写作，输入框暂时不能填入',
  draft: '输入框里有没发出去的内容，没有覆盖',
})

/**
 * @param {unknown} text
 * @returns {'drafted' | 'empty' | 'no-composer' | 'busy' | 'draft'}
 */
export function offerShellChatDraft(text) {
  const value = typeof text === 'string' ? text : String(text ?? '')
  if (!value.trim()) return 'empty'
  if (!target) return 'no-composer'
  return target(value)
}
