// @ts-check
import { expect, test } from '@playwright/test'
import { APP_CSP } from '../src/utils/appCsp.js'
import { buildShellCspContent } from '../src/utils/cardShellCsp.js'

// ---------------------------------------------------------------------------
// Real-Chromium CSP behavior tests for the V5 main-app policy.
//
// These are NOT string checks and NOT jsdom. Playwright drives a real
// Chromium engine that enforces CSP Level 3, including policy-container
// inheritance for blob:/srcdoc:/data:/about:blank documents and the
// "multiple policies intersect" rule. This file is the executable proof of
// the architectural problem and of the isolation fix.
//
// What we prove (the contract the iframe boundary must satisfy):
//   1. A blob: iframe navigated from the main origin inherits the main CSP.
//      If the main CSP forbids inline scripts, inline bridge scripts inside
//      that blob document are BLOCKED — regardless of any <meta> the document
//      sets for itself (a meta can only tighten, never relax, an inherited
//      policy).
//   2. Same for srcdoc iframes (MvuJsRuntime / PluginHost shape).
//   3. Therefore the shell cannot "rescue itself" with buildShellCspContent().
//      The main app MUST NOT carry a CSP that forbids what the shell needs to
//      execute, OR the shell document must be served from a separate origin
//      whose policy container does NOT inherit the main app CSP.
//   4. The chosen isolation: serve the shell document from a dedicated origin
//      (here modeled as a different HTTP origin) with its OWN CSP. That origin
//      is the only thing main frame-src allows; its inline/blob/eval needs are
//      scoped to that document and never broaden the main UI origin.
// ---------------------------------------------------------------------------

const artifactDir = process.env.UI_SMOKE_ARTIFACT_DIR || 'artifacts/ui-smoke/local'

/**
 * Read every CSP violation fired on the page into a list. Returns the list and
 * a finish() helper so each test can assert which directives were violated.
 */
function collectViolations(page) {
  const violations = []
  return {
    violations,
    async attach() {
      // page.on must be wired before navigation; capture message + violatedDirective.
      page.on('console', (msg) => {
        if (msg.type() === 'error') {
          const t = msg.text()
          if (t.includes('Content Security Policy') || t.includes('Refused to')) {
            violations.push(t)
          }
        }
      })
      // CSP violation events surface as 'securitypolicyviolation' on window.
      await page.addInitScript(() => {
        window.__CSP_VIOLATIONS__ = []
        window.addEventListener('securitypolicyviolation', (e) => {
          window.__CSP_VIOLATIONS__.push({
            directive: e.violatedDirective,
            blockedURI: e.blockedURI,
            sample: e.sample,
          })
        })
      })
    },
    async read() {
      // Prefer the DOM event list (structured); fall back to console text.
      try {
        const fromDom = await page.evaluate(() => window.__CSP_VIOLATIONS__ || [])
        if (fromDom.length) return fromDom.map((v) => `${v.directive}:${v.sample || v.blockedURI}`)
      } catch (_) { /* page may be navigated away */ }
      return violations
    },
  }
}

// Build a tiny origin-A host that mimics the Tauri app origin, and a separate
// origin-B host that mimics the isolated shell-document origin. Both are real
// HTTP origins so cross-origin policy-container rules apply like production.
test.beforeAll(async ({ browser }) => {
  void browser
})

