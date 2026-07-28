import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import {
  orderedTavernHelperFromShells,
  parseShellEntry,
  collectVisibleThButtons,
  analyzeEsmModuleSource,
  extractBareImportTarget,
  fetchClassicScriptSource,
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

test('collectVisibleThButtons unique preserve order', () => {
  const shells = [
    {
      kind: 'tavern_helper_module',
      label: 'MVU',
      entry: { remote_url: { url: 'https://example.com/a.js' } },
      buttons: ['重新读取初始变量', '重新处理变量'],
      deps: [],
    },
    {
      kind: 'tavern_helper_module',
      label: '工坊',
      entry: { inline_js: { js: 'x'.repeat(300) } },
      buttons: ['命定创意工坊', '重新读取初始变量'],
      deps: [],
    },
  ]
  const scripts = orderedTavernHelperFromShells(shells)
  const buttons = collectVisibleThButtons(scripts)
  assert.deepEqual(
    buttons.map((b) => b.name),
    ['重新读取初始变量', '重新处理变量', '命定创意工坊'],
  )
})

test('deferred inline_js without body is kept', () => {
  const shells = [
    {
      kind: 'tavern_helper_module',
      label: '创意工坊',
      entry: { inline_js: { js: '', deferred: true, byte_len: 60000 } },
      buttons: ['命定创意工坊'],
      deps: [],
    },
  ]
  const ordered = orderedTavernHelperFromShells(shells)
  assert.equal(ordered.length, 1)
  assert.equal(ordered[0].deferred, true)
  assert.equal(ordered[0].js, '')
})

test('extractBareImportTarget only unwraps a single side-effect import', () => {
  assert.equal(
    extractBareImportTarget(
      " \ufeff import 'https://cdn.example.test/card-script.js';\r\n",
    ),
    'https://cdn.example.test/card-script.js',
  )
  assert.equal(
    extractBareImportTarget(
      "import thing from 'https://cdn.example.test/module.js';",
    ),
    null,
  )
  assert.equal(
    extractBareImportTarget(
      "import 'https://cdn.example.test/one.js';\nwindow.ready = true;",
    ),
    null,
  )
})

test('fetchClassicScriptSource follows an import-only wrapper and returns classic code', async () => {
  const calls = []
  const source = await fetchClassicScriptSource(
    'https://cards.example.test/wrapper.js',
    async (url) => {
      calls.push(url)
      if (url.endsWith('/wrapper.js')) return "import './bundle.js'"
      return 'window.cardScriptReady = true;'
    },
  )

  assert.deepEqual(calls, [
    'https://cards.example.test/wrapper.js',
    'https://cards.example.test/bundle.js',
  ])
  assert.equal(source, 'window.cardScriptReady = true;')
})

test('fetchClassicScriptSource rejects wrapper cycles', async () => {
  await assert.rejects(
    fetchClassicScriptSource(
      'https://cards.example.test/a.js',
      async (url) =>
        url.endsWith('/a.js') ? "import './b.js'" : "import './a.js'",
    ),
    /cycle/,
  )
})

test('ES module scanner recognizes minified and conventional imports', async () => {
  const source = [
    "import{registerMvuSchema as r}from'https://cdn.example.test/mvu.js';",
    'export{value as default}from"./value.js";',
    "import 'https://cdn.example.test/side-effect.js';",
    'const lazy = import("./lazy.js");',
  ].join('')
  const analysis = await analyzeEsmModuleSource(source)

  assert.deepEqual(analysis.imports.map((item) => item.specifier), [
    'https://cdn.example.test/mvu.js',
    './value.js',
    'https://cdn.example.test/side-effect.js',
    './lazy.js',
  ])
  assert.equal(analysis.isModule, true)
  assert.equal(
    (await analyzeEsmModuleSource('window.cardScriptReady = true;')).isModule,
    false,
  )
  assert.equal(
    analyzeEsmModuleSource('const load = () => import("./lazy.js");').isModule,
    true,
  )
})

test('TavernHelper classifies remote card entries before execution', () => {
  const source = fs.readFileSync(
    new URL('../src/components/TavernHelperRuntime.vue', import.meta.url),
    'utf8',
  )

  assert.match(source, /ask\('prepare_remote_script', \{ url: url \}\)/)
  assert.match(source, /window\.__sfThRunRemoteUrl\(item\.url\)/)
  assert.match(source, /descriptor\.module/)
  assert.doesNotMatch(source, /window\.__sfThImportUrl\(item\.url\)/)
})
