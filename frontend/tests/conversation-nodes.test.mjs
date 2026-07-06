import test from 'node:test'
import assert from 'node:assert/strict'
import { findLastAssistantConversationNode } from '../src/utils/conversationNodes.js'

test('finds the latest assistant message node for Meta provenance lookup', () => {
  const node = findLastAssistantConversationNode([
    { id: 'u1', role: 'user' },
    { id: 'a1', role: 'assistant', active_variant: 0, variants: [{ provenance: { seed: 1 } }] },
    { id: 'u2', role: 'user' },
    { id: 'a2', role: 'assistant', active_variant: 0, variants: [{ provenance: { seed: 2 } }] },
  ], 'conv-1')

  assert.deepEqual(node, {
    conversation_id: 'conv-1',
    node_id: 'a2',
  })
})

test('ignores streaming placeholders and missing conversation ids', () => {
  assert.equal(findLastAssistantConversationNode([
    { id: 'editor-streaming', role: 'assistant', active_variant: 0, variants: [{ provenance: { seed: 1 } }] },
  ], 'conv-1'), null)

  assert.equal(findLastAssistantConversationNode([
    { id: 'a1', role: 'assistant', active_variant: 0, variants: [{ provenance: { seed: 1 } }] },
  ], null), null)
})

test('ignores assistant messages without provenance', () => {
  assert.equal(findLastAssistantConversationNode([
    { id: 'opening', role: 'assistant', active_variant: 0, variants: [{ provenance: null }] },
  ], 'conv-1'), null)
})
