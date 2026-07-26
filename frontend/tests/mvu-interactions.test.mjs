import test from 'node:test'
import assert from 'node:assert/strict'
import {
  evaluateMvuValueExpr,
  flattenInteractionActions,
  planMvuInteraction,
} from '../src/utils/mvuInteractions.js'

test('flattens multi actions recursively with a depth cap', () => {
  const flat = flattenInteractionActions([
    { kind: 'modify_variable', key: 'hp', value_expr: '+5' },
    {
      kind: 'multi',
      actions: [
        { kind: 'trigger_next_turn', hint: '发起攻击' },
        { kind: 'multi', actions: [{ kind: 'modify_variable', key: 'mp', value_expr: '-3' }] },
      ],
    },
  ])
  assert.deepEqual(flat.map((a) => a.kind), [
    'modify_variable',
    'trigger_next_turn',
    'modify_variable',
  ])

  // 自引用 multi 不允许无限展开
  const cyclic = { kind: 'multi', actions: [] }
  cyclic.actions.push(cyclic)
  assert.deepEqual(flattenInteractionActions([cyclic]), [])
})

test('evaluates value expressions: delta, literal, bare word, unsupported JS', () => {
  assert.deepEqual(evaluateMvuValueExpr('+5', 40), { ok: true, value: 45 })
  assert.deepEqual(evaluateMvuValueExpr('+5', undefined), { ok: true, value: 5 })
  assert.deepEqual(evaluateMvuValueExpr('-3', 40), { ok: true, value: 37 })
  assert.deepEqual(evaluateMvuValueExpr('-3', '战斗中'), { ok: true, value: -3 })
  assert.deepEqual(evaluateMvuValueExpr('100', 40), { ok: true, value: 100 })
  assert.deepEqual(evaluateMvuValueExpr('"重伤"', null), { ok: true, value: '重伤' })
  assert.deepEqual(evaluateMvuValueExpr('true', null), { ok: true, value: true })
  assert.deepEqual(evaluateMvuValueExpr('战斗中', null), { ok: true, value: '战斗中' })

  assert.equal(evaluateMvuValueExpr('getvar("hp") * 2', 40).ok, false)
  assert.equal(evaluateMvuValueExpr('Math.max(0, hp - 5)', 40).ok, false)
  assert.equal(evaluateMvuValueExpr('', 40).ok, false)
})

test('evaluates the canonical analyzer "<key> ± N" form as a delta, never a string', () => {
  // 分析器一号示例格式（mvu_analyzer.rs 的 "value_expr": "hp - 10"）
  assert.deepEqual(evaluateMvuValueExpr('hp - 10', 100, 'hp'), { ok: true, value: 90 })
  assert.deepEqual(evaluateMvuValueExpr('hp + 5', undefined, 'hp'), { ok: true, value: 5 })
  assert.deepEqual(evaluateMvuValueExpr('好感度 + 5', 10, '好感度'), { ok: true, value: 15 })

  // 引用其他变量 / 无目标 key：拒绝，绝不落成字符串
  assert.equal(evaluateMvuValueExpr('mp - 10', 100, 'hp').ok, false)
  assert.equal(evaluateMvuValueExpr('hp - 10', 100).ok, false)

  // 算术混排的表达式不得当字面量（防数值变量被写成垃圾字符串）
  assert.equal(evaluateMvuValueExpr('hp * 2', 100, 'hp').ok, false)
  assert.equal(evaluateMvuValueExpr('攻击力 / 2', 100, 'hp').ok, false)
  assert.equal(evaluateMvuValueExpr('a => a + 1', 100, 'hp').ok, false)

  // CJK 内连字符的裸词仍是合法字符串字面量
  assert.deepEqual(evaluateMvuValueExpr('重伤-濒死', null, 'state'), { ok: true, value: '重伤-濒死' })
})

test('plans a mapping into writes, hints, and skipped entries', () => {
  const plan = planMvuInteraction(
    {
      element_label: '攻击按钮',
      actions: [
        { kind: 'modify_variable', key: 'hp', value_expr: '-10' },
        { kind: 'modify_variable', key: 'mp', value_expr: 'mp - 5' },
        { kind: 'modify_variable', key: 'state', value_expr: '战斗中' },
        { kind: 'modify_variable', key: 'luck', value_expr: 'Math.random()' },
        { kind: 'trigger_next_turn', hint: '主角发起攻击' },
        { kind: 'run_original_js', js_snippet: 'doAttack()', description: '原始攻击逻辑' },
      ],
    },
    { variables: [{ key: 'hp', value: 80 }, { key: 'mp', value: 30 }] },
  )

  assert.equal(plan.label, '攻击按钮')
  assert.deepEqual(plan.writes, [
    { key: 'hp', value: 70 },
    { key: 'mp', value: 25 },
    { key: 'state', value: '战斗中' },
  ])
  assert.deepEqual(plan.hints, ['主角发起攻击'])
  assert.equal(plan.skipped.length, 2)
  assert.equal(plan.skipped[0].kind, 'modify_variable')
  assert.equal(plan.skipped[1].kind, 'run_original_js')
})

test('plans across key notations: resolves stored key and rewrites target', () => {
  // 存量变量以旧记法入库（stat_data. 前缀），action 键是归一化点记法：
  // 取值要跨记法命中，写回要用已存储的键（不产生第二个变量）
  const legacyStored = planMvuInteraction(
    {
      element_label: '受击',
      actions: [{ kind: 'modify_variable', key: '主角.hp', value_expr: '-10' }],
    },
    { variables: [{ key: 'stat_data.主角.hp', value: 80 }] },
  )
  assert.deepEqual(legacyStored.writes, [{ key: 'stat_data.主角.hp', value: 70 }])

  // 反向：action 键是旧记法，变量已是归一化键
  const legacyAction = planMvuInteraction(
    {
      element_label: '受击',
      actions: [{ kind: 'modify_variable', key: 'stat_data.主角.hp', value_expr: '-10' }],
    },
    { variables: [{ key: '主角.hp', value: 80 }] },
  )
  assert.deepEqual(legacyAction.writes, [{ key: '主角.hp', value: 70 }])

  // 无现有变量：新变量用归一化键落盘
  const fresh = planMvuInteraction(
    {
      element_label: '标记',
      actions: [{ kind: 'modify_variable', key: '/世界/天气', value_expr: '雨' }],
    },
    { variables: [] },
  )
  assert.deepEqual(fresh.writes, [{ key: '世界.天气', value: '雨' }])
})

test('keyed delta expr matches target across notations', () => {
  // key 与表达式前导标识符记法不同但归一后相同 → 按增量解释
  assert.deepEqual(evaluateMvuValueExpr('stat_data.hp - 10', 80, 'hp'), { ok: true, value: 70 })
  assert.deepEqual(evaluateMvuValueExpr('hp + 5', 80, 'stat_data.hp'), { ok: true, value: 85 })
  // 归一后仍不同 → 拒绝（引用其他变量）
  assert.equal(evaluateMvuValueExpr('stat_data.mp - 10', 80, 'hp').ok, false)
})

test('plans an empty mapping safely', () => {
  const plan = planMvuInteraction(null, {})
  assert.deepEqual(plan.writes, [])
  assert.deepEqual(plan.hints, [])
  assert.deepEqual(plan.skipped, [])
})
