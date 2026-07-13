/**
 * Injectable persistence / degraded-helper adapters for plugin bridge surfaces.
 *
 * These replace the ambiguous degraded shims (saveChat / callGenericPopup /
 * getRequestHeaders) with explicit, deterministic, injectable contracts that
 * can be tested without editing `tauri-app` or relying on GUI/IPC.
 *
 * Design rules (PLAN: Persistence and Degraded Helpers):
 * - Every adapter returns an explicit { ok, degraded, cancelled, reason, ... }
 *   contract instead of a silent undefined / degraded promise.
 * - Adapters honor cancellation signals and never expose authorization,
 *   api keys, prompt bodies, or raw error stacks to plugins.
 * - The default adapters reproduce the existing StoryForge degraded behavior
 *   so ST boolean compatibility (`await saveChat() === true`) is preserved.
 */

export const PERSISTENCE_DEGRADED_REASON = 'local_mirror_only_no_host_persist'
const POPUP_DEGRADED_REASON = 'no_ui_returns_default_or_null'
const HEADERS_DEGRADED_REASON = 'static_json_content_type_only'

const POPUP_TYPES = Object.freeze({
  TEXT: 1,
  CONFIRM: 2,
  INPUT: 3,
  DISPLAY: 4,
  CROP: 5,
})

// Keys that must never be echoed back to a plugin, even from a custom adapter.
const FORBIDDEN_HEADER_KEYS = new Set([
  'authorization',
  'apikey',
  'api-key',
  'x-api-key',
  'api_key',
  'bearer',
  'cookie',
  'set-cookie',
  'x-auth-token',
])

function isCancelled(signal) {
  return Boolean(signal?.aborted)
}

function makeCancelledResult(reason = 'cancelled') {
  return { ok: false, cancelled: true, degraded: false, reason, persistedAt: null }
}

function makeDegradedFallback(reason, detail) {
  return { ok: false, cancelled: false, degraded: true, reason, detail: detail || null, persistedAt: null }
}

/**
 * Build a deterministic saveChat persistence adapter.
 *
 * @param {object} [options]
 * @param {(snapshot)=>({ok:boolean,persistedAt?:string}|Promise<{ok,persistedAt}>)} [options.persist]
 *        When omitted the adapter is degraded (local-mirror-only) and resolves
 *        to the ST-compatible truthy boolean.
 * @param {(snapshot)=>unknown} [options.chatSnapshot]
 *        Normalizes the snapshot before persistence. Defaults to identity.
 * @returns {{saveChat:(payload, options?)=>Promise<object>}}
 */
export function createSaveChatAdapter(options = {}) {
  const persist = typeof options.persist === 'function' ? options.persist : null
  const chatSnapshot = typeof options.chatSnapshot === 'function' ? options.chatSnapshot : (snapshot) => snapshot

  async function saveChat(payload = {}, adapterOptions = {}) {
    if (isCancelled(adapterOptions.signal)) return makeCancelledResult()

    const snapshot = chatSnapshot(payload)
    if (!persist) {
      // Degraded default: no host persist path. Keep ST boolean compatibility.
      return { ok: true, degraded: true, reason: PERSISTENCE_DEGRADED_REASON, cancelled: false, persistedAt: null }
    }
    try {
      const result = await persist(snapshot)
      if (isCancelled(adapterOptions.signal)) return makeCancelledResult()
      if (result && result.ok) {
        return {
          ok: true,
          degraded: false,
          cancelled: false,
          reason: null,
          persistedAt: result.persistedAt ?? null,
        }
      }
      return makeDegradedFallback(
        result?.reason || 'persist_rejected',
        result?.detail || null,
      )
    } catch {
      // Never surface raw error text; callers may forward into audit/plugin space.
      return makeDegradedFallback('persist_failed', null)
    }
  }

  return { saveChat }
}

/**
 * Default saveChat adapter: degraded local-mirror-only, ST-compatible.
 */
export function createDefaultSaveChatAdapter() {
  return createSaveChatAdapter({})
}

/**
 * Turn a saveChat result into a ST-compatible promise that:
 *   - resolves truthy (`true`) so `await saveChat() === true`
 *   - carries a visible `.degraded` / `.reason` / `.ok` marker
 *
 * Mirrors the historical contract used by the iframe bridge.
 */