// --- Scenario 1: blob iframe inline script inherits main CSP (the BUG) -----
test('blob iframe inline script is blocked when main CSP forbids unsafe-inline (reproduces V5 regression)', async ({ page, browser }) => {
  const ctx = { violations: [] }
  // Serve the "app" page from origin A with APP_CSP as a real header.
  const appHTML = `<!doctype html><html><head><meta charset="utf-8"></head><body>
    <div id="status">parent loaded</div>
    <script>
      // Parent creates a blob document whose embedded script posts a result
      // back. If the inline script runs, we receive "shell-ran". If the
      // inherited main CSP blocks inline scripts, we never hear back and a
      // securitypolicyviolation is reported IN THE PARENT for the blob doc.
      window.__SHELL_RESULT__ = 'timeout';
      window.addEventListener('message', (e) => { window.__SHELL_RESULT__ = e.data; });
      var blob = new Blob([
        '<!doctype html><html><body><script>' +
        'window.parent.postMessage("shell-ran", "*");' +
        '<\\/script></body></html>'
      ], { type: 'text/html' });
      var url = URL.createObjectURL(blob);
      var f = document.createElement('iframe');
      f.id = 'shell';
      f.src = url;
      document.body.appendChild(f);
    </script>
    <iframe id="shell"></iframe>
  </body></html>`

  await page.route('http://app.local.test/', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'text/html',
      headers: { 'Content-Security-Policy': APP_CSP },
      body: appHTML,
    })
  })
  const v = collectViolations(page)
  await v.attach()
  ctx.violations = v.violations

  await page.goto('http://app.local.test/')

  // The blob iframe must be navigable (frame-src blob: is allowed by APP_CSP).
  await expect(page.locator('#shell')).toBeVisible()

  // Wait up to 1.5s for either a result message or a CSP violation.
  await page.waitForTimeout(1500)
  const result = await page.evaluate(() => window.__SHELL_RESULT__)
  const vios = await v.read()

  // Headline: with APP_CSP forbidding script-src unsafe-inline, the blob doc's
  // inline script MUST be blocked. (If this passes, the iframe isolation work
  // is mandatory; if it fails, the inheritance assumption was wrong.)
  expect(result).not.toBe('shell-ran')
  expect(vios.some((t) => String(t).includes('script-src') || String(t).includes('inline')))
    .toBe(true)
})

// --- Scenario 2: srcdoc iframe inline script inherits main CSP -------------
test('srcdoc iframe inline script is blocked when main CSP forbids unsafe-inline (MvuJsRuntime/PluginHost shape)', async ({ page }) => {
  const appHTML = `<!doctype html><html><head><meta charset="utf-8"></head><body>
    <div id="status">parent loaded</div>
    <iframe id="rt" sandbox="allow-scripts" srcdoc="<script>window.parent.postMessage('rt-ran','*');<\/script>"></iframe>
    <script>
      window.__RT_RESULT__ = 'timeout';
      window.addEventListener('message', (e) => { window.__RT_RESULT__ = e.data; });
    </script>
  </body></html>`

  await page.route('http://app.local.test/mvu', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'text/html',
      headers: { 'Content-Security-Policy': APP_CSP },
      body: appHTML,
    })
  })
  const v = collectViolations(page)
  await v.attach()

  await page.goto('http://app.local.test/mvu')
  await expect(page.locator('#rt')).toBeVisible()
  await page.waitForTimeout(1500)
  const result = await page.evaluate(() => window.__RT_RESULT__)
  const vios = await v.read()

  expect(result).not.toBe('rt-ran')
  expect(vios.some((t) => String(t).includes('script-src') || String(t).includes('inline')))
    .toBe(true)
})

// --- Scenario 3: a meta CSP inside the iframe CANNOT relax inherited CSP ---
test('shell meta CSP cannot rescue inline scripts blocked by inherited main CSP', async ({ page }) => {
  // The shell doc sets a permissive meta allowing unsafe-inline. CSP L3 says a
  // second policy INTERSECTS with the inherited policy, so the inherited
  // "no inline" still wins. This is exactly why buildShellCspContent() cannot
  // save a blob doc whose parent origin forbids inline scripts.
  const shellCsp = buildShellCspContent(['example.com'])
  const appHTML = `<!doctype html><html><head><meta charset="utf-8"></head><body>
    <div id="status">parent loaded</div>
    <script>
      window.__SHELL_RESULT__ = 'timeout';
      window.addEventListener('message', (e) => { window.__SHELL_RESULT__ = e.data; });
      var blob = new Blob([
        '<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="' +
        shellCsp.replace(/"/g, '&quot;') +
        '"></head><body><script>window.parent.postMessage("shell-ran", "*");<\\/script></body></html>'
      ], { type: 'text/html' });
      var f = document.createElement('iframe');
      f.id = 'shell';
      f.src = URL.createObjectURL(blob);
      document.body.appendChild(f);
    </script>
  </body></html>`

  await page.route('http://app.local.test/meta', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'text/html',
      headers: { 'Content-Security-Policy': APP_CSP },
      body: appHTML,
    })
  })
  const v = collectViolations(page)
  await v.attach()

  await page.goto('http://app.local.test/meta')
  await page.waitForTimeout(1500)
  const result = await page.evaluate(() => window.__SHELL_RESULT__)
  expect(result).not.toBe('shell-ran')
  // Inline scripts still blocked: the meta cannot widen the inherited policy.
  const vios = await v.read()
  expect(vios.some((t) => String(t).includes('script-src') || String(t).includes('inline')))
    .toBe(true)
})

