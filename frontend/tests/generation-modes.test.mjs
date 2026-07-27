import test from 'node:test'
import assert from 'node:assert/strict'
import {
  generationModeCatalog,
  generationModeCostLabel,
} from '../src/utils/generationModes.js'

test('writing mode catalog exposes the preflight call estimate for every product mode', () => {
  assert.deepEqual(
    generationModeCatalog.map(({ value }) => value),
    ['continuation', 'duet', 'sequential_crew'],
  )
  for (const mode of generationModeCatalog) {
    assert.match(mode.callEstimate, /次/)
    assert.equal(generationModeCostLabel(mode.value), mode.callEstimate)
  }
  assert.match(generationModeCostLabel('sequential_crew'), /2\+N/)
})

test('unknown mode does not invent a cost estimate', () => {
  assert.equal(generationModeCostLabel('unknown'), '')
})
