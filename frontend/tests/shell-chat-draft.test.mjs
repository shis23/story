import test from 'node:test'
import assert from 'node:assert/strict'

import {
  offerShellChatDraft,
  registerShellDraftTarget,
  SHELL_DRAFT_NOT_FILLED,
} from '../src/utils/shellChatDraft.js'

test('a card shell draft needs text and a mounted composer', () => {
  assert.equal(offerShellChatDraft(''), 'empty')
  assert.equal(offerShellChatDraft('   \n'), 'empty')
  assert.equal(offerShellChatDraft(null), 'empty')
  assert.equal(offerShellChatDraft('出发'), 'no-composer')
  for (const outcome of ['empty', 'no-composer', 'busy', 'draft']) {
    assert.ok(SHELL_DRAFT_NOT_FILLED[outcome], outcome)
  }
})

test('the registered composer decides, and only its own unregister clears it', () => {
  const seen = []
  const first = registerShellDraftTarget((text) => {
    seen.push(['first', text])
    return 'drafted'
  })
  assert.equal(offerShellChatDraft('角色：艾琳'), 'drafted')

  // A second composer (route change mounts the new one before the old unmounts)
  // takes over; the stale unregister must not drop it.
  const second = registerShellDraftTarget((text) => {
    seen.push(['second', text])
    return 'busy'
  })
  first()
  assert.equal(offerShellChatDraft(42), 'busy')
  assert.deepEqual(seen, [['first', '角色：艾琳'], ['second', '42']])

  second()
  assert.equal(offerShellChatDraft('出发'), 'no-composer')
})