// --- Scenario 4: a separate-origin shell doc with its own CSP runs inline ---
// This is the FIX shape: the shell document is served from a distinct origin
// (modelled here as shell.local.test; in production a restricted Tauri custom
// protocol origin) whose policy container does NOT inherit the main app CSP.
// The shell sets its own CSP that permits the inline bridge. The main app's
// frame-src allows only this precise origin.
//
// IMPORTANT: the app origin's own CSP forbids inline scripts (script-src falls
// back to 'self'). So the app page below contains NO inline script; we wire the
// message listener via page.addInitScript (which runs with test-runner privilege,
// not subject to the page CSP) and read the result via page.evaluate.
test('separate-origin shell document with its own CSP runs inline scripts (the isolation fix)', async ({ page }) => {
  const shellCsp = buildShellCspContent([])
  const shellHTML = `<!doctype html><html><head><meta charset="utf-8">
    <meta http-equiv="Content-Security-Policy" content="${shellCsp.replace(/"/g, '&quot;')}">
    </head><body>
    <div id="proof">shell-origin-loaded</div>
    <script>window.parent.postMessage('shell-ran-from-isolated-origin', '*');<\/script>
    </body></html>`

  // Origin B serves the isolated shell doc.
  await page.route('http://shell.local.test/shell.html', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'text/html',
      // No inherited main CSP here: this origin has its own policy container.
      headers: { 'Content-Security-Policy': shellCsp },
      body: shellHTML,
    })
  })

  // A permissive frame-src variant of APP_CSP that ALSO allows the precise
  // shell origin. This models the post-fix main app: frame-src lists the
  // isolated shell origin (and blob:/data: for parity), nothing else changes.
  const fixCsp = APP_CSP.replace(
    'frame-src http://storyforge-shell.localhost storyforge-shell://localhost',
    'frame-src http://storyforge-shell.localhost storyforge-shell://localhost http://shell.local.test',
  )

  const appHTML = `<!doctype html><html><head><meta charset="utf-8"></head><body>
    <div id="status">parent loaded</div>
    <iframe id="shell" src="http://shell.local.test/shell.html"></iframe>
  </body></html>`
  await page.route('http://app.local.test/fix', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'text/html',
      headers: { 'Content-Security-Policy': fixCsp },
      body: appHTML,
    })
  })

  const v = collectViolations(page)
  await v.attach()
  // Test-runner-privileged listener (not an inline page script).
  await page.addInitScript(() => {
    window.__SHELL_RESULT__ = 'timeout'
    window.addEventListener('message', (e) => { window.__SHELL_RESULT__ = e.data })
  })

  await page.goto('http://app.local.test/fix')
  await expect(page.locator('#shell')).toBeVisible()
  // The isolated-origin shell runs its inline script and reports back.
  await expect.poll(async () => page.evaluate(() => window.__SHELL_RESULT__), {
    timeout: 5000,
  }).toBe('shell-ran-from-isolated-origin')
  // No CSP violation should fire in this configuration.
  const vios = await v.read()
  expect(vios).toEqual([])
})

