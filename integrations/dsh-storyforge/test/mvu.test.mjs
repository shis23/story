// MVU 面板与交互：与桌面实现对拍；DSH 装配（面板行、写入目标、作用域解析）。
// 运行：node --test integrations/dsh-storyforge/test/*.test.mjs

import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath, pathToFileURL } from 'node:url'

import * as mvu from '../lib/mvu.js'

const here = dirname(fileURLToPath(import.meta.url))
const desktopUtils = resolve(here, '../../../frontend/src/utils')
const desktopFile = (name) => pathToFileURL(resolve(desktopUtils, name)).href
const noDesktop = !existsSync(resolve(desktopUtils, 'mvuInteractions.js')) && 'desktop frontend not checked out'

const KEYS = [
  'hp',
  ' stat_data.hp ',
  '/世界/时间',
  'stat_data/角色/hp',
  'stat_data.stat_data.角色.<角色名>.好感度',
  '女性角色.{角色名}.好感度',
  'a..b',
  '<x>',
  '',
  null,
  'path/with/slash',
]

const VARIABLES = [
  { key: 'hp', value: 60 },
  { key: 'stat_data.mp', value: '30' },
  { key: '女性角色.小美.好感度', value: 12 },
  { key: '女性角色.{角色名}.信任', value: 5 },
  { key: '状态', value: '战斗中' },
  { key: '装备', value: { 武器: '短剑' } },
]

const BINDINGS = [
  { element: 'hp_bar', variable_key: 'hp', display: { kind: 'bar', max: 100 } },
  { element: 'mp_bar', variable_key: 'mp', display: { kind: 'bar', max: 0 } },
  { element: 'love', variable_key: '女性角色.{角色名}.好感度', display: { kind: 'bar', max: 50 } },
  { element: 'trust', variable_key: '女性角色.<角色名>.信任', display: { kind: 'text' } },
  { element: 'state', variable_key: '状态', display: { kind: 'tag' } },
  { element: 'mood', variable_key: '状态', display: { kind: 'icon', mapping: { 战斗中: '⚔', _default: '·' } } },
  { element: 'none', variable_key: '不存在', display: { kind: 'icon', mapping: {} } },
]

const EXPRESSIONS = [
  ['+5', 60, 'hp'],
  ['-5', 60, 'hp'],
  ['-5', '战斗中', '状态'],
  ['+3', null, 'hp'],
  ['hp - 10', 60, 'hp'],
  ['stat_data.hp - 10', 60, 'hp'],
  ['mp - 10', 60, 'hp'],
  ['42', 1, 'hp'],
  ['"文字"', 1, 'x'],
  ['{"a":1}', null, 'x'],
  ['战斗中', null, '状态'],
  ['hp ＋ 10', 60, 'hp'],
  ['Math.max(1, 2)', 1, 'hp'],
  ['setvar("x", 1)', 1, 'x'],
  ['', 1, 'x'],
  ['[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]', null, 'x'],
]

const MAPPINGS = [
  { element_label: '攻击', actions: [{ kind: 'modify_variable', key: 'hp', value_expr: '-10' }, { kind: 'trigger_next_turn', hint: '我发起攻击' }] },
  {
    element_label: '组合',
    actions: [
      { kind: 'multi', actions: [{ kind: 'modify_variable', key: '女性角色.<角色名>.好感度', value_expr: '+2' }, { kind: 'run_original_js', js_snippet: 'x()', description: '特效' }] },
      { kind: 'trigger_next_turn', hint: '  ' },
      { kind: 'teleport' },
    ],
  },
  { element_label: '新变量', actions: [{ kind: 'modify_variable', key: 'stat_data/金币', value_expr: '100' }] },
  { element_label: '坏表达式', actions: [{ kind: 'modify_variable', key: 'hp', value_expr: 'mp - 1' }] },
]

test('key helpers match the desktop implementation', { skip: noDesktop }, async () => {
  const desktop = await import(desktopFile('mvuKey.js'))
  for (const key of KEYS) {
    assert.equal(mvu.normalizeMvuKey(key), desktop.normalizeMvuKey(key), JSON.stringify(key))
    for (const name of ['', '小美']) {
      assert.equal(mvu.expandMvuTemplateKey(key, name), desktop.expandMvuTemplateKey(key, name), JSON.stringify([key, name]))
      assert.deepEqual(
        mvu.findMvuVariableForInstance(VARIABLES, key, name),
        desktop.findMvuVariableForInstance(VARIABLES, key, name),
        JSON.stringify([key, name]),
      )
    }
  }
})

