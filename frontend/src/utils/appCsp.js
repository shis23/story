/**
 * Main-application Content-Security-Policy for StoryForge.
 *
 * Scope: this is the *outer* app CSP (`crates/tauri-app/tauri.conf.json`
 * `app.security.csp`). It bounds what the trusted StoryForge UI origin may do —
 * NOT the card-shell iframe, which ships its own stricter policy
 * (`cardShellCsp.js`, injected as a `<meta>` in `wrapRemoteHtml`).
 *
 * Verified resource model for the main app (full inventory in
 * docs/workstreams/SECURITY-APP-CSP-RESULT-2026-07-27.md):
 *   - App bundle served from 'self' (Vite dev origin http://localhost:1420,
 *     production Tauri custom-protocol origin).
 *   - ALL host I/O (LLM, files, imports, secrets) goes through Tauri `invoke`
 *     (frontend/src/tauri-api.js) — i.e. the Rust host process, not the WebView.
 *     The main UI has ZERO direct network egress by design.
 *   - Remote CDNs (jQuery/Vue/zod/Ejs/lodash in CardShellHost.vue /
 *     TavernHelperRuntime.vue) are fetched host-mediated
 *     (`__sfHostFetchText`/`__sfThHostFetchText` -> `ask('fetch_text')`
 *     postMessage -> Rust card_shell_cache.rs with SSRF+allowlist), then
 *     injected as inline scripts INSIDE the shell iframe. They are bounded by
 *     the shell CSP, never by this outer policy.
 *   - The shell iframe is loaded from a parent-created blob: URL, so it needs
 *     frame-src blob:. Large cached assets are served via the registered
 *     storyforge-cache custom protocol (http://storyforge-cache.localhost on
 *     Windows/Android, storyforge-cache://localhost on macOS/Linux — kept in
 *     sync with card_shell_cache.rs:27-29 and cardShellCsp.js SHELL_CACHE_ORIGINS).
 *
 * Tauri 2 behavior: the developer must declare ipc:/asset:/custom-protocol
 * sources explicitly; Tauri appends nonces/hashes for bundled code at build
 * time. connect-src needs `ipc: http://ipc.localhost`.
 *
 * Keep this string byte-for-byte identical to the `csp` value in
 * crates/tauri-app/tauri.conf.json (enforced by app-csp.test.mjs).
 */

// Cache protocol origins — mirror card_shell_cache.rs platform split and
// cardShellCsp.js SHELL_CACHE_ORIGINS. Listed in BOTH platform forms so the
// static policy works regardless of which platform the bundle is built for.
export const APP_CACHE_ORIGINS = [
  'http://storyforge-cache.localhost',
  'storyforge-cache://localhost',
]

/**
 * The main-application CSP. This MUST stay identical to the value configured in
 * crates/tauri-app/tauri.conf.json -> app.security.csp.
 *
 * Directive rationale (each non-'self' source documented):
 *   default-src 'self'        — baseline; app bundle only.
 *   connect-src ipc: http://ipc.localhost data: blob: <cache origins>
 *                             — Tauri IPC (mandatory), data:/blob: for local
 *                               in-memory URLs, storyforge-cache for large
 *                               cached assets served by the custom protocol.
 *                               NO remote https/http host: the app does not
 *                               originate network calls.
 *   img-src 'self' data: blob: <cache origins>
 *                             — local images + cache-protocol images + in-memory.
 *   media-src 'self' data: blob: <cache origins>
 *   font-src 'self' data: blob: <cache origins>
 *   style-src 'self' 'unsafe-inline'
 *                             — Vue runtime injects scoped style + the index
 *                               inline theme bootstrap need inline styles.
 *                               Scripts are NOT given 'unsafe-inline' — only
 *                               styles, and only because Vue scoped CSS + the
 *                               theme bootstrap require it.
 *   frame-src blob: data:     — the shell iframe is loaded from a parent-created
 *                               blob: document; data: kept for parity.
 *   worker-src 'self' blob:   — app workers (if any) are bundled or blob-based.
 *   object-src 'none'         — no plugins/Flash/Java.
 *   form-action 'none'        — no form submission from the app origin.
 *   base-uri 'none'           — forbid <base> hijack of relative URLs.
 *
 * Deliberately NOT granted (and tested absent):
 *   - Any `https://` or `http://` remote host, `*`, `unsafe-eval`,
 *     `unsafe-inline` under script-src. The app has no direct remote script
 *     load and no eval/WASM-eval requirement.
 */
export const APP_CSP = [
  "default-src 'self'",
  `connect-src ipc: http://ipc.localhost data: blob: ${APP_CACHE_ORIGINS.join(' ')}`,
  `img-src 'self' data: blob: ${APP_CACHE_ORIGINS.join(' ')}`,
  `media-src 'self' data: blob: ${APP_CACHE_ORIGINS.join(' ')}`,
  `font-src 'self' data: blob: ${APP_CACHE_ORIGINS.join(' ')}`,
  "style-src 'self' 'unsafe-inline'",
  'frame-src blob: data:',
  "worker-src 'self' blob:",
  "object-src 'none'",
  "form-action 'none'",
  "base-uri 'none'",
].join('; ')

/** Parse a CSP string into a { directive: sources[] } map (for tests/audit). */
export function parseCsp(csp) {
  const out = {}
  for (const raw of String(csp || '').split(';')) {
    const parts = raw.trim().split(/\s+/).filter(Boolean)
    if (!parts.length) continue
    const [name, ...sources] = parts
    // Last write wins matches browser semantics for duplicate directives.
    out[name] = sources
  }
  return out
}

/** Convenience for tests/audit: the directive map for APP_CSP. */
export const APP_CSP_DIRECTIVES = parseCsp(APP_CSP)
