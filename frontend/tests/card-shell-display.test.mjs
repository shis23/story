import test from 'node:test'
import assert from 'node:assert/strict'
import {
  extractShellMountsFromDisplay,
  classifyShellUrl,
  resolveShellSurfaces,
} from '../src/utils/cardShellDisplay.js'

const HOME =
  'https://testingcf.jsdelivr.net/gh/The-poem-of-destiny/FrontEnd-for-destined-journey@1.6.2/dist/home/index.html'
const STATUS =
  'https://testingcf.jsdelivr.net/gh/The-poem-of-destiny/FrontEnd-for-destined-journey@1.6.2/dist/status/index.html'
const CUSTOM =
  'https://testingcf.jsdelivr.net/gh/The-poem-of-destiny/FrontEnd-for-destined-journey@1.6.2/dist/custom_start/index.html'

test('classifyShellUrl maps test-card paths', () => {
  assert.equal(classifyShellUrl(HOME), 'opening_home')
  assert.equal(classifyShellUrl(STATUS), 'status')
  assert.equal(classifyShellUrl(CUSTOM), 'opening_custom')
})

test('extractShellMountsFromDisplay finds body.load urls and strips glue', () => {
  const display = [
    '前序叙事。',
    '```',
    '<body>',
    `<script>$('body').load('${STATUS}')</script>`,
    '</body>',
    '```',
    '后序。',
    `$('body').load("${HOME}")`,
  ].join('\n')

  const { mounts, residualText } = extractShellMountsFromDisplay(display)
  assert.equal(mounts.length, 2)
  assert.deepEqual(
    mounts.map((m) => m.kind).sort(),
    ['opening_home', 'status'],
  )
  assert.match(residualText, /前序叙事/)
  assert.match(residualText, /后序/)
  assert.doesNotMatch(residualText, /\$\('body'\)\.load/)
})

test('resolveShellSurfaces fills missing campaign fallbacks', () => {
  const resolved = resolveShellSurfaces([], {
    statusUrl: STATUS,
    openingUrl: HOME,
  })
  assert.equal(resolved.length, 2)
  assert.equal(resolved[0].kind, 'status')
  assert.equal(resolved[1].kind, 'opening_home')
})

test('partitionShellMountsByTrust auto-mounts only card-manifest urls (H3)', async () => {
  const { partitionShellMountsByTrust } = await import('../src/utils/cardShellDisplay.js')
  const attacker = 'https://files.catbox.moe/attacker.html'
  const mounts = [
    { url: HOME, kind: 'opening_home' },
    { url: attacker, kind: 'message_html' },
  ]

  const { allowed, needsConfirmation } = partitionShellMountsByTrust(mounts, [HOME])
  assert.deepEqual(allowed.map((m) => m.url), [HOME])
  assert.deepEqual(needsConfirmation.map((m) => m.url), [attacker])

  // 用户显式放行后可挂载
  const approved = partitionShellMountsByTrust(mounts, [HOME], [attacker])
  assert.equal(approved.needsConfirmation.length, 0)
  assert.equal(approved.allowed.length, 2)

  // 无任何信任上下文（legacy/无 manifest）：全部需要确认，fail closed
  const none = partitionShellMountsByTrust(mounts, null, null)
  assert.equal(none.allowed.length, 0)
  assert.equal(none.needsConfirmation.length, 2)
})
