import test from 'node:test'
import assert from 'node:assert/strict'
import { buildMvuExecuteResultData } from '../src/utils/mvuExecuteResult.js'

// M-04：MVU JS fallback 的 variable_updates 不得写 __storyforge* 保留命名空间。
// 后端 mvu_execute_result 写入边界暂无镜像守卫（另一 owner 补），前端先 fail closed。

test('buildMvuExecuteResultData drops reserved-namespace variable updates', () => {
  const dropped = []
  const payload = buildMvuExecuteResultData(
    {
      request_id: 'req-1',
      variable_updates: {
        hp: 90,
        '主角.生命值': 80,
        __storyforge_card_shell_variables: { 'character:current': {} },
        __storyforge_anything: 1,
      },
      side_effects: [{ kind: 'log' }],
      error: null,
    },
    (keys) => dropped.push(...keys),
  )

  assert.deepEqual(payload.variableUpdates, { hp: 90, '主角.生命值': 80 })
  assert.deepEqual(dropped, ['__storyforge_card_shell_variables', '__storyforge_anything'])
  assert.equal(payload.requestId, 'req-1')
  assert.deepEqual(payload.sideEffects, [{ kind: 'log' }])
  assert.equal(payload.error, null)
})

test('buildMvuExecuteResultData warns when the caller passes no drop callback', () => {
  const warnings = []
  const originalWarn = console.warn
  console.warn = (...args) => warnings.push(args)
  try {
    buildMvuExecuteResultData({
      request_id: 'req-2',
      variable_updates: { __storyforge_card_shell_variables: {} },
    })
  } finally {
    console.warn = originalWarn
  }

  assert.equal(warnings.length, 1)
  assert.match(String(warnings[0][0]), /reserved-namespace/)
  assert.deepEqual(warnings[0][1], ['__storyforge_card_shell_variables'])
})

test('buildMvuExecuteResultData normalizes hostile or missing fields', () => {
  assert.deepEqual(buildMvuExecuteResultData(), {
    requestId: undefined,
    variableUpdates: {},
    sideEffects: [],
    error: null,
  })
  // 非对象/数组的 variable_updates 不得原样透传（后端需要 map）
  assert.deepEqual(buildMvuExecuteResultData({ variable_updates: 'nope' }).variableUpdates, {})
  assert.deepEqual(buildMvuExecuteResultData({ variable_updates: [1, 2] }).variableUpdates, {})
  assert.deepEqual(buildMvuExecuteResultData({ side_effects: 'nope' }).sideEffects, [])
  // 前缀形似但不是保留命名空间的键必须保留
  assert.deepEqual(
    buildMvuExecuteResultData({ variable_updates: { _storyforge_card: 1, hp: 1 } }).variableUpdates,
    { _storyforge_card: 1, hp: 1 },
  )
})
