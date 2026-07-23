import test from 'node:test'
import assert from 'node:assert/strict'
import { normalizeShellHeight, shouldShowOpeningShell } from '../src/utils/cardShellPresentation.js'

test('keeps the opening shell available for the first story message', () => {
  assert.equal(
    shouldShowOpeningShell({
      openingUrl: 'https://example.test/home/index.html',
      conversationId: 'conversation-created-with-opening',
      openingArmed: true,
      messageCount: 1,
      isWriting: false,
    }),
    true,
  )
})

test('hides the opening shell once the story moves past its first message', () => {
  const openingUrl = 'https://example.test/home/index.html'

  assert.equal(shouldShowOpeningShell({ openingUrl, openingArmed: true, messageCount: 2 }), false)
  assert.equal(shouldShowOpeningShell({ openingUrl, openingArmed: true, messageCount: 1, isWriting: true }), false)
  assert.equal(shouldShowOpeningShell({ openingUrl, openingArmed: false, messageCount: 1 }), false)
})

test('uses a safe natural iframe height instead of a short fixed pane', () => {
  assert.equal(normalizeShellHeight(812), 812)
  assert.equal(normalizeShellHeight(12), 160)
  assert.equal(normalizeShellHeight('bad'), null)
})
