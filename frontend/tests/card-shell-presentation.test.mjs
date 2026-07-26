import test from 'node:test'
import assert from 'node:assert/strict'
import {
  getOpeningShellPresentation,
  normalizeShellHeight,
  resolveCardShellManifestTarget,
  shouldShowEmptyWritingState,
  shouldShowOpeningShell,
} from '../src/utils/cardShellPresentation.js'

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

test('gives the opening shell a concrete viewport-sized height without auto-resizing it smaller', () => {
  assert.deepEqual(getOpeningShellPresentation(1000), {
    height: '840px',
    autoHeight: false,
  })
  // Short windows still keep a usable multi-step setup surface.
  assert.deepEqual(getOpeningShellPresentation(500), {
    height: '520px',
    autoHeight: false,
  })
  // Tall windows cap near one reading pane, not an endless iframe.
  assert.deepEqual(getOpeningShellPresentation(1600), {
    height: '900px',
    autoHeight: false,
  })
})

test('keeps a fresh campaign opening out of the generic empty home state', () => {
  assert.equal(
    shouldShowEmptyWritingState({
      showOpening: true,
      messageCount: 0,
      isWriting: false,
      greetingCount: 0,
    }),
    false,
  )
  assert.equal(
    shouldShowEmptyWritingState({
      showOpening: false,
      messageCount: 0,
      isWriting: false,
      greetingCount: 0,
    }),
    true,
  )
})

test('uses the active Campaign card instead of a stale legacy character for its opening shell', () => {
  assert.deepEqual(
    resolveCardShellManifestTarget({
      activeCampaign: { id: 'campaign-new', card_id: 'card-new' },
      activeChar: { id: 'character-old' },
      activeCharDetail: { id: 'character-old-detail' },
    }),
    { kind: 'campaign-card', cardId: 'card-new' },
  )
  assert.equal(
    resolveCardShellManifestTarget({
      activeCampaign: { id: 'campaign-without-card' },
      activeChar: { id: 'character-old' },
    }),
    null,
  )
})
