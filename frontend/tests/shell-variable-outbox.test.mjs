import test from 'node:test'
import assert from 'node:assert/strict'
import {
  toJsonValue,
  persistShellVariableWrite,
  createVariableWriteAudit,
} from '../src/utils/shellVariableOutbox.js'

test('toJsonValue serializes plain data', () => {
  assert.equal(toJsonValue('a'), 'a')
  assert.equal(toJsonValue(1), 1)
  assert.deepEqual(toJsonValue({ x: 1 }), { x: 1 })
  assert.equal(toJsonValue(undefined), null)
})

test('persistShellVariableWrite writes campaign by default', async () => {
  const calls = []
  const res = await persistShellVariableWrite({
    campaignId: 'c1',
    instanceId: null,
    key: 'hp',
    value: 12,
    setCampaignVariable: async (cid, key, value) => {
      calls.push(['camp', cid, key, value])
    },
    setCharacterVariable: async () => {
      throw new Error('should not instance')
    },
  })
  assert.equal(res.ok, true)
  assert.equal(res.scope, 'campaign')
  assert.deepEqual(calls, [['camp', 'c1', 'hp', 12]])
})

test('persistShellVariableWrite honors instance: prefix', async () => {
  const calls = []
  const res = await persistShellVariableWrite({
    campaignId: 'c1',
    instanceId: 'fallback',
    key: 'instance:inst-9:mood',
    value: 'calm',
    setCampaignVariable: async () => {
      throw new Error('should not camp')
    },
    setCharacterVariable: async (cid, iid, key, value) => {
      calls.push([cid, iid, key, value])
    },
  })
  assert.equal(res.ok, true)
  assert.equal(res.scope, 'instance')
  assert.deepEqual(calls, [['c1', 'inst-9', 'mood', 'calm']])
})

test('persistShellVariableWrite fails without campaign', async () => {
  const res = await persistShellVariableWrite({
    campaignId: null,
    key: 'x',
    value: 1,
    setCampaignVariable: async () => {},
    setCharacterVariable: async () => {},
  })
  assert.equal(res.ok, false)
  assert.match(res.error, /no active campaign/)
})

test('createVariableWriteAudit rings', () => {
  const audit = createVariableWriteAudit(2)
  audit.push({ key: 'a' })
  audit.push({ key: 'b' })
  audit.push({ key: 'c' })
  assert.deepEqual(
    audit.list().map((x) => x.key),
    ['c', 'b'],
  )
})
