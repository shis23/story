/**
 * Lightweight Markdown formatter (ST style).
 * Supports: *italic (actions)* / **bold** / line breaks.
 * HTML-escapes input first to prevent XSS, then applies formatting.
 *
 * @param {string} text - Raw text to format
 * @returns {string} HTML string safe for v-html rendering
 */
export function formatContent(text) {
  if (!text) return ''
  // 1. HTML escape
  let s = String(text)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
  // 2. **bold** (before italic to avoid ** being consumed by *)
  s = s.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
  // 3. *italic* (actions/narration, ST standard)
  s = s.replace(/\*([^*\n]+)\*/g, '<em>$1</em>')
  // 4. Line breaks
  s = s.replace(/\n/g, '<br>')
  return s
}

const RENDERABLE_HTML_TAG_RE = /<\/?(?:div|span|p|br|strong|em|b|i|u|s|small|sub|sup|ruby|rt|rp|ul|ol|li|table|thead|tbody|tr|th|td|hr|blockquote|pre|code|details|summary|progress|meter)(?:\s|>|\/>)/i

/**
 * Only render HTML when it is display-only derived content.
 *
 * Raw ST data tags such as <data_block> are common in persisted message content
 * and must stay on the escaped text path unless a display regex transformed
 * them into a known renderable fragment.
 */
export function shouldRenderHtmlDisplay(displayContent, sourceContent) {
  if (!displayContent || displayContent === sourceContent) return false
  return RENDERABLE_HTML_TAG_RE.test(String(displayContent))
}
