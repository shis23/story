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

test('maps selector variables and MVU writes to the shell persistence bridge', async () => {
  const script = createCardShellRuntimeCompatibilityScript({
    selectorVariables: {
      'message:11': { stat_data: { '主角': { '生命值': 72 } } },
      'character:current': { status_theme_id: 'parchment' },
    },
  })
  const requests = []
  const shellWindow = {
    TavernHelper: {},
    __sfShellAsk: async (type, payload) => {
      requests.push({ type, payload })
      if (type === 'mvu_status') return { ready: true }
      if (type === 'shell_variables_set') return { ok: true }
      throw new Error(`unexpected request: ${type}`)
    },
  }

  Function('window', script)(shellWindow)

  const messageSelector = { type: 'message', message_id: 11 }
  assert.deepEqual(shellWindow.getVariables(messageSelector), {
    stat_data: { '主角': { '生命值': 72 } },
  })
  assert.deepEqual(shellWindow.Mvu.getMvuData(messageSelector), {
    stat_data: { '主角': { '生命值': 72 } },
  })

  await shellWindow.Mvu.replaceMvuData(
    { stat_data: { '主角': { '生命值': 99 } } },
    messageSelector,
  )
  await shellWindow.insertOrAssignVariables(
    { status_theme_id: 'crimson' },
    { type: 'character' },
  )
  await shellWindow.deleteVariable({ type: 'character' }, 'status_theme_id')

  // M2：replace 家族（replaceMvuData/deleteVariable）不再下发整桶快照，
  // 而是相对本壳视图的键级补丁；merge 家族保持补丁语义。
  assert.deepEqual(requests.filter((request) => request.type === 'shell_variables_set'), [
    {
      type: 'shell_variables_set',
      payload: {
        selector: messageSelector,
        variables: { sets: { stat_data: { '主角': { '生命值': 99 } } }, deletes: [] },
        mode: 'patch',
      },
    },
    {
      type: 'shell_variables_set',
      payload: {
        selector: { type: 'character' },
        variables: { status_theme_id: 'crimson' },
        mode: 'merge',
      },
    },
    {
      type: 'shell_variables_set',
      payload: {
        selector: { type: 'character' },
        variables: { sets: {}, deletes: ['status_theme_id'] },
        mode: 'patch',
      },
    },
  ])
})

test('bridges real campaign stat data into the Mvu shim read path (M5)', async () => {
  const script = createCardShellRuntimeCompatibilityScript({
    selectorVariables: { 'message:current': { stat_data: { '主角': { '心情': '紧张' } } } },
    mvuStatData: { '主角': { '生命值': 55, '心情': '平静' }, '世界': { '时间': '黄昏' } },
  })
  const shellWindow = {
    TavernHelper: {},
    __sfShellAsk: async (type) => {
      if (type === 'mvu_data_get') return { '主角': { '生命值': 40 } }
      if (type === 'mvu_status') return { ready: true }
      throw new Error(`unexpected request: ${type}`)
    },
  }
  Function('window', script)(shellWindow)

  // events 常量表存在（缺失时 eventOn(Mvu.events.…) 会 TypeError 杀死整段脚本）
  assert.equal(shellWindow.Mvu.events.VARIABLE_UPDATE_ENDED, 'mag_variable_update_ended')
  assert.equal(shellWindow.Mvu.events.SINGLE_VARIABLE_UPDATED, 'mag_variable_updated')

  // 注入的真实变量树打底，壳自写桶键级覆盖
  const data = shellWindow.Mvu.getMvuData({ type: 'message' })
  assert.deepEqual(data.stat_data, {
    '主角': { '生命值': 55, '心情': '紧张' },
    '世界': { '时间': '黄昏' },
  })

  // 刷新桥：底座替换为宿主返回的最新树
  await shellWindow.Mvu.refreshMvuData()
  const refreshed = shellWindow.Mvu.getMvuData({ type: 'message' })
  assert.deepEqual(refreshed.stat_data, { '主角': { '生命值': 40, '心情': '紧张' } })
})

test('encodes persisted selector data before injecting it into a shell script', () => {
  const script = createCardShellRuntimeCompatibilityScript({
    selectorVariables: { message: { stat_data: { note: '</script><img src=x>' } } },
  })

  assert.doesNotMatch(script, /<\/script>/i)
  assert.match(script, /\\u003c\/script\\u003e/i)
})
