/**
 * Register an inline shell document on the isolated `storyforge-shell`
 * origin and return the iframe-loadable URL.
 *
 * # Why (V5 CSP isolation)
 *
 * CardShellHost / TavernHelperRuntime (blob iframes) and MvuJsRuntime /
 * PluginHost (srcdoc iframes) all embed inline `<script>` bridges. CSP L3
 * makes a blob/srcdoc/data/about:blank document INHERIT its creator's policy
 * container, and multiple CSPs only intersect — so a strict main-app CSP
 * (no script-src 'unsafe-inline') breaks every inline bridge, and the
 * shell's own `<meta>` cannot widen it. We therefore serve the shell document
 * from a SEPARATE origin (the restricted `storyforge-shell` Tauri custom
 * protocol) whose policy container does NOT inherit the main app CSP. That
 * origin carries its own shell CSP (HTTP header set in
 * crates/tauri-app/src/shell_doc_protocol.rs, mirrored as a `<meta>` injected
 * by buildShellCspMetaTag). The main app's frame-src allows only this origin.
 *
 * Mirrors shell_doc_protocol.rs SHELL_DOC_ORIGIN platform split and the
 * card_shell_cache.rs origin used by cardShellCsp.js SHELL_CACHE_ORIGINS.
 */

// On Windows/Android Tauri/Wry exposes the registered custom protocol under an
// http localhost origin; macOS/Linux keep the native scheme origin. Mirror of
// shell_doc_protocol.rs SHELL_DOC_ORIGIN.
const IS_WIN_OR_ANDROID = (() => {
  if (typeof navigator !== 'undefined' && navigator.userAgent) {
    return /Win/.test(navigator.userAgent) || /Android/.test(navigator.userAgent)
  }
  if (typeof process !== 'undefined' && process.platform) {
    return process.platform === 'win32' || process.platform === 'android'
  }
  return true
})()

export const SHELL_DOC_ORIGIN = IS_WIN_OR_ANDROID
  ? 'http://storyforge-shell.localhost'
  : 'storyforge-shell://localhost'

let _invoke = null
const TOKEN_RE = /^[a-f0-9]{64}$/

/**
 * Inject the Tauri `invoke` dependency. Avoids a hard import here so the module
 * stays unit-testable without @tauri-apps/api present. CardShellHost et al.
 * call this once at module init with the same invoke they already use.
 */
export function configureShellDocInvoke(invoke) {
  _invoke = invoke
}

/**
 * Register a shell document and return its URL on the isolated origin.
 *
 * @param {string} html full document HTML (must already include its own
 *   `<meta http-equiv="Content-Security-Policy">` from buildShellCspMetaTag)
 * @returns {Promise<string>} iframe `src` URL, e.g.
 *   `http://storyforge-shell.localhost/<token>`
 */
export async function registerShellDoc(html) {
  if (!_invoke) {
    throw new Error('registerShellDoc: Tauri invoke not configured (call configureShellDocInvoke)')
  }
  const token = await _invoke('card_shell_register_doc', { html })
  if (typeof token !== 'string' || !TOKEN_RE.test(token)) {
    throw new Error('registerShellDoc: invalid shell document token returned by host')
  }
  return `${SHELL_DOC_ORIGIN}/${token}`
}

export function shellDocTokenFromUrl(url) {
  if (typeof url !== 'string') return null
  const prefix = `${SHELL_DOC_ORIGIN}/`
  if (!url.startsWith(prefix)) return null
  const token = url.slice(prefix.length)
  return TOKEN_RE.test(token) ? token : null
}

/**
 * Release a registered document that was replaced/unmounted before its first
 * GET consumed the one-shot host entry. Invalid or non-shell URLs are ignored.
 */
export async function releaseShellDoc(url) {
  const token = shellDocTokenFromUrl(url)
  if (!_invoke || !token) return false
  return Boolean(await _invoke('card_shell_unregister_doc', { token }))
}

/**
 * Test/fixture helper: build a URL for a pre-known token without IPC. Used by
 * the Playwright CSP-behavior tests (which mock the protocol via page.route,
 * not via Tauri) and by Node unit tests that only inspect URL shape.
 */
export function shellDocUrlForToken(token) {
  if (!TOKEN_RE.test(String(token || ''))) {
    throw new Error('shellDocUrlForToken: invalid shell document token')
  }
  return `${SHELL_DOC_ORIGIN}/${token}`
}
