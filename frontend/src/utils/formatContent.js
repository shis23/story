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
  let s = text
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
