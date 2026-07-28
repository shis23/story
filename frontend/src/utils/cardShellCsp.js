/**
 * Content-Security-Policy for wrapped card-shell documents.
 *
 * The shell iframe is served from an isolated custom-protocol origin. Without
 * its own CSP every side channel (<img src>, <script src>, XHR, CSS url()) can
 * reach arbitrary hosts and exfiltrate the opening seeds / variable buckets
 * embedded in the wrapper. This policy pins network-capable directives to the
 * same host allowlist the host-mediated fetch proxy enforces. JavaScript module
 * source is served through lease-scoped tokens on the restricted shell origin;
 * data:/blob: remain available only for local card mechanics.
 */

// Keep in sync with card_shell_cache.rs LOCAL_PROTOCOL_ORIGIN (Windows/Android
// use the http localhost mapping, macOS/Linux the native scheme).
export const SHELL_CACHE_ORIGINS = [
  'http://storyforge-cache.localhost',
  'storyforge-cache://localhost',
]

export const SHELL_MODULE_ORIGINS = [
  'http://storyforge-shell.localhost',
  'storyforge-shell://localhost',
]

/** Hostnames only — reject entries that could smuggle schemes/paths into CSP. */
function sanitizeHosts(allowedHosts) {
  const out = []
  const seen = new Set()
  for (const raw of Array.isArray(allowedHosts) ? allowedHosts : []) {
    const host = String(raw || '').trim().toLowerCase()
    if (!host || !/^[a-z0-9][a-z0-9.-]*$/.test(host) || seen.has(host)) continue
    seen.add(host)
    out.push(`https://${host}`)
  }
  return out
}

/**
 * @param {string[] | null | undefined} allowedHosts hostnames from
 *   card_shell_list_allowed_hosts (user-extended allowlist included)
 * @returns {string} CSP header value
 */
export function buildShellCspContent(allowedHosts = []) {
  const net = sanitizeHosts(allowedHosts)
  const cache = SHELL_CACHE_ORIGINS
  const modules = SHELL_MODULE_ORIGINS
  const join = (sources) => sources.filter(Boolean).join(' ')
  const directives = [
    "default-src 'none'",
    // Inline bridge scripts + lease-scoped restricted-origin modules. Card JS may eval.
    `script-src ${join(["'unsafe-inline'", "'unsafe-eval'", 'blob:', 'data:', ...modules, ...net])}`,
    `style-src ${join(["'unsafe-inline'", 'blob:', 'data:', ...net])}`,
    `img-src ${join(['data:', 'blob:', ...cache, ...net])}`,
    `font-src ${join(['data:', 'blob:', ...cache, ...net])}`,
    `media-src ${join(['data:', 'blob:', ...cache, ...net])}`,
    // WebView2 evaluates custom-protocol import() against connect-src too.
    // The shell origin exposes only bounded, lease-scoped opaque-token resources.
    `connect-src ${join(['data:', 'blob:', ...cache, ...modules, ...net])}`,
    'frame-src blob: data:',
    'worker-src blob:',
    'child-src blob:',
    "object-src 'none'",
    "form-action 'none'",
  ]
  return directives.join('; ')
}

/** `<meta http-equiv>` tag ready for head injection. */
export function buildShellCspMetaTag(allowedHosts = []) {
  const content = buildShellCspContent(allowedHosts).replace(/"/g, '&quot;')
  return `<meta http-equiv="Content-Security-Policy" content="${content}">`
}