test('status bar values match the desktop model', { skip: noDesktop }, async () => {
  const desktop = await import(desktopFile('mvuStatusBarModel.js'))
  for (const binding of BINDINGS) {
    for (const name of ['', '小美']) {
      const label = JSON.stringify([binding.element, name])
      assert.equal(mvu.getMvuValue(VARIABLES, binding.variable_key, name), desktop.getMvuValue(VARIABLES, binding.variable_key, name), label)
      assert.equal(mvu.mvuBarPercent(binding, VARIABLES, name), desktop.mvuBarPercent(binding, VARIABLES, name), label)
      assert.equal(mvu.mvuIconFor(binding, VARIABLES, '·', name), desktop.mvuIconFor(binding, VARIABLES, '·', name), label)
      assert.equal(mvu.mvuDisplayValue(VARIABLES, binding.variable_key, '—', name), desktop.mvuDisplayValue(VARIABLES, binding.variable_key, '—', name), label)
    }
  }
  for (const value of [null, undefined, '12', 'abc', 7]) assert.equal(mvu.toMvuNumber(value), desktop.toMvuNumber(value))
})

const CARD = {
  source_character_id: 'src-1',
  character_definitions: [
    { id: 'def-a', name: '小美' },
    { id: 'def-b', name: '阿强' },
  ],
}
const INSTANCES = [
  { id: 'inst-a', name: '小美', definition_id: 'def-a', variables: [{ key: 'hp', value: 40 }] },
  { id: 'inst-b', name: '阿强', definition_id: 'def-b', variables: [] },
  { id: 'inst-t', name: '路人', definition_id: null, variables: [{ key: 'hp', value: 1 }] },
]
const CAMPAIGN_VARIABLES = [
  { key: 'hp', value: 100 },
  { key: 'story_clock', value: '第1天' },
]

test('campaign sections match the desktop assembly', { skip: noDesktop }, async () => {
  const desktop = await import(desktopFile('campaignMvuStatusBar.js'))
  const translations = [
    { translation: { ui_bindings: BINDINGS, fallback_fragments: [{ description: 'x' }] } },
    { translation: { ui_bindings: [], fallback_fragments: [] } },
    { translation: null },
    null,
  ]
  for (const translationDetail of translations) {
    const input = { card: CARD, translationDetail, instances: INSTANCES, campaignVariables: CAMPAIGN_VARIABLES }
    assert.deepEqual(mvu.buildCampaignMvuStatusSections(input), desktop.buildCampaignMvuStatusSections(input))
  }
  assert.deepEqual(mvu.buildCampaignMvuStatusSections({ instances: null }), desktop.buildCampaignMvuStatusSections({ instances: null }))
})

test('interaction plans match the desktop interpreter', { skip: noDesktop }, async () => {
  const desktop = await import(desktopFile('mvuInteractions.js'))
  for (const [expr, current, key] of EXPRESSIONS) {
    assert.deepEqual(mvu.evaluateMvuValueExpr(expr, current, key), desktop.evaluateMvuValueExpr(expr, current, key), JSON.stringify(expr))
  }
  for (const mapping of MAPPINGS) {
    for (const name of ['', '小美']) {
      assert.deepEqual(
        mvu.planMvuInteraction(mapping, { variables: VARIABLES, instanceName: name }),
        desktop.planMvuInteraction(mapping, { variables: VARIABLES, instanceName: name }),
        JSON.stringify([mapping.element_label, name]),
      )
    }
  }
})

test('write scoping matches persistShellVariableWrite', { skip: noDesktop }, async () => {
  const desktop = await import(desktopFile('shellVariableOutbox.js'))
  const cases = [
    { instanceId: null, key: 'hp', value: 3 },
    { instanceId: 'inst-a', key: 'instance:inst-a:hp', value: { a: 1 } },
    { instanceId: 'inst-a', key: 'inst:hp', value: 'x' },
    { instanceId: null, key: 'inst:hp', value: 1 },
    { instanceId: null, key: 'campaign:story_clock', value: '第2天' },
    { instanceId: null, key: 'campaign:', value: 1 },
    { instanceId: 'inst-a', key: 'instance:inst-a:', value: 1 },
    { instanceId: null, key: '__storyforge_card_shell_variables', value: 1 },
    { instanceId: 'inst-a', key: 'instance:inst-a:__storyforge_x', value: 1 },
    { instanceId: null, key: '  ', value: 1 },
    { instanceId: null, key: 'k', value: undefined },
  ]
  for (const input of cases) {
    const calls = []
    const expected = await desktop.persistShellVariableWrite({
      campaignId: 'camp',
      ...input,
      setCampaignVariable: async (campaignId, key, value) => calls.push({ scope: 'campaign', instanceId: null, key, value }),
      setCharacterVariable: async (campaignId, instanceId, key, value) => calls.push({ scope: 'instance', instanceId, key, value }),
    })
    const actual = mvu.resolveVariableWrite(input)
    const label = JSON.stringify(input)
    assert.equal(actual.ok, expected.ok, label)
    assert.equal(actual.scope, expected.scope, label)
    assert.equal(actual.key, expected.key, label)
    if (expected.ok) {
      assert.deepEqual({ scope: actual.scope, instanceId: actual.instanceId, key: actual.key, value: actual.value }, calls[0], label)
    } else {
      assert.equal(actual.error, expected.error, label)
      assert.equal(calls.length, 0, label)
    }
  }
})