export function classifySaveChatResult(result) {
  const ok = result?.ok !== false
  const pending = Promise.resolve(ok ? true : false)
  pending.ok = ok
  pending.degraded = Boolean(result?.degraded)
  pending.reason = result?.reason || null
  pending.cancelled = Boolean(result?.cancelled)
  pending.persistedAt = result?.persistedAt ?? null
  return pending
}

/**
 * Build a popup (callGenericPopup) adapter.
 *
 * @param {object} [options]
 * @param {(request, pluginContext)=>boolean} [options.isAuthorized]
 *        When provided, unauthorized callers receive `undefined` (explicit
 *        unsupported) instead of a degraded default.
 * @param {(request)=>(Promise<unknown>|unknown)} [options.render]
 *        Custom render; receives { type, html, defaultValue }.
 * @returns {{popup:(request, pluginContext?)=>Promise<unknown>}}
 */
export function createPopupAdapter(options = {}) {
  const isAuthorized = typeof options.isAuthorized === 'function' ? options.isAuthorized : null
  const render = typeof options.render === 'function' ? options.render : null

  async function popup(request = {}, pluginContext = null) {
    if (isAuthorized && !isAuthorized(pluginContext)) {
      // Explicit unsupported — do NOT pretend the user confirmed.
      return undefined
    }
    if (render) {
      try {
        return await render(request)
      } catch {
        return undefined
      }
    }
    return degradedPopupDefault(request)
  }

  return { popup }
}

export function createDefaultPopupAdapter() {
  return createPopupAdapter({})
}

function degradedPopupDefault(request = {}) {
  const type = request.type
  if (request.defaultValue !== undefined) return Promise.resolve(String(request.defaultValue))
  if (type === POPUP_TYPES.CONFIRM || String(type || '').toLowerCase() === 'confirm') {
    return Promise.resolve(null)
  }
  return Promise.resolve('')
}

/**
 * Build a getRequestHeaders adapter that redacts credential-bearing headers.
 *
 * @param {object} [options]
 * @param {()=>object} [options.getHeaders]
 *        Custom header source. Redaction is always applied on top.
 * @returns {{getHeaders:()=>object}}
 */
export function createRequestHeadersAdapter(options = {}) {
  const getHeaders = typeof options.getHeaders === 'function' ? options.getHeaders : null

  function resolve() {
    let raw = {}
    if (getHeaders) {
      try {
        raw = getHeaders() || {}
      } catch {
        raw = {}
      }
    } else {
      raw = { 'Content-Type': 'application/json' }
    }
    return redactHeaders(raw)
  }

  return { getHeaders: resolve, degradedReason: getHeaders ? null : HEADERS_DEGRADED_REASON }
}

export function createDefaultRequestHeadersAdapter() {
  return createRequestHeadersAdapter({})
}

function redactHeaders(headers) {
  const out = {}
  for (const [key, value] of Object.entries(headers || {})) {
    const normalized = String(key || '').toLowerCase()
    if (FORBIDDEN_HEADER_KEYS.has(normalized)) continue
    if (FORBIDDEN_HEADER_KEYS.has(String(normalized).replace(/[^a-z0-9]/g, ''))) continue
    if (typeof value === 'string' && /bearer\s|sk-|SF_SECRET_|api[_-]?key|authorization/i.test(value)) {
      continue
    }
    out[key] = value
  }
  if (!out['Content-Type']) out['Content-Type'] = 'application/json'
  return out
}

/**
 * Merge partial adapter overrides on top of the degraded defaults.
 * Missing surfaces fall back to defaults so the host always has a usable set.
 *
 * @param {{saveChat?:ReturnType<typeof createSaveChatAdapter>, popup?:ReturnType<typeof createPopupAdapter>, requestHeaders?:ReturnType<typeof createRequestHeadersAdapter>}} [overrides]
 */
export function applyPersistenceAdapters(overrides = {}) {
  return {
    saveChat: overrides.saveChat || createDefaultSaveChatAdapter(),
    popup: overrides.popup || createDefaultPopupAdapter(),
    requestHeaders: overrides.requestHeaders || createDefaultRequestHeadersAdapter(),
  }
}
