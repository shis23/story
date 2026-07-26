import test from 'node:test'
import assert from 'node:assert/strict'
import {
  CARD_SHELL_VARIABLES_KEY,
  mergeCardShellVariables,
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
