/**
 * 卡片 HTML 块：从展示文本里切出要在 Turn 尾部渲染的完整 HTML 块，其余作 DSH 正文。
 *
 * `splitRawHtmlBlocks` 与桌面 `frontend/src/utils/formatContent.js` 同口径（逐字移植；
 * `test/html.test.mjs` 用同一组样例和桌面实现对拍）：只认完整闭合的顶层
 * details / div / table / ul / ol / blockquote 块，未闭合的块与 `<data_block>` 一类数据
 * 标签留在文字里。
 */

import { readableText } from './text.js'

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
 * 原文 → 文字段与完整的顶层 HTML 块（按出现顺序）。没有块时返回与输入逐字相同的单个文字段。
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

/**
 * 展示文本 → 要渲染的 HTML 块（附文字版，浏览器半边净化失败时显示）。
 * @returns {Array<{html:string, text:string}>}
 */
export function renderableBlocks(display) {
  return splitRawHtmlBlocks(display)
    .filter((segment) => segment.type === 'html')
    .map((segment) => ({ html: segment.content, text: readableText(segment.content) }))
}

/**
 * 展示文本 → DSH 正文与交给 Turn 尾部渲染的块数：正文去掉这些块后转成可读文字。
 * 整条回复全是块时退回整段文字版、块数记 0（正文不能为空：空回复在 DSH 历史里
 * 不算一轮，下一轮会错位；文字版已在正文里，尾部不再重复）。
 * @returns {{body: string, blocks: number}}
 */
export function splitForDsh(display) {
  const segments = splitRawHtmlBlocks(display)
  const blocks = segments.filter((segment) => segment.type === 'html').length
  if (blocks === 0) return { body: readableText(display), blocks: 0 }
  const prose = segments
    .filter((segment) => segment.type === 'text')
    .map((segment) => segment.content)
    .join('\n\n')
  // 切走块后剩下的正文可能只剩卡壳占位（`<StatusPlaceHolderImpl/>`），照样按 HTML 去标签。
  const body = readableText(prose, { html: true })
  return body ? { body, blocks } : { body: readableText(display), blocks: 0 }
}