const STATUS = {
  campaign_id: 'camp',
  card_name: '测试卡',
  source_character_id: 'src-1',
  analyzed_at: '2026-10-14T00:00:00Z',
  translation: {
    ui_bindings: [
      { element: 'hp_bar', variable_key: 'hp', display: { kind: 'bar', max: 100 } },
      { element: 'state', variable_key: '状态', display: { kind: 'tag' } },
      { element: 'gear', variable_key: '装备', display: { kind: 'text' } },
    ],
    update_rules: ['受伤时 hp 减少'],
    interactions: [MAPPINGS[0], MAPPINGS[2]],
    fallback_fragments: [],
  },
  definitions: [{ id: 'def-a', name: '小美' }],
  instances: [{ id: 'inst-a', name: '小美', definition_id: 'def-a', is_temporary: false, variables: [{ key: 'hp', value: 20 }, { key: '装备', value: { 武器: '短剑' } }] }],
  campaign_variables: [{ key: '状态', value: '战斗中' }],
  pending_apply: [],
}

test('panel model renders rows the way the desktop status bar does', () => {
  assert.deepEqual(mvu.panelModel(null), { state: 'none' })
  assert.equal(mvu.panelModel({ ...STATUS, translation: null }).state, 'unanalyzed')
  const model = mvu.panelModel(STATUS)
  assert.equal(model.state, 'ready')
  assert.deepEqual(model.interactions, [
    { index: 0, label: '攻击' },
    { index: 1, label: '新变量' },
  ])
  assert.deepEqual(model.sections, [
    {
      instanceId: 'inst-a',
      instanceName: '小美',
      fallbackCount: 0,
      rows: [
        { element: 'hp_bar', label: 'hp', kind: 'bar', value: 20, max: 100, percent: 20, tone: 'err' },
        { element: 'state', label: '状态', kind: 'tag', text: '战斗中' },
        { element: 'gear', label: '装备', kind: 'text', text: '{"武器":"短剑"}' },
      ],
    },
  ])
  assert.deepEqual([60, 50, 26, 25].map(mvu.barTone), ['ok', 'warn', 'warn', 'err'])
  assert.deepEqual(model.fallbacks, [])
  const withFallback = mvu.panelModel({ ...STATUS, translation: { ...STATUS.translation, fallback_fragments: [{ description: '开场动画', reason: '需要 DOM' }] } })
  assert.deepEqual(withFallback.fallbacks, [{ description: '开场动画', reason: '需要 DOM' }])
  assert.equal(withFallback.sections[0].fallbackCount, 1)
})

test('interactions write to the single bound instance and keep hints for the next turn', () => {
  const attack = mvu.planInteraction(STATUS, STATUS.translation.interactions[0])
  assert.deepEqual(attack.writes, [{ ok: true, scope: 'instance', instanceId: 'inst-a', key: 'hp', value: 10 }])
  assert.deepEqual(attack.plan.hints, ['我发起攻击'])
  assert.deepEqual(attack.rejected, [])
  // 新变量落归一化键。
  assert.deepEqual(mvu.planInteraction(STATUS, STATUS.translation.interactions[1]).writes, [
    { ok: true, scope: 'instance', instanceId: 'inst-a', key: '金币', value: 100 },
  ])
  // 两个卡绑定实例：不猜，写 Campaign 作用域。
  const two = {
    ...STATUS,
    definitions: [...STATUS.definitions, { id: 'def-b', name: '阿强' }],
    instances: [...STATUS.instances, { id: 'inst-b', name: '阿强', definition_id: 'def-b', variables: [] }],
  }
  assert.deepEqual(mvu.planInteraction(two, STATUS.translation.interactions[0]).writes, [
    { ok: true, scope: 'campaign', instanceId: null, key: 'hp', value: 10 },
  ])
  // 内部命名空间被拦下，不交给 sidecar。
  const reserved = mvu.planInteraction(STATUS, { element_label: '坏', actions: [{ kind: 'modify_variable', key: '__storyforge_x', value_expr: '1' }] })
  assert.deepEqual(reserved.writes, [])
  assert.equal(reserved.rejected[0].error, 'reserved namespace key rejected: __storyforge_x')
})
