import test from 'node:test'
import assert from 'node:assert/strict'
import { normalizeShellHeight, shouldShowOpeningShell } from '../src/utils/cardShellPresentation.js'

test('shows the opening shell only before a conversation has begun', () => {
  assert.equal(
    shouldShowOpeningShell({
      openingUrl: 'https://example.test/home/index.html',
      conversationId: null,
      messageCount: 1,
      isWriting: false,
    }),
    true,
  )
})

test('hides the opening shell once story writing has begun', () => {
  const openingUrl = 'https://example.test/home/index.html'

  assert.equal(shouldShowOpeningShell({ openingUrl, conversationId: 'conv-1', messageCount: 1 }), false)
  assert.equal(shouldShowOpeningShell({ openingUrl, conversationId: null, messageCount: 2 }), false)
  assert.equal(shouldShowOpeningShell({ openingUrl, conversationId: null, messageCount: 1, isWriting: true }), false)
})

test('uses a safe natural iframe height instead of a short fixed pane', () => {
  assert.equal(normalizeShellHeight(812), 812)
  assert.equal(normalizeShellHeight(12), 160)
  assert.equal(normalizeShellHeight('bad'), null)
})
