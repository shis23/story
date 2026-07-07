import test from 'node:test'
import assert from 'node:assert/strict'
import {
  acceptTypedPatchFlow,
  dismissTypedPatchFlow,
  explainGenerationFlow,
  proposeRepairsFlow,
  refreshTypedPatchesFlow,
  sortHealthIssues,
} from '../src/utils/metaPanelFlow.js'

test('sortHealthIssues puts errors before warnings without mutating input', () => {
  const issues = [
    { id: 'w1', severity: 'warning' },
    { id: 'e1', severity: 'error' },
    { id: 'w2', severity: 'warning' },
  ]

  const sorted = sortHealthIssues(issues)

  assert.deepEqual(sorted.map((issue) => issue.id), ['e1', 'w1', 'w2'])
  assert.deepEqual(issues.map((issue) => issue.id), ['w1', 'e1', 'w2'])
})

test('proposeRepairsFlow marks typed patches stale from preview results', async () => {
  const calls = []
  const patches = await proposeRepairsFlow({
    campaignId: 'camp-1',
    proposeCampaignRepairs: async (campaignId) => {
      calls.push(['propose', campaignId])
      return [{ id: 'patch-ok' }, { id: 'patch-stale' }]
    },
    previewTypedPatch: async (patchId, campaignId) => {
      calls.push(['preview', patchId, campaignId])
      return { stale: patchId === 'patch-stale', diff: [{ path: patchId }] }
    },
  })

  assert.deepEqual(calls, [
    ['propose', 'camp-1'],
    ['preview', 'patch-ok', 'camp-1'],
    ['preview', 'patch-stale', 'camp-1'],
  ])
  assert.deepEqual(patches, [
    { id: 'patch-ok', _stale: false, _previewDiff: [{ path: 'patch-ok' }] },
    { id: 'patch-stale', _stale: true, _previewDiff: [{ path: 'patch-stale' }] },
  ])
})

test('refreshTypedPatchesFlow tolerates preview failures per patch', async () => {
  const patches = await refreshTypedPatchesFlow({
    campaignId: 'camp-1',
    listTypedPatches: async () => [{ id: 'patch-a' }, { id: 'patch-b' }],
    previewTypedPatch: async (patchId) => {
      if (patchId === 'patch-b') throw new Error('preview failed')
      return { stale: true }
    },
  })

  assert.deepEqual(patches, [
    { id: 'patch-a', _stale: true },
    { id: 'patch-b', _stale: false },
  ])
})

test('acceptTypedPatchFlow removes accepted patch and refreshes health', async () => {
  const calls = []
  const result = await acceptTypedPatchFlow({
    campaignId: 'camp-1',
    patchId: 'patch-1',
    patches: [{ id: 'patch-1' }, { id: 'patch-2' }],
    acceptTypedPatch: async (patchId, campaignId) => calls.push(['accept', patchId, campaignId]),
    refreshHealth: async () => {
      calls.push(['health'])
      return sortHealthIssues([
        { id: 'warning', severity: 'warning' },
        { id: 'error', severity: 'error' },
      ])
    },
  })

  assert.deepEqual(calls, [
    ['accept', 'patch-1', 'camp-1'],
    ['health'],
  ])
  assert.deepEqual(result.patches, [{ id: 'patch-2' }])
  assert.deepEqual(result.healthIssues.map((issue) => issue.id), ['error', 'warning'])
})

test('acceptTypedPatchFlow keeps accepted patch removed when health refresh fails', async () => {
  const result = await acceptTypedPatchFlow({
    campaignId: 'camp-1',
    patchId: 'patch-1',
    patches: [{ id: 'patch-1' }, { id: 'patch-2' }],
    acceptTypedPatch: async () => {},
    refreshHealth: async () => {
      throw new Error('health offline')
    },
  })

  assert.deepEqual(result.patches, [{ id: 'patch-2' }])
  assert.equal(result.healthIssues, undefined)
  assert.equal(result.healthError.message, 'health offline')
})

test('dismissTypedPatchFlow removes dismissed patch without health refresh', async () => {
  const calls = []
  const patches = await dismissTypedPatchFlow({
    patchId: 'patch-1',
    patches: [{ id: 'patch-1' }, { id: 'patch-2' }],
    dismissTypedPatch: async (patchId) => calls.push(['dismiss', patchId]),
  })

  assert.deepEqual(calls, [['dismiss', 'patch-1']])
  assert.deepEqual(patches, [{ id: 'patch-2' }])
})

test('explainGenerationFlow calls backend only when provenance node is available', async () => {
  const calls = []
  assert.equal(await explainGenerationFlow({
    lastConversationNode: null,
    explainGeneration: async () => {
      throw new Error('should not be called')
    },
  }), null)

  const result = await explainGenerationFlow({
    lastConversationNode: { conversation_id: 'conv-1', node_id: 'node-2' },
    explainGeneration: async (conversationId, nodeId) => {
      calls.push([conversationId, nodeId])
      return { scene_brief: 'trace' }
    },
  })

  assert.deepEqual(calls, [['conv-1', 'node-2']])
  assert.deepEqual(result, { scene_brief: 'trace' })
})
