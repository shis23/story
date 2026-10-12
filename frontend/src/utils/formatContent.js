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

const RAW_HTML_BLOCK_OPEN_RE = /<(details|div|table|ul|ol|blockquote)(?=[\s>/])[^>]*>/gi

function findBlockEnd(text, tag, from) {
  const re = new RegExp(`<(/?)${tag}(?=[\\s>/])[^>]*>`, 'gi')
  re.lastIndex = from
  let depth = 1
  let m
  while ((m = re.exec(text)) !== null) {
    depth += m[1] ? -1 : 1
    if (depth === 0) return m.index + m[0].length
  }
  return -1
}

/**
 * Split raw message text into prose and complete top-level HTML blocks.
 *
 * Cards whose opening carries an HTML status bar (<details>/<div>…) get the
 * model to repeat it every turn, so the backend quality gate accepts it as the
 * card's own format. Those blocks render as sanitized HTML; prose stays on the
 * escaped markdown path. Unknown/data tags (<data_block>, <StatusPlaceHolderImpl/>)
 * and blocks that are not closed yet (mid-stream) stay text.
 *
 * No block found → a single text segment byte-identical to the input.
 *
 * @param {string} text
 * @returns {Array<{type:'text'|'html', content:string}>}
 */
export function splitRawHtmlBlocks(text) {
  const source = String(text || '')
  const segments = []
  let cursor = 0
  const re = new RegExp(RAW_HTML_BLOCK_OPEN_RE.source, RAW_HTML_BLOCK_OPEN_RE.flags)
  let m
  while ((m = re.exec(source)) !== null) {
    const end = findBlockEnd(source, m[1], m.index + m[0].length)
    if (end < 0) break
    const before = source.slice(cursor, m.index).replace(/^(\r?\n)+/, '').replace(/(\r?\n)+$/, '')
    if (before.trim()) segments.push({ type: 'text', content: before })
    segments.push({ type: 'html', content: source.slice(m.index, end) })
    cursor = end
    re.lastIndex = end
  }
  if (!segments.length) return [{ type: 'text', content: source }]
  const rest = source.slice(cursor).replace(/^(\r?\n)+/, '')
  if (rest.trim()) segments.push({ type: 'text', content: rest })
  return segments
}
