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
