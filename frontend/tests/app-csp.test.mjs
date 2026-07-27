import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import url from 'node:url'

import { APP_CSP, APP_CSP_DIRECTIVES, APP_CACHE_ORIGINS, parseCsp } from '../src/utils/appCsp.js'

const __dirname = path.dirname(url.fileURLToPath(import.meta.url))
// tests/ -> frontend/ -> repo root.
const REPO_ROOT = path.resolve(__dirname, '..', '..')
const CONF_PATH = path.join(REPO_ROOT, 'crates', 'tauri-app', 'tauri.conf.json')

// ---------------------------------------------------------------------------
// Static contract: the main-app CSP must be a tight, auditable boundary.
// ---------------------------------------------------------------------------

test('default-src is self only', () => {
  assert.deepEqual(APP_CSP_DIRECTIVES['default-src'], ["'self'"])
})

test('connect-src allows Tauri IPC + local memory + cache protocol, nothing remote', () => {
  const cs = APP_CSP_DIRECTIVES['connect-src']
  assert.ok(cs.includes('ipc:'), 'connect-src must allow Tauri IPC')
  assert.ok(cs.includes('http://ipc.localhost'), 'connect-src must allow Tauri IPC localhost')
  assert.ok(cs.includes('data:'), 'connect-src must allow data:')
  assert.ok(cs.includes('blob:'), 'connect-src must allow blob:')
  // Every cache-protocol origin must be present (both platform forms).
  for (const origin of APP_CACHE_ORIGINS) {
    assert.ok(cs.includes(origin), `connect-src must include ${origin}`)
  }
})

test('NO directive grants arbitrary remote egress (the V5 regression guard)', () => {
  // This is the headline failing-first assertion for V5: with csp:null there
  // is no policy at all. Once a policy exists, it must never contain wildcard
  // schemes, public http(s) hosts, unsafe-eval, or script-src unsafe-inline.
  const forbidden = ['*', 'https:', 'http:', "'unsafe-eval'", "'unsafe-inline'"]
  for (const [name, sources] of Object.entries(APP_CSP_DIRECTIVES)) {
    for (const bad of forbidden) {
      if (bad === "'unsafe-inline'") {
        // style-src is the ONLY directive allowed to keep 'unsafe-inline'
        // (Vue scoped CSS + theme bootstrap). script-src must NOT.
        if (name === 'style-src') continue
        assert.ok(
          !sources.includes(bad),
          `${name} must not include 'unsafe-inline'`,
        )
        continue
      }
      assert.ok(!sources.includes(bad), `${name} must not include ${bad}`)
    }
  }
  // No public-network hostname may sneak into any directive. localhost is
  // allowed only as the Tauri IPC / cache-protocol forms.
  for (const sources of Object.values(APP_CSP_DIRECTIVES)) {
    for (const src of sources) {
      if (/^https?:\/\//i.test(src)) {
        const host = src.replace(/^https?:\/\//i, '').split('/')[0]
        assert.ok(
          host === 'ipc.localhost' || host === 'storyforge-cache.localhost',
          `unexpected network host in CSP: ${src}`,
        )
      }
    }
  }
})

test('script-src does not grant unsafe-inline or unsafe-eval', () => {
  // The main app has no inline-script requirement; Tauri injects nonces/hashes
  // for bundled code at build time. If a future change needs inline scripts it
  // must be argued per-consumer, not relaxed silently.
  const ss = APP_CSP_DIRECTIVES['script-src']
  if (ss) {
    assert.ok(!ss.includes("'unsafe-inline'"))
    assert.ok(!ss.includes("'unsafe-eval'"))
  } else {
    // script-src falls back to default-src 'self' — also fine.
    assert.deepEqual(APP_CSP_DIRECTIVES['default-src'], ["'self'"])
  }
})

test('object-src, form-action and base-uri are locked down', () => {
  assert.deepEqual(APP_CSP_DIRECTIVES['object-src'], ["'none'"])
  assert.deepEqual(APP_CSP_DIRECTIVES['form-action'], ["'none'"])
  assert.deepEqual(APP_CSP_DIRECTIVES['base-uri'], ["'none'"])
})

test('frame-src allows blob: for the shell iframe and data:', () => {
  // CardShellHost.vue creates a blob: document for the sandboxed card shell.
  // Without frame-src blob: the shell cannot load at all.
  const fs = APP_CSP_DIRECTIVES['frame-src']
  assert.ok(fs.includes('blob:'))
  assert.ok(fs.includes('data:'))
})

test('cache-protocol origins mirror card_shell_cache.rs + cardShellCsp.js', () => {
  // Both platform forms must be present so the same policy works on every OS.
  assert.deepEqual(APP_CACHE_ORIGINS.sort(), [
    'http://storyforge-cache.localhost',
    'storyforge-cache://localhost',
  ].sort())
})

test('parseCsp round-trips the policy without dropping directives', () => {
  const reparsed = parseCsp(APP_CSP)
  assert.deepEqual(reparsed, APP_CSP_DIRECTIVES)
})

// ---------------------------------------------------------------------------
// Config drift guard: the constant above MUST equal the live tauri.conf.json
// value. Catches the classic "policy documented, config still null" drift.
// This assertion fails until tauri.conf.json is updated (fail-first).
// ---------------------------------------------------------------------------

test('tauri.conf.json app.security.csp equals APP_CSP byte-for-byte', () => {
  const conf = JSON.parse(fs.readFileSync(CONF_PATH, 'utf8'))
  const configured = conf?.app?.security?.csp
  assert.equal(
    configured,
    APP_CSP,
    `tauri.conf.json app.security.csp must equal APP_CSP. Got: ${JSON.stringify(configured)}`,
  )
  assert.notEqual(configured, null, 'app.security.csp must not be null (V5)')
})

// ---------------------------------------------------------------------------
// Shell compatibility regression: the outer policy must not narrow channels
// the shell iframe relies on. (The shell has its own meta CSP, but blob iframe
// loading and the cache protocol still need to pass the OUTER policy because
// the shell document is created by the trusted parent origin.)
// ---------------------------------------------------------------------------

test('shell iframe channels survive the outer policy', () => {
  // Parent creates `new Blob([html])` + createObjectURL, then loads it in a
  // sandbox=allow-scripts iframe. frame-src must permit blob:.
  assert.ok(APP_CSP_DIRECTIVES['frame-src'].includes('blob:'))
  // Large cached assets are addressed as storyforge-cache URLs and may be
  // fetched/img'd by both the app and the shell.
  for (const origin of APP_CACHE_ORIGINS) {
    assert.ok(APP_CSP_DIRECTIVES['connect-src'].includes(origin))
    assert.ok(APP_CSP_DIRECTIVES['img-src'].includes(origin))
  }
})