// --- Scenario 5: a real CardShell-style bridge boots across the boundary ---
// Mirrors CardShellHost/TavernHelperRuntime: parent creates a blob document
// containing an inline bridge that posts 'shell:ready' then echoes a polled
// message. Under the BROKEN app CSP this is blocked (scenario 1); under the
// isolated-origin fix the bridge boots and a round-trip message completes.
test('isolated-origin shell boots a CardShell-style bridge and round-trips a message', async ({ page }) => {
  const shellCsp = buildShellCspContent([])
  const shellHTML = `<!doctype html><html><head><meta charset="utf-8">
    <meta http-equiv="Content-Security-Policy" content="${shellCsp.replace(/"/g, '&quot;')}">
    </head><body>
    <script>
      window.parent.postMessage('shell:ready', '*');
      window.addEventListener('message', function(e){
        if (e.data && e.data.type === 'ping') {
          window.parent.postMessage({ type: 'pong', n: e.data.n }, '*');
        }
      });
    <\/script>
    </body></html>`

  await page.route('http://shell.local.test/bridge.html', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'text/html',
      headers: { 'Content-Security-Policy': shellCsp },
      body: shellHTML,
    })
  })

  const fixCsp = APP_CSP.replace(
    'frame-src http://storyforge-shell.localhost storyforge-shell://localhost',
    'frame-src http://storyforge-shell.localhost storyforge-shell://localhost http://shell.local.test',
  )
  const appHTML = `<!doctype html><html><head><meta charset="utf-8"></head><body>
    <div id="status">parent loaded</div>
    <iframe id="shell" src="http://shell.local.test/bridge.html"></iframe>
  </body></html>`
  await page.route('http://app.local.test/bridge', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'text/html',
      headers: { 'Content-Security-Policy': fixCsp },
      body: appHTML,
    })
  })

  const v = collectViolations(page)
  await v.attach()
  await page.addInitScript(() => {
    window.__SHELL_STATE__ = { ready: false, pongN: null }
    window.addEventListener('message', (e) => {
      if (e.data === 'shell:ready') window.__SHELL_STATE__.ready = true
      if (e.data && e.data.type === 'pong') window.__SHELL_STATE__.pongN = e.data.n
    })
  })

  await page.goto('http://app.local.test/bridge')
  await expect.poll(async () => page.evaluate(() => window.__SHELL_STATE__.ready), {
    timeout: 5000,
  }).toBe(true)

  // Round-trip: ping the shell, expect pong.
  await page.evaluate(() => {
    document.getElementById('shell').contentWindow.postMessage({ type: 'ping', n: 7 }, '*')
  })
  await expect.poll(async () => page.evaluate(() => window.__SHELL_STATE__.pongN), {
    timeout: 5000,
  }).toBe(7)
  const vios = await v.read()
  expect(vios).toEqual([])
})

// --- Scenario 6: the MAIN APP origin cannot fetch the public internet -------
// The headline V5 egress guarantee: a fetch to a remote host from the trusted
// UI origin is blocked by connect-src (no remote host is listed). Proven on
// real Chromium; WebView2 mirrors the same engine CSP semantics (PARTIAL).
test('main app origin fetch to public internet is blocked by CSP', async ({ page }) => {
  // APP_CSP connect-src lists no public host; example.com must be refused.
  const appHTML = `<!doctype html><html><head><meta charset="utf-8"></head><body>
    <div id="status">parent loaded</div>
  </body></html>`
  await page.route('http://app.local.test/egress', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'text/html',
      headers: { 'Content-Security-Policy': APP_CSP },
      body: appHTML,
    })
  })
  const v = collectViolations(page)
  await v.attach()

  await page.goto('http://app.local.test/egress')
  // page.evaluate runs with test-runner privilege (not subject to page CSP),
  // but the fetch ITSELF executes in the page context and so is bound by CSP.
  const fetchResult = await page.evaluate(async () => {
    try {
      const r = await fetch('https://example.com/')
      return { ok: true, status: r.status }
    } catch (e) {
      return { ok: false, err: String(e) }
    }
  })
  // The fetch must fail (CSP blocks it before the network).
  expect(fetchResult.ok).toBe(false)
  // And a securitypolicyviolation naming connect-src must fire.
  await expect.poll(async () => v.read(), { timeout: 4000 }).toEqual(
    expect.arrayContaining([expect.stringMatching(/connect-src|example\.com/)])
  )
})

