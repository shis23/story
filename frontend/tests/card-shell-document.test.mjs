import test from 'node:test'
import assert from 'node:assert/strict'
import {
  createCardShellRuntimeCompatibilityScript,
  isCardShellBridgeMessageForSession,
  makeCardShellInlineModuleId,
  ownsCardShellInlineModule,
  rewriteCardShellTopBridgeAccess,
} from '../src/utils/cardShellDocument.js'

test('scopes inline module identifiers to their owning shell host', () => {
  const statusModule = makeCardShellInlineModuleId('card-shell-status', 1)
  const openingModule = makeCardShellInlineModuleId('card-shell-opening', 1)

  assert.equal(statusModule, 'card-shell-status-mod-1')
  assert.equal(openingModule, 'card-shell-opening-mod-1')
  assert.notEqual(statusModule, openingModule)
})

test('only the shell owning an inline module answers its bridge request', () => {
  const statusModules = new Map([['card-shell-status-mod-1', 'status source']])
  const openingModules = new Map([['card-shell-opening-mod-1', 'opening source']])

  assert.equal(ownsCardShellInlineModule(statusModules, 'card-shell-status-mod-1'), true)
  assert.equal(ownsCardShellInlineModule(openingModules, 'card-shell-status-mod-1'), false)
  assert.equal(ownsCardShellInlineModule(openingModules, ''), false)
})

test('routes bridge messages to the host session that created the iframe', () => {
  const matching = {
    data: { __sf_shell_bridge: true, shellSession: 'status-host:1' },
  }
  const sibling = {
    data: { __sf_shell_bridge: true, shellSession: 'message-host:1' },
  }

  assert.equal(isCardShellBridgeMessageForSession(matching, 'status-host:1'), true)
  assert.equal(isCardShellBridgeMessageForSession(sibling, 'status-host:1'), false)
  assert.equal(isCardShellBridgeMessageForSession({ data: { __sf_shell_bridge: true } }, 'status-host:1'), false)
})

test('rewrites known top-level ST globals to the isolated shell bridge', () => {
  const source = [
    "window.top?.TavernHelper.getCharWorldbookNames('current')",
    "window.top.TavernHelper.getWorldbook('book')",
    'window.top?.tavernHelper.updateWorldbookWith(name, update)',
    'window.top?.SillyTavern.getContext()',
  ].join(';')

  const rewritten = rewriteCardShellTopBridgeAccess(source)

  assert.match(rewritten, /window\.TavernHelper\.getCharWorldbookNames/)
  assert.match(rewritten, /window\.TavernHelper\.getWorldbook/)
  assert.match(rewritten, /window\.tavernHelper\.updateWorldbookWith/)
  assert.match(rewritten, /window\.SillyTavern\.getContext/)
  assert.doesNotMatch(rewritten, /window\.top\??\.(?:TavernHelper|tavernHelper|SillyTavern)/)
})

test('does not rewrite unrelated top-window access', () => {
  const source = 'if (window.top !== window) window.top.postMessage({ ready: true }, "*")'

  assert.equal(rewriteCardShellTopBridgeAccess(source), source)
})

test('provides diagnostic compatibility globals so a card reports unavailable features instead of hanging', async () => {
  const script = createCardShellRuntimeCompatibilityScript({
    worldbookName: 'storyforge:campaign:campaign-a',
  })

  assert.match(script, /window\.getTavernHelperVersion/)
  assert.match(script, /window\.waitGlobalInitialized/)
  assert.match(script, /Mvu unavailable in the StoryForge card shell/)
  assert.doesNotMatch(script, /window\.top/)

  const requests = []
  const shellWindow = {
    TavernHelper: {},
    __sfShellAsk: async (type, payload) => {
      requests.push({ type, payload })
      if (type === 'mvu_status') return { ready: true }
      if (type === 'campaign_worldbook_get') return [{ name: '命定系统-测试核心', enabled: true }]
      if (type === 'campaign_worldbook_update') return { updated: 1 }
      throw new Error(`unexpected request: ${type}`)
    },
  }
  Function('window', script)(shellWindow)

  assert.equal(shellWindow.getTavernHelperVersion(), '4.3.17')
  assert.equal(
    shellWindow.TavernHelper.getCharWorldbookNames('current').primary,
    'storyforge:campaign:campaign-a',
  )
  assert.deepEqual(
    await shellWindow.TavernHelper.getWorldbook('storyforge:campaign:campaign-a'),
    [{ name: '命定系统-测试核心', enabled: true }],
  )

  await shellWindow.TavernHelper.updateWorldbookWith(
    'storyforge:campaign:campaign-a',
    (entries) => entries.map((entry) => ({ ...entry, enabled: false })),
  )
  assert.deepEqual(requests.at(-1), {
    type: 'campaign_worldbook_update',
    payload: {
      name: 'storyforge:campaign:campaign-a',
      entries: [{ name: '命定系统-测试核心', enabled: false }],
    },
  })

  assert.equal(await shellWindow.waitGlobalInitialized('Mvu'), shellWindow.Mvu)
})
