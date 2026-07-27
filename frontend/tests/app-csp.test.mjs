import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import url from 'node:url'

import { APP_CSP, APP_CSP_DIRECTIVES, APP_CACHE_ORIGINS, APP_SHELL_DOC_ORIGINS, parseCsp } from '../src/utils/appCsp.js'

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
  // allowed only as the Tauri IPC / the two registered custom protocols.
  for (const sources of Object.values(APP_CSP_DIRECTIVES)) {
    for (const src of sources) {
      if (/^https?:\/\//i.test(src)) {
        const host = src.replace(/^https?:\/\//i, '').split('/')[0]
        assert.ok(
          host === 'ipc.localhost'
            || host === 'storyforge-cache.localhost'
            || host === 'storyforge-shell.localhost',
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

test('frame-src allows only the isolated shell origins', () => {
  // V5 CSP isolation: shell documents are served from the dedicated
  // storyforge-shell origin so they do NOT inherit the main app policy
  // container. blob: is intentionally absent — a parent-created blob document
  // would inherit this CSP and have its inline bridges blocked.
  const fs = APP_CSP_DIRECTIVES['frame-src']
  for (const origin of APP_SHELL_DOC_ORIGINS) {
    assert.ok(fs.includes(origin), `frame-src must include ${origin}`)
  }
  assert.equal(fs.length, APP_SHELL_DOC_ORIGINS.length)
  assert.ok(!fs.includes('data:'), 'no production data: iframe consumer exists')
  assert.ok(!fs.includes('blob:'), 'frame-src must NOT grant blob: (inheritance risk)')
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
// Shell compatibility regression: the outer policy must let the shell load via
// its ISOLATED origin and must NOT rely on blob: (which would inherit this
// CSP). The shell document's own policy container is set by the
// storyforge-shell protocol response (shell_doc_protocol.rs); it is not and
// cannot be widened by this outer policy.
// ---------------------------------------------------------------------------

test('shell iframe loads via the isolated origin, not blob:', () => {
  for (const origin of APP_SHELL_DOC_ORIGINS) {
    assert.ok(APP_CSP_DIRECTIVES['frame-src'].includes(origin))
  }
  assert.ok(!APP_CSP_DIRECTIVES['frame-src'].includes('blob:'))
  // Large cached assets are addressed as storyforge-cache URLs and are
  // fetch/img'd by both the app and the shell.
  for (const origin of APP_CACHE_ORIGINS) {
    assert.ok(APP_CSP_DIRECTIVES['connect-src'].includes(origin))
    assert.ok(APP_CSP_DIRECTIVES['img-src'].includes(origin))
  }
})

test('shell-doc origins mirror shell_doc_protocol.rs + shellDocUrl.js', () => {
  assert.deepEqual(APP_SHELL_DOC_ORIGINS.sort(), [
    'http://storyforge-shell.localhost',
    'storyforge-shell://localhost',
  ].sort())
})

test('no worker-src relaxation (no consumer)', () => {
  // worker-src is intentionally omitted; it falls back to default-src 'self'.
  // If a future worker need appears, add it per-consumer with a test.
  assert.ok(!('worker-src' in APP_CSP_DIRECTIVES))
})
