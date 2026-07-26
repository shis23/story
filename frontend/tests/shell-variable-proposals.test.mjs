import test from 'node:test'
import assert from 'node:assert/strict'
import {
  previewShellVariableValue,
  resetShellVariableProposalSeq,
  takeShellVariableProposal,
  upsertShellVariableProposal,
} from '../src/utils/shellVariableProposals.js'

test('queues shell writes as pending proposals instead of direct persistence', () => {
  resetShellVariableProposalSeq()
  let list = []
  list = upsertShellVariableProposal(list, { key: 'hp', value: 10 })
  list = upsertShellVariableProposal(list, { key: 'mood', value: '愤怒' })
  assert.equal(list.length, 2)
  assert.deepEqual(list.map((p) => p.key), ['hp', 'mood'])
  assert.equal(list[0].source, 'card-shell')
})

test('same-key spam collapses to the latest value (no confirm-surface flooding)', () => {
  resetShellVariableProposalSeq()
  let list = []
  for (let i = 0; i < 20; i++) {
    list = upsertShellVariableProposal(list, { key: 'hp', value: i })
  }
  assert.equal(list.length, 1)
  assert.equal(list[0].value, 19)
})

test('ignores empty keys and bounds distinct-key floods', () => {
  resetShellVariableProposalSeq()
  let list = upsertShellVariableProposal([], { key: '   ', value: 1 })
  assert.equal(list.length, 0)
  list = upsertShellVariableProposal(list, {})
  assert.equal(list.length, 0)

  for (let i = 0; i < 60; i++) {
    list = upsertShellVariableProposal(list, { key: `k${i}`, value: i })
  }
  assert.equal(list.length, 50)
  // 最老的先被挤掉
  assert.equal(list[0].key, 'k10')
})

test('take removes exactly the confirmed proposal', () => {
  resetShellVariableProposalSeq()
  let list = []
  list = upsertShellVariableProposal(list, { key: 'a', value: 1 })
  list = upsertShellVariableProposal(list, { key: 'b', value: 2 })
  const id = list[0].id

  const { proposal, rest } = takeShellVariableProposal(list, id)
  assert.equal(proposal.key, 'a')
  assert.deepEqual(rest.map((p) => p.key), ['b'])

  const miss = takeShellVariableProposal(rest, 'svp_nope')
  assert.equal(miss.proposal, null)
  assert.equal(miss.rest, rest)
})

test('value preview stays short and readable', () => {
  assert.equal(previewShellVariableValue(null), 'null')
  assert.equal(previewShellVariableValue(undefined), 'null')
  assert.equal(previewShellVariableValue(42), '42')
  assert.equal(previewShellVariableValue({ a: 1 }), '{"a":1}')
  const long = previewShellVariableValue('x'.repeat(200))
  assert.ok(long.length <= 60)
  assert.ok(long.endsWith('…'))
})
