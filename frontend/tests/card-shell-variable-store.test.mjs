import test from 'node:test'
import assert from 'node:assert/strict'
import {
  CARD_SHELL_VARIABLES_KEY,
  mergeCardShellVariables,
  patchCardShellVariables,
  readCardShellVariables,
  selectorKey,
  updateCardShellVariables,
} from '../src/utils/cardShellVariableStore.js'

test('reads the persisted shell snapshot from Campaign variables', () => {
  const snapshot = readCardShellVariables([
    { key: 'story_clock', value: 'day 1' },
    {
      key: CARD_SHELL_VARIABLES_KEY,
      value: { message: { stat_data: { hp: 75 } }, character: { status_theme_id: 'ivory' } },
    },
  ])

  assert.deepEqual(snapshot, {
    'message:current': { stat_data: { hp: 75 } },
    'character:current': { status_theme_id: 'ivory' },
  })
})

test('keeps distinct selector identities while preserving sibling values', () => {
  assert.equal(selectorKey({ type: 'message', message_id: 9 }), 'message:9')
  assert.equal(selectorKey({ type: 'message', message_id: 10 }), 'message:10')
  assert.equal(selectorKey({ type: 'character' }), 'character:current')
  assert.equal(selectorKey(null), 'local:current')

  assert.deepEqual(
    updateCardShellVariables(
      { 'character:current': { status_theme_id: 'ivory' }, 'message:9': { stat_data: { hp: 10 } } },
      { type: 'message', message_id: 9 },
      { stat_data: { hp: 99 } },
    ),
    { 'character:current': { status_theme_id: 'ivory' }, 'message:9': { stat_data: { hp: 99 } } },
  )
})

test('does not overwrite one message selector when another message changes', () => {
  const afterFirst = updateCardShellVariables({}, { type: 'message', message_id: 9 }, { stat_data: { hp: 10 } })
  const afterSecond = updateCardShellVariables(afterFirst, { type: 'message', message_id: 10 }, { stat_data: { hp: 20 } })

  assert.deepEqual(afterSecond, {
    'message:9': { stat_data: { hp: 10 } },
    'message:10': { stat_data: { hp: 20 } },
  })
})

test('merges concurrent patches for distinct fields of the same selector', () => {
  const afterFirstHost = mergeCardShellVariables(
    { 'character:current': { status_theme_id: 'indigo' } },
    { type: 'character' },
    { map_drawings: [{ id: 'line-1' }] },
  )
  const afterSecondHost = mergeCardShellVariables(
    afterFirstHost,
    { type: 'character' },
    { map_markers: [{ id: 'marker-1' }] },
  )

  assert.deepEqual(afterSecondHost, {
    'character:current': {
      status_theme_id: 'indigo',
      map_drawings: [{ id: 'line-1' }],
      map_markers: [{ id: 'marker-1' }],
    },
  })
})

test('applies key-level patches without erasing keys the shell never saw (M2)', () => {
  // 开场壳先写了主题键；状态壳基于过期快照（没见过主题键）发 patch —
  // 主题键必须存活，只有 patch 声明的键被改/删。
  const afterOpening = mergeCardShellVariables(
    {},
    { type: 'character' },
    { status_theme_id: 'parchment' },
  )
  const afterStatusPatch = patchCardShellVariables(
    afterOpening,
    { type: 'character' },
    { sets: { stat_data: { hp: 42 } }, deletes: [] },
  )

  assert.deepEqual(afterStatusPatch, {
    'character:current': {
      status_theme_id: 'parchment',
      stat_data: { hp: 42 },
    },
  })

  const afterDelete = patchCardShellVariables(
    afterStatusPatch,
    { type: 'character' },
    { sets: {}, deletes: ['stat_data'] },
  )
  assert.deepEqual(afterDelete, {
    'character:current': { status_theme_id: 'parchment' },
  })
})

test('patch tolerates malformed payloads', () => {
  const base = { 'character:current': { keep: 1 } }
  assert.deepEqual(
    patchCardShellVariables(base, { type: 'character' }, null),
    { 'character:current': { keep: 1 } },
  )
  assert.deepEqual(
    patchCardShellVariables(base, { type: 'character' }, { sets: 'junk', deletes: [7, 'keep'] }),
    { 'character:current': {} },
  )
})

test('serializes same-campaign mutations and survives a failed predecessor (L2)', async () => {
  const { enqueueCardShellVariableMutation } = await import('../src/utils/cardShellVariableStore.js')
  const order = []
  let releaseFirst
  const firstGate = new Promise((resolve) => { releaseFirst = resolve })

  // 同 campaign：第二个 mutation 必须等第一个完成（即便第二个先就绪）
  const first = enqueueCardShellVariableMutation('camp-a', async () => {
    order.push('first:start')
    await firstGate
    order.push('first:end')
    return 1
  })
  const second = enqueueCardShellVariableMutation('camp-a', async () => {
    order.push('second')
    return 2
  })
  // 异 campaign：不被 camp-a 的门闩阻塞
  const other = enqueueCardShellVariableMutation('camp-b', async () => {
    order.push('other')
    return 3
  })

  assert.equal(await other, 3)
  assert.deepEqual(order, ['first:start', 'other'])

  releaseFirst()
  assert.equal(await first, 1)
  assert.equal(await second, 2)
  assert.deepEqual(order, ['first:start', 'other', 'first:end', 'second'])

  // 前驱失败不阻塞后继
  const failing = enqueueCardShellVariableMutation('camp-a', async () => {
    throw new Error('boom')
  })
  await assert.rejects(failing, /boom/)
  assert.equal(await enqueueCardShellVariableMutation('camp-a', async () => 'after-failure'), 'after-failure')
})
