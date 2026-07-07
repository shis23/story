import test from 'node:test'
import assert from 'node:assert/strict'
import { buildGreetingOptionsFromDetail } from '../src/utils/campaignGreetingOptions.js'

test('builds greeting options from first message and alternate greetings', () => {
  assert.deepEqual(buildGreetingOptionsFromDetail({
    first_mes: 'Hello.',
    alternate_greetings: ['Alt one.', 'Alt two.'],
  }), [
    { label: '默认', content: 'Hello.' },
    { label: '备选 1', content: 'Alt one.' },
    { label: '备选 2', content: 'Alt two.' },
  ])
})

test('skips empty and duplicate greeting content while preserving source labels', () => {
  assert.deepEqual(buildGreetingOptionsFromDetail({
    first_mes: 'Hello.',
    alternate_greetings: ['', 'Hello.', '  ', 'Alt later.'],
  }), [
    { label: '默认', content: 'Hello.' },
    { label: '备选 4', content: 'Alt later.' },
  ])
})

test('handles missing detail and non-array alternates', () => {
  assert.deepEqual(buildGreetingOptionsFromDetail(null), [])
  assert.deepEqual(buildGreetingOptionsFromDetail({ first_mes: '', alternate_greetings: null }), [])
  assert.deepEqual(buildGreetingOptionsFromDetail({ first_mes: 'Hello.', alternate_greetings: 'Alt' }), [
    { label: '默认', content: 'Hello.' },
  ])
})
