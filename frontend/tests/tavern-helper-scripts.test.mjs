import test from 'node:test'
import assert from 'node:assert/strict'
import {
  orderedTavernHelperFromShells,
  parseShellEntry,
} from '../src/utils/tavernHelperScripts.js'

test('parseShellEntry handles serde externally tagged remote_url', () => {
  const p = parseShellEntry({
    remote_url: {
      url: 'https://testingcf.jsdelivr.net/gh/MagicalAstrogy/MagVarUpdate/artifact/bundle.js',
    },
  })
  assert.equal(p.kind, 'remote_url')
  assert.match(p.url, /MagVarUpdate/)
})

test('parseShellEntry handles inline_js', () => {
  const p = parseShellEntry({ inline_js: { js: 'window.x=1' } })
  assert.equal(p.kind, 'inline_js')
  assert.equal(p.js, 'window.x=1')
})

test('orderedTavernHelperFromShells preserves order and filters kinds', () => {
  const shells = [
    {
      kind: 'status_bar',
      entry: { remote_url: { url: 'https://example.com/status/index.html' } },
      label: '状态栏',
      deps: [],
      trigger: 'x',
    },
    {
      kind: 'tavern_helper_module',
      entry: {
        remote_url: {
          url: 'https://testingcf.jsdelivr.net/gh/MagicalAstrogy/MagVarUpdate/artifact/bundle.js',
        },
      },
      label: '【命定之诗】MVU beta',
      deps: [],
      trigger: 'tavern_helper.scripts',
    },
    {
      kind: 'tavern_helper_module',
      entry: {
        remote_url: {
          url: 'https://testingcf.jsdelivr.net/gh/The-poem-of-destiny/FrontEnd-for-destined-journey@1.6.2/dist/data_schema/index.js',
        },
      },
      label: '【命定之诗】mvu zod',
      deps: [],
      trigger: 'tavern_helper.scripts',
    },
    {
      kind: 'tavern_helper_module',
      entry: { inline_js: { js: 'x'.repeat(300) } },
      label: '【命定之诗】创意工坊v6.1',
      deps: [],
      trigger: 'tavern_helper.scripts',
    },
  ]
  const ordered = orderedTavernHelperFromShells(shells)
  assert.equal(ordered.length, 3)
  assert.equal(ordered[0].label, '【命定之诗】MVU beta')
  assert.equal(ordered[1].kind, 'remote_url')
  assert.equal(ordered[2].kind, 'inline_js')
  assert.ok(ordered[2].js.length >= 300)
})
