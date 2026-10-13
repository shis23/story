/** 面向用户的文字处理：会话标题、开场白显示、底层模型错误提示。 */

/**
 * DSH 会话标题上限 80 字节（UTF-8，`session-title.maxTitleBytes`），分叉时 DSH 在末尾追加
 * ` (1)` 再按字节截断——长标题会被截成「……还有个 (」。这里预留 8 字节，超长时自己
 * 截断并补省略号，分叉序号就能完整保留。
 */
export const SESSION_TITLE_BYTES = 72

export function fitSessionTitle(title, maxBytes = SESSION_TITLE_BYTES) {
  const text = String(title ?? '').trim()
  if (Buffer.byteLength(text, 'utf8') <= maxBytes) return text
  const budget = maxBytes - Buffer.byteLength('…', 'utf8')
  let used = 0
  let output = ''
  for (const character of text) {
    const bytes = Buffer.byteLength(character, 'utf8')
    if (used + bytes > budget) break
    output += character
    used += bytes
  }
  return `${output.trimEnd()}…`
}

const ENTITIES = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: '\u00a0' }

function decodeEntities(text) {
  return text.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (whole, name) => {
    if (name[0] === '#') {
      const code = name[1] === 'x' || name[1] === 'X' ? Number.parseInt(name.slice(2), 16) : Number(name.slice(1))
      return Number.isInteger(code) && code > 0 && code <= 0x10ffff ? String.fromCodePoint(code) : whole
    }
    return ENTITIES[name.toLowerCase()] ?? whole
  })
}

/** 开场白里出现这些标签才当 HTML 处理（只有 `<user>` 一类宏的纯文本原样返回）。 */
const HTML_HINT = /<\/?(?:div|p|br|span|ul|ol|li|h[1-6]|details|summary|table|tr|td|th|b|i|u|strong|em|font|center|img|hr|style|script|blockquote|pre|code|section)\b/i
const BLOCK_TAGS = 'p|div|ul|ol|h[1-6]|tr|table|details|summary|section|blockquote|pre|hr'
/** 表格单元格的占位（私用区字符，正文里不会出现），按行拼成 `- 甲 | 乙`。 */
const CELL = '\uE000'

/**
 * 卡的开场白常带状态栏 HTML（`<div>` / `<details>` / `<li>` 以及 `<StatusPlaceHolderImpl/>`
 * 一类卡壳占位），模型照着开场白每轮也会写一份（StoryForge 的质量门把它当卡的既定格式放行）。
 * DSH 的对话视图按 Markdown 渲染，这些标签会变成整屏代码块。这里转成可读文字：去注释、
 * 脚本与样式，块级标签换行，`<li>` 变成 `- ` 列表，表格每行变成 `- 甲 | 乙`（单元格不再
 * 粘连；用列表而不是裸行，Markdown 不会把相邻两行并成一段），其余标签去掉。保留 `<user>` / `<char>` /
 * `<bot>` 这类 ST 宏。存档里的原文不变，只影响显示；开场白与每轮回复共用。
 */
export function readableText(raw) {
  const source = typeof raw === 'string' ? raw : ''
  if (!HTML_HINT.test(source)) return source.trim()
  const text = source
    .replace(/\r\n?/g, '\n')
    .replace(/<!--[\s\S]*?-->/g, '')
    .replace(/<(script|style)\b[^>]*>[\s\S]*?<\/\1\s*>/gi, '')
    .replace(/<br\s*\/?>/gi, '\n')
    .replace(/<li\b[^>]*>/gi, '\n- ')
    .replace(/<t[dh]\b[^>]*>/gi, CELL)
    .replace(new RegExp(`</?(?:${BLOCK_TAGS}|li)\\b[^>]*>`, 'gi'), '\n')
    .replace(/<\/?(?!(?:user|char|bot)>)[A-Za-z][A-Za-z0-9-]*(?:\s[^>]*)?\/?>/g, '')
  // 行首缩进去掉（Markdown 里四个空格就成了代码块；全角空格「　」是正文缩进，保留）。
  const trim = (line) => line.replace(/^[ \t\u00a0]+|[ \t\u00a0]+$/g, '')
  const lines = decodeEntities(text)
    .split('\n')
    .map((line) => {
      if (!line.includes(CELL)) return trim(line)
      // \u8868\u683c\u884c\uff1a\u7a7a\u5355\u5143\u683c\uff08\u6392\u7248\u7528\uff09\u4e22\u6389\uff1b\u6574\u884c\u90fd\u7a7a\u5c31\u5f53\u7a7a\u884c\u3002
      const cells = line.split(CELL).map(trim).filter(Boolean)
      return cells.length ? `- ${cells.join(' | ')}` : ''
    })
  // 列表项之间的空行去掉，免得一条状态栏被拆成稀疏的松散列表。
  const kept = lines.filter((line, index) => {
    if (line) return true
    const previous = lines.slice(0, index).findLast((candidate) => candidate)
    const next = lines.slice(index + 1).find((candidate) => candidate)
    return !(previous?.startsWith('- ') && next?.startsWith('- '))
  })
  return kept.join('\n').replace(/\n{3,}/g, '\n\n').trim()
}

/** 已知的底层模型错误码 → 用户能照着做的提示。 */
const LLM_HINTS = [
  [/MISSING_CREDENTIAL/, '底层模型还没有配置 API Key。到 DSH「设置 → 模型」填写这个服务商的密钥，或在 StoryForge 首页「管线设置」换一个已配置的模型，再重试。'],
  [/INVALID_CREDENTIAL/, '底层模型的 API Key 无效。到 DSH「设置 → 模型」检查密钥后重试。'],
  [/ACCOUNT_QUOTA|insufficient[_ ]quota|quota exceeded/i, '底层模型账户额度不足。充值或换一个模型后重试。'],
  [/\b429\b|rate[_ ]limit/i, '底层模型限流（请求太频繁）。稍等一会儿再试。'],
  [/TRANSPORT|EMPTY_RESPONSE|Connection error|服务端错误 \(5xx\)|LLM error: 超时/i, '底层模型服务暂时连不上（网络中断、超时或服务端出错，自动重试后仍失败）。稍后重发这一轮即可，已写的故事不受影响。'],
  [/no_model|还没有选 StoryForge 的底层模型/, '还没有选 StoryForge 的底层模型：到左侧栏 StoryForge 首页「管线设置」选择后再试。'],
]

/**
 * 底层模型错误文字 → 一句可操作的提示；认不出时返回 null。
 * @param text - 原始错误（可能被管线包成 `LLM error: HTTP 请求失败: MISSING_CREDENTIAL: …`）。
 */
export function llmHint(text) {
  const message = String(text ?? '')
  for (const [pattern, hint] of LLM_HINTS) if (pattern.test(message)) return hint
  return null
}

/** 能认出的错误换成「提示 + 原始错误」，认不出的原样返回。 */
export function friendlyError(error) {
  const message = String(error?.message ?? error)
  const hint = llmHint(message)
  if (!hint || message.startsWith(hint)) return error instanceof Error ? error : new Error(message)
  const friendly = new Error(`${hint}\n（原始错误：${message}）`)
  if (error?.code !== undefined) friendly.code = error.code
  return friendly
}
