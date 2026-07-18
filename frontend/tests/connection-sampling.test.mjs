import assert from 'node:assert/strict'
import test from 'node:test'

import {
  normalizeOptionalMaxTokens,
  normalizeReasoningMode,
} from '../src/utils/connectionSampling.js'

test('empty max_tokens delegates output sizing to the provider', () => {
  assert.equal(normalizeOptionalMaxTokens(null), null)
  assert.equal(normalizeOptionalMaxTokens(''), null)
  assert.equal(normalizeOptionalMaxTokens('  '), null)
})

test('a positive integer is an explicit max_tokens cap', () => {
  assert.equal(normalizeOptionalMaxTokens('384000'), 384000)
  assert.equal(normalizeOptionalMaxTokens(4096), 4096)
})

test('max_tokens rejects zero, fractions, and non-numeric values', () => {
  for (const value of ['0', '-1', '1.5', 'many']) {
    assert.throws(() => normalizeOptionalMaxTokens(value), /positive integer/)
  }
})

test('reasoning mode defaults to provider-compatible disabled and accepts explicit capture modes', () => {
  assert.equal(normalizeReasoningMode(null), 'disabled')
  assert.equal(normalizeReasoningMode('prompted'), 'prompted')
  assert.equal(normalizeReasoningMode('native'), 'native')
  assert.equal(normalizeReasoningMode('disabled'), 'disabled')
})

test('reasoning mode rejects unknown values', () => {
  assert.throws(() => normalizeReasoningMode('mystery'), /reasoning mode/)
})
