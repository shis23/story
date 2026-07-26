import test from 'node:test'
import assert from 'node:assert/strict'
import {
  buildOpeningChatSeed,
  buildOpeningChatSwipes,
  resolveOpeningChatSelection,
} from '../src/utils/cardShellOpeningChat.js'

test('builds ordered swipes from greeting options without empty or duplicate text', () => {
  assert.deepEqual(
    buildOpeningChatSwipes([
      { label: '默认', content: 'first' },
      { label: '备选 1', content: 'alt-a' },
      { label: 'dup', content: 'first' },
      { label: 'empty', content: '   ' },
      'alt-b',
    ]),
    ['first', 'alt-a', 'alt-b'],
  )
})

test('seeds chat[0] with swipes and a forced unmatched swipe_id', () => {
  const seed = buildOpeningChatSeed(
    [
      { content: 'first' },
      { content: 'alt-1' },
      { content: 'alt-2' },
    ],
    { selectedIndex: 1, name: '命定之诗' },
  )

  assert.equal(seed.name, '命定之诗')
  assert.equal(seed.is_user, false)
  assert.equal(seed.swipe_id, -1)
  assert.deepEqual(seed.swipes, ['first', 'alt-1', 'alt-2'])
  assert.equal(seed.mes, 'alt-1')
  assert.equal(seed.message, 'alt-1')
})

test('resolves a shell swipe selection back onto greeting options', () => {
  const options = [
    { content: 'first' },
    { content: 'alt-1' },
    { content: 'alt-2' },
  ]

  assert.deepEqual(
    resolveOpeningChatSelection({ swipe_id: 2, mes: 'ignored' }, options),
    { greetingIndex: 2, content: 'alt-2' },
  )
  assert.deepEqual(
    resolveOpeningChatSelection({ swipe_id: 99, mes: 'alt-1' }, options),
    { greetingIndex: 1, content: 'alt-1' },
  )
  assert.deepEqual(
    resolveOpeningChatSelection({ swipe_id: null, mes: 'custom text' }, options),
    { greetingIndex: null, content: 'custom text' },
  )
})
