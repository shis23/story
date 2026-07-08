import test from 'node:test'
import assert from 'node:assert/strict'
import { subagentRolesFromProvenance } from '../src/utils/pipelineTrace.js'

test('returns empty array when provenance is null/undefined', () => {
  assert.deepEqual(subagentRolesFromProvenance(null), [])
  assert.deepEqual(subagentRolesFromProvenance(undefined), [])
})

test('returns empty array when provenance has no subagent_results', () => {
  assert.deepEqual(subagentRolesFromProvenance({ seed: 1 }), [])
  assert.deepEqual(subagentRolesFromProvenance({ seed: 2, subagent_results: null }), [])
})

test('returns empty array when subagent_results is empty', () => {
  assert.deepEqual(subagentRolesFromProvenance({ seed: 3, subagent_results: [] }), [])
})

test('filters out subagent_results without character_id', () => {
  const result = subagentRolesFromProvenance({
    seed: 10,
    subagent_results: [
      { character_id: 'a1', display_name: 'Agent One' },
      { display_name: 'NoIdAgent' },  // no character_id → filtered out
      { character_id: 'a3', display_name: 'Agent Three' },
    ],
  })
  assert.equal(result.length, 2)
  assert.deepEqual(result[0], { id: 'a1', label: 'Agent One' })
  assert.deepEqual(result[1], { id: 'a3', label: 'Agent Three' })
})

test('falls back to character_id when display_name is absent', () => {
  const result = subagentRolesFromProvenance({
    seed: 20,
    subagent_results: [
      { character_id: 'a1', display_name: 'Visible' },
      { character_id: 'a2' },  // no display_name → label = character_id
    ],
  })
  assert.equal(result.length, 2)
  assert.deepEqual(result[1], { id: 'a2', label: 'a2' })
})

test('preserves ordering of subagent_results', () => {
  const result = subagentRolesFromProvenance({
    seed: 30,
    subagent_results: [
      { character_id: 'first', display_name: 'First' },
      { character_id: 'second', display_name: 'Second' },
      { character_id: 'third', display_name: 'Third' },
    ],
  })
  assert.equal(result.length, 3)
  assert.equal(result[0].id, 'first')
  assert.equal(result[1].id, 'second')
  assert.equal(result[2].id, 'third')
})

test('handles display_name empty string same as absent', () => {
  const result = subagentRolesFromProvenance({
    seed: 40,
    subagent_results: [
      { character_id: 'a1', display_name: '' },
    ],
  })
  assert.equal(result.length, 1)
  // Empty string is falsy in JS, so label falls back to character_id
  assert.equal(result[0].label, 'a1')
})
