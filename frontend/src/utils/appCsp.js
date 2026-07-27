/**
 * Main-application Content-Security-Policy for StoryForge.
 *
 * Scope: this is the *outer* app CSP (`crates/tauri-app/tauri.conf.json`
 * `app.security.csp`). It bounds what the trusted StoryForge UI origin may do.
 *
 * ⚠ SECURITY MODEL — how the shell iframe relates to this policy.
 *
 * CSP Level 3 (https://w3c.github.io/webappsec-csp/#security-inherit-csp):
 * a document created from `blob:` / `srcdoc:` / `data:` / `about:blank`
 * INHERITS its creator's policy container. When multiple CSPs apply (an
 * inherited one PLUS one declared in the document via `<meta>` or an HTTP
 * header), they only ever INTERSECT — a later `<meta>` can TIGHTEN but never
 * WIDEN an inherited policy.
 *
 * Consequence for StoryForge: every card/plugin/MVU runtime embeds an inline
 * `<script>` bridge in its iframe document. If that document inherited a main
 * app CSP that forbids inline scripts, the bridge would be blocked — and the
 * shell's own `buildShellCspMetaTag` could not rescue it. We therefore serve
 * shell documents from a SEPARATE origin (the restricted `storyforge-shell`
 * Tauri custom protocol; see `shellDocUrl.js` + `shell_doc_protocol.rs`) whose
 * policy container does NOT inherit this main app CSP. That origin carries its
 * own shell CSP. The main app's `frame-src` below allows ONLY that origin.
 *
 * The earlier comment claiming the shell is "never bounded by the outer
 * policy" was WRONG for blob/srcdoc documents and has been removed.
 *
 * Verified resource model for the main app (full inventory in
 * docs/workstreams/SECURITY-APP-CSP-RESULT-2026-07-27.md):
 *   - App bundle from 'self' (Vite dev origin http://localhost:1420, production
 *     Tauri custom-protocol origin).
 *   - ALL host I/O (LLM, files, imports, secrets) goes through Tauri `invoke`
 *     (frontend/src/tauri-api.js) — i.e. the Rust host process, not the WebView.
 *     The main UI has ZERO direct network egress by design.
 *   - Remote CDNs (jQuery/Vue/zod/Ejs/lodash in CardShellHost.vue /
 *     TavernHelperRuntime.vue) are fetched host-mediated
 *     (`__sfHostFetchText`/`__sfThHostFetchText` -> `ask('fetch_text')`
 *     postMessage -> Rust card_shell_cache.rs with SSRF+allowlist), then
 *     injected as inline scripts INSIDE the isolated shell document. They are
 *     bounded by the shell origin's own CSP, never by this outer policy.
 *
 * Tauri 2 behavior: the developer must declare ipc:/asset:/custom-protocol
 * sources explicitly; Tauri appends nonces/hashes for bundled code at build
 * time. connect-src needs `ipc: http://ipc.localhost`.
 *
 * DRIFT GUARD: this constant is a byte-for-byte duplicate of the `csp` value
 * in crates/tauri-app/tauri.conf.json. It is NOT generated at build time, so
 * it is not a true single source of truth — it is a drift guard. The test
 * `app-csp.test.mjs` asserts equality and fails the build if the two drift.
 */

// Shell-document origin (isolated custom protocol). Mirrors
// shell_doc_protocol.rs SHELL_DOC_ORIGIN and shellDocUrl.js SHELL_DOC_ORIGIN.
export const APP_SHELL_DOC_ORIGINS = [
  'http://storyforge-shell.localhost',
  'storyforge-shell://localhost',
]

// Cache protocol origins — mirror card_shell_cache.rs platform split and
// cardShellCsp.js SHELL_CACHE_ORIGINS. Large cached assets are served here.
export const APP_CACHE_ORIGINS = [
  'http://storyforge-cache.localhost',
  'storyforge-cache://localhost',
]

/**
 * The main-application CSP. This MUST stay identical to the value configured in
 * crates/tauri-app/tauri.conf.json -> app.security.csp (drift-guarded by
 * app-csp.test.mjs).
 *
 * Directive rationale (each non-'self' source documented with its consumer):
 *   default-src 'self'        — baseline; app bundle only.
 *   connect-src ipc: http://ipc.localhost data: blob: <cache origins>
 *                             — Tauri IPC (mandatory; frontend/src/tauri-api.js),
 *                               data:/blob: for local in-memory URLs,
 *                               storyforge-cache for large cached assets served
 *                               by the custom protocol. NO remote https/http
 *                               host: the app originates no network calls.
 *   img-src 'self' data: blob: <cache origins>
 *                             — local + cache-protocol + in-memory images.
 *   media-src 'self' data: blob: <cache origins>
 *                             — local + cache-protocol + in-memory media.
 *   font-src 'self' data: blob: <cache origins>
 *                             — app fonts are bundled ('self'); data:/blob:/
 *                               cache for parity with img/media when the shell
 *                               surfaces a cached font.
 *   style-src 'self' 'unsafe-inline'
 *                             — Vue runtime injects scoped style; index.html
 *                               theme bootstrap is inline. Scripts are NOT
 *                               given 'unsafe-inline' — only styles.
 *   frame-src <shell origins>
 *                             — ONLY the isolated storyforge-shell origin may
 *                               be framed (CardShellHost/TavernHelperRuntime/
 *                               MvuJsRuntime/PluginHost all load shell docs
 *                               there). data:/blob: are intentionally NOT
 *                               granted because there is no production
 *                               consumer and parent-created inline documents
 *                               inherit this app CSP.
 *   object-src 'none'         — no plugins/Flash/Java.
 *   form-action 'none'        — no form submission from the app origin.
 *   base-uri 'none'           — forbid <base> hijack of relative URLs.
 *
 * Deliberately NOT granted (and tested absent):
 *   - Any `https://` or `http://` remote host (the two *.localhost forms are
 *     local registered protocols, not network).
 *   - `*`.
 *   - `script-src 'unsafe-inline'` / `'unsafe-eval'` (no WASM, no eval need;
 *     package.json has no wasm/onnx/sqlite dep; dist/ emits no .wasm). Tauri
 *     build-time hashing covers index.html's inline theme bootstrap script.
 *   - `worker-src blob:` (no consumer; removed). worker-src is omitted entirely
 *     and falls back to default-src 'self'.
 */
export const APP_CSP = [
  "default-src 'self'",
  `connect-src ipc: http://ipc.localhost data: blob: ${APP_CACHE_ORIGINS.join(' ')}`,
  `img-src 'self' data: blob: ${APP_CACHE_ORIGINS.join(' ')}`,
  `media-src 'self' data: blob: ${APP_CACHE_ORIGINS.join(' ')}`,
  `font-src 'self' data: blob: ${APP_CACHE_ORIGINS.join(' ')}`,
  "style-src 'self' 'unsafe-inline'",
  `frame-src ${APP_SHELL_DOC_ORIGINS.join(' ')}`,
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
