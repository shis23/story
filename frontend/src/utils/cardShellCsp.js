/**
 * Content-Security-Policy for wrapped card-shell documents.
 *
 * The shell iframe is a blob: document with tauri csp:null, so without its own
 * CSP every side channel (<img src>, <script src>, XHR, CSS url()) can reach
 * arbitrary hosts and exfiltrate the opening seeds / variable buckets embedded
 * in the wrapper. This policy pins all network-capable directives to the same
 * host allowlist the host-mediated fetch proxy enforces (card_shell_cache.rs),
 * closing the bypass while keeping data:/blob: needed by the module runner and
 * the local cache protocol used for large cached assets.
 */

// Keep in sync with card_shell_cache.rs LOCAL_PROTOCOL_ORIGIN (Windows/Android
// use the http localhost mapping, macOS/Linux the native scheme).
export const SHELL_CACHE_ORIGINS = [
  'http://storyforge-cache.localhost',
  'storyforge-cache://localhost',
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
  const join = (sources) => sources.filter(Boolean).join(' ')
  const directives = [
    "default-src 'none'",
    // Inline bridge scripts + blob: module graphs. Card JS may eval.
    `script-src ${join(["'unsafe-inline'", "'unsafe-eval'", 'blob:', 'data:', ...net])}`,
    `style-src ${join(["'unsafe-inline'", 'blob:', 'data:', ...net])}`,
    `img-src ${join(['data:', 'blob:', ...cache, ...net])}`,
    `font-src ${join(['data:', 'blob:', ...cache, ...net])}`,
    `media-src ${join(['data:', 'blob:', ...cache, ...net])}`,
    // fetch() is proxied over postMessage (no network); this bounds direct XHR.
    `connect-src ${join(['data:', 'blob:', ...cache, ...net])}`,
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
