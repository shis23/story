/**
 * 卡片 HTML 块：从展示文本里切出要在 Turn 尾部渲染的完整 HTML 块，其余作 DSH 正文。
 *
 * `splitRawHtmlBlocks` 与桌面 `frontend/src/utils/formatContent.js` 同口径（逐字移植；
 * `test/html.test.mjs` 用同一组样例和桌面实现对拍）：只认完整闭合的顶层
 * details / div / table / ul / ol / blockquote 块，未闭合的块与 `<data_block>` 一类数据
 * 标签留在文字里。
 */

import { readableText } from './text.js'
import { segmentShellContent } from './vendor/card-shell.mjs'

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
 * 去掉完整的 `<script>` 元素。档 1 不执行脚本（DOMPurify 也会去掉），而 MVU 状态栏一类卡
 * 会在脚本的字符串字面量里拼 `<div>…</div>`——不先去掉，块切分会把这些片段当成顶层块，
 * 尾部渲染出 `' + s.体力 + '` 一类碎片。未闭合的 `<script>` 原样保留。
 */
export function withoutScripts(display) {
  return String(display || '').replace(/<script\b[^>]*>[\s\S]*?<\/script\s*>/gi, '')
}

/**
 * 展示文本 → 要渲染的 HTML 块（附文字版，浏览器半边净化失败时显示）。
 * @returns {Array<{html:string, text:string}>}
 */
export function renderableBlocks(display) {
  return splitRawHtmlBlocks(withoutScripts(display))
    .filter((segment) => segment.type === 'html')
    .map((segment) => ({ html: segment.content, text: readableText(segment.content) }))
}

/** 整条回复只有卡片界面时的正文（DSH 不收空回复）。 */
export const SHELL_ONLY_BODY = '（卡片界面，显示在这条回复下方。）'

/**
 * 展示文本 → DSH 正文与交给 Turn 尾部渲染的块数、卡片界面数：正文去掉卡片界面
 * （`.load(url)` 胶水、带脚本的内联文档，桌面 `segmentShellContent` 切分）与 HTML 块后
 * 转成可读文字。没有卡片界面时与只切块完全一致。
 * 整条回复全是块时退回整段文字版、块数记 0（正文不能为空：空回复在 DSH 历史里
 * 不算一轮，下一轮会错位；文字版已在正文里，尾部不再重复）。全是卡片界面时正文用
 * 一句占位（界面的脚本胶水没有可读文字）。
 * @returns {{body: string, blocks: number, shells: number}}
 */
export function splitForDsh(display) {
  const pieces = segmentShellContent(display)
  const shells = pieces.filter((piece) => piece.type === 'shell').length
  const text = shells === 0 ? String(display || '') : pieces.map((piece) => piece.content ?? '').filter(Boolean).join('\n\n')
  const segments = splitRawHtmlBlocks(withoutScripts(text))
  const blocks = segments.filter((segment) => segment.type === 'html').length
  if (blocks === 0 && shells === 0) return { body: readableText(display), blocks: 0, shells: 0 }
  const prose = segments
    .filter((segment) => segment.type === 'text')
    .map((segment) => segment.content)
    .join('\n\n')
  // 切走块后剩下的正文可能只剩卡壳占位（`<StatusPlaceHolderImpl/>`），照样按 HTML 去标签。
  const body = readableText(prose, { html: true })
  if (body) return { body, blocks, shells }
  if (shells === 0) return { body: readableText(display), blocks: 0, shells: 0 }
  return { body: SHELL_ONLY_BODY, blocks, shells }
}