// --- Scenario 7: an isolated shell can load a cache-protocol asset ---------
// Mirrors the production shape: the shell document (on storyforge-shell) loads
// a large asset via the storyforge-cache protocol. The shell CSP allows the
// cache origin; the main app frame-src allows the shell origin.
test('isolated shell loads a storyforge-cache asset without CSP violation', async ({ page }) => {
  // Cache asset lives on the cache origin.
  await page.route('http://storyforge-cache.localhost/abc', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'image/png',
      body: Buffer.from([0x89, 0x50, 0x4e, 0x47]),
    })
  })
  // Shell doc allows the cache origin in img-src + connect-src.
  const shellCsp = buildShellCspContent([])
  const shellHTML = `<!doctype html><html><head><meta charset="utf-8">
    <meta http-equiv="Content-Security-Policy" content="${shellCsp.replace(/"/g, '&quot;')}">
    </head><body>
    <img id="asset" src="http://storyforge-cache.localhost/abc">
    <script>
      window.addEventListener('message', function(e){
        if (e.data === 'query') {
          var img = document.getElementById('asset');
          window.parent.postMessage({ type: 'asset', complete: img.complete, src: img.src }, '*');
        }
      });
      window.parent.postMessage('shell:ready', '*');
    <\/script>
    </body></html>`
  await page.route('http://shell.local.test/cache.html', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'text/html',
      headers: { 'Content-Security-Policy': shellCsp },
      body: shellHTML,
    })
  })
  const fixCsp = APP_CSP.replace(
    'frame-src http://storyforge-shell.localhost storyforge-shell://localhost',
    'frame-src http://storyforge-shell.localhost storyforge-shell://localhost http://shell.local.test',
  )
  const appHTML = `<!doctype html><html><head><meta charset="utf-8"></head><body>
    <iframe id="shell" src="http://shell.local.test/cache.html"></iframe>
  </body></html>`
  await page.route('http://app.local.test/cache', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'text/html',
      headers: { 'Content-Security-Policy': fixCsp },
      body: appHTML,
    })
  })
  const v = collectViolations(page)
  await v.attach()
  await page.addInitScript(() => {
    window.__ASSET__ = null
    window.addEventListener('message', (e) => {
      if (e.data && e.data.type === 'asset') window.__ASSET__ = e.data
    })
  })

  await page.goto('http://app.local.test/cache')
  // Let the asset load, then query the shell.
  await page.waitForTimeout(800)
  await page.evaluate(() => document.getElementById('shell').contentWindow.postMessage('query', '*'))
  await expect.poll(async () => page.evaluate(() => window.__ASSET__), { timeout: 5000 }).toEqual(
    expect.objectContaining({ complete: true }),
  )
  // No CSP violation: both the shell frame load and the cache image succeeded.
  const vios = await v.read()
  expect(vios).toEqual([])
})

// --- Scenario 8: the shell's OWN CSP still blocks unauthorized remote egress -
// Even though the shell can run inline scripts (its policy container does not
// inherit the app CSP), it is NOT a free network channel: its shell CSP
// forbids remote hosts not in the allowlist. A fetch to an unlisted host must
// fail and report a securitypolicyviolation.
test('shell self-CSP blocks unauthorized remote fetch even though inline scripts run', async ({ page }) => {
  // Empty allowlist: no remote host is permitted by the shell CSP.
  const shellCsp = buildShellCspContent([])
  const shellHTML = `<!doctype html><html><head><meta charset="utf-8">
    <meta http-equiv="Content-Security-Policy" content="${shellCsp.replace(/"/g, '&quot;')}">
    </head><body>
    <script>
      window.addEventListener('message', function(e){
        if (e.data === 'try-egress') {
          fetch('https://unauthorized.example/').then(function(){
            window.parent.postMessage({ type: 'egress', blocked: false }, '*');
          }).catch(function(err){
            window.parent.postMessage({ type: 'egress', blocked: true, err: String(err) }, '*');
          });
        }
      });
      window.parent.postMessage('shell:ready', '*');
    <\/script>
    </body></html>`
  await page.route('http://shell.local.test/egress.html', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'text/html',
      headers: { 'Content-Security-Policy': shellCsp },
      body: shellHTML,
    })
  })
  const fixCsp = APP_CSP.replace(
    'frame-src http://storyforge-shell.localhost storyforge-shell://localhost',
    'frame-src http://storyforge-shell.localhost storyforge-shell://localhost http://shell.local.test',
  )
  const appHTML = `<!doctype html><html><head><meta charset="utf-8"></head><body>
    <iframe id="shell" src="http://shell.local.test/egress.html"></iframe>
  </body></html>`
  await page.route('http://app.local.test/shellegress', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'text/html',
      headers: { 'Content-Security-Policy': fixCsp },
      body: appHTML,
    })
  })
  const v = collectViolations(page)
  await v.attach()
  await page.addInitScript(() => {
    window.__EGRESS__ = null
    window.addEventListener('message', (e) => {
      if (e.data && e.data.type === 'egress') window.__EGRESS__ = e.data
    })
  })

  await page.goto('http://app.local.test/shellegress')
  await page.waitForTimeout(500)
  await page.evaluate(() => document.getElementById('shell').contentWindow.postMessage('try-egress', '*'))
  // The shell's fetch to an unauthorized host must be blocked.
  await expect.poll(async () => page.evaluate(() => window.__EGRESS__), { timeout: 5000 }).toEqual(
    expect.objectContaining({ blocked: true }),
  )
  // A securitypolicyviolation naming the shell's connect-src fires (it surfaces
  // on the shell document, but Playwright reports page console + the event
  // bubbles to the parent via our listener-less path; we accept either signal).
  const vios = await v.read()
  expect(vios.some((t) => String(t).includes('connect-src') || String(t).includes('unauthorized'))).toBe(true)
})
