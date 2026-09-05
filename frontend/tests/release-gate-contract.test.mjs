import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

test('the release gate includes every mandatory deterministic suite', () => {
  const gate = readFileSync(new URL('../../scripts/verify-release.ps1', import.meta.url), 'utf8')
  for (const command of [
    'run-release-build-tests.ps1',
    'test:ui',
    'test:csp',
    'test:mobile-chrome',
    'smoke:ui',
  ]) {
    assert.ok(gate.includes(command), `Missing mandatory gate: ${command}`)
  }
  const steps = [...gate.matchAll(/^\s+Invoke-NativeStep -Name /gm)].length + 1
  const count = gate.match(/\$TotalSteps = if \(\$SecretScanOnly\) \{ 1 \} else \{ (\d+) \}/)
  assert.equal(Number(count?.[1]), steps)
})

test('UI smoke owns its server process and resolves config independently of cwd', () => {
  const smoke = readFileSync(new URL('../../scripts/run-ui-smoke.ps1', import.meta.url), 'utf8')
  assert.match(smoke, /Join-Path \$frontendDir 'playwright\.config\.mjs'/)
  assert.match(smoke, /Start-Process -FilePath \$nodeBin/)
  assert.match(smoke, /node_modules\\vite\\bin\\vite\.js/)
  assert.doesNotMatch(smoke, /Start-Process -FilePath 'npm\.cmd'/)
  assert.match(smoke, /Stop-Process -Id \$server\.Id/)
})
