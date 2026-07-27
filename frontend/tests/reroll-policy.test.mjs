import test from 'node:test'
import assert from 'node:assert/strict'
import { rerollPolicy } from '../src/utils/rerollPolicy.js'

test('sequential suffix replay requires both selected and recorded modes to match', () => {
  assert.deepEqual(rerollPolicy('sequential_crew', 'sequential_crew'), {
    editorOnly: false,
    sequentialSuffix: true,
  })
  assert.deepEqual(rerollPolicy('sequential_crew', null), {
    editorOnly: false,
    sequentialSuffix: false,
  })
  assert.deepEqual(rerollPolicy('sequential_crew', 'big_scene'), {
    editorOnly: false,
    sequentialSuffix: false,
  })
})

test('legacy and big-scene paths keep artifact-level rerolls', () => {
  assert.deepEqual(rerollPolicy('big_scene', 'big_scene', true), {
    editorOnly: true,
    sequentialSuffix: false,
  })
  assert.deepEqual(rerollPolicy('big_scene', 'sequential_crew', true), {
    editorOnly: false,
    sequentialSuffix: false,
  })
  assert.deepEqual(rerollPolicy(null, null, true), {
    editorOnly: false,
    sequentialSuffix: false,
  })
})
