import test from 'node:test'
import assert from 'node:assert/strict'
import {
  buildOpeningChatSeed,
  buildOpeningChatSwipes,
  resolveOpeningChatSelection,
  rewriteOpeningMessages,
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

test('rewrites the single opening assistant message across variant shapes', () => {
  // 无 variants：直接改 content/display_content
  const flat = rewriteOpeningMessages(
    [{ id: 'n1', role: 'assistant', content: 'first', display_content: 'first' }],
    'scene-2',
  )
  assert.equal(flat.length, 1)
  assert.equal(flat[0].content, 'scene-2')
  assert.equal(flat[0].display_content, 'scene-2')

  // 有 variants：只改 active variant，其余保持
  const withVariants = rewriteOpeningMessages(
    [{
      id: 'n1',
      role: 'assistant',
      active_variant: 1,
      variants: [
        { content: 'v0', display_content: 'v0' },
        { content: 'v1', display_content: 'v1' },
      ],
    }],
    'scene-2',
  )
  assert.equal(withVariants[0].variants[0].content, 'v0')
  assert.equal(withVariants[0].variants[1].content, 'scene-2')
  assert.equal(withVariants[0].variants[1].display_content, 'scene-2')
})

test('refuses to rewrite when conversation is past the opening state', () => {
  // 多条消息 / 首条非 assistant / 空列表：一律返回 null，禁止触碰历史
  assert.equal(
    rewriteOpeningMessages(
      [
        { role: 'assistant', content: 'opening' },
        { role: 'user', content: 'next turn' },
      ],
      'scene-2',
    ),
    null,
  )
  assert.equal(rewriteOpeningMessages([{ role: 'user', content: 'hi' }], 'x'), null)
  assert.equal(rewriteOpeningMessages([], 'x'), null)
  assert.equal(rewriteOpeningMessages(null, 'x'), null)
})
