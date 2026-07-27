import test from 'node:test'
import assert from 'node:assert/strict'
import { createPinia, setActivePinia } from 'pinia'
import { useWritingStore } from '../../src/stores/writing.js'
import { useCampaignStore } from '../../src/stores/campaign.js'
import { useMessageVariants } from '../../src/composables/useMessageVariants.js'

function setup() {
  setActivePinia(createPinia())
  const writing = useWritingStore()
  const campaign = useCampaignStore()
  campaign.activeCampaign = { id: 'campaign-1', name: '测试档案' }
  campaign.currentConversationId = 'conversation-1'
  writing.messages = [{
    id: 'node-1',
    role: 'assistant',
    active_variant: 0,
    variants: [{ id: 'variant-1', status: 'draft', content: '正文' }],
  }]
  return { writing, campaign }
}

test('campaign accept opens receipt before calling backend and later submits selection', async () => {
  const { writing } = setup()
  const accepts = []
  const variants = useMessageVariants({
    getActiveTurnReceiptApi: async () => ({
      turn_id: 'turn-1',
      ready: true,
      derivation_failed: false,
      items: [{
        mutation_index: 3,
        kind: 'chronicle',
        title: '本轮纪要',
        detail: '发生了一件事',
        selected_by_default: true,
      }],
    }),
    acceptVariantApi: async (...args) => accepts.push(args),
  })

  await variants.handleAcceptVariant({ nodeId: 'node-1' })
  assert.equal(accepts.length, 0)
  assert.equal(writing.pendingReceipt.nodeId, 'node-1')

  await variants.handleAcceptVariant({
    nodeId: 'node-1',
    selectedMutationIndices: [3],
  })
  assert.deepEqual(accepts, [['conversation-1', 'node-1', false, [3]]])
  assert.equal(writing.pendingReceipt, null)
})

test('postprocess retry replaces failed receipt without rewriting the draft', async () => {
  const { writing } = setup()
  writing.openTurnReceipt('node-1', {
    turn_id: 'turn-1',
    derivation_failed: true,
    can_retry: true,
    items: [],
  })
  const variants = useMessageVariants({
    retryActiveTurnPostprocessApi: async () => ({
      turn_id: 'turn-1',
      derivation_failed: false,
      can_retry: false,
      ready: true,
      items: [{
        mutation_index: 0,
        kind: 'chronicle',
        title: '新纪要',
        detail: '重试成功',
        selected_by_default: true,
      }],
    }),
  })

  await variants.handleRetryPostprocess({ nodeId: 'node-1' })

  assert.equal(writing.pendingReceipt.derivation_failed, false)
  assert.equal(writing.pendingReceipt.items[0].title, '新纪要')
})

test('campaign whole reroll forwards the selected generation mode', async () => {
  const { writing } = setup()
  writing.setGenerationMode('sequential_crew')
  const requests = []
  const variants = useMessageVariants({
    regenerateApi: async (request) => requests.push(request),
    getConversationApi: async () => null,
  })

  await variants.handleReroll({ messageId: 'node-1', kind: 'all', hint: '收紧节奏' })

  assert.equal(requests.length, 1)
  assert.equal(requests[0].generationMode, 'sequential_crew')
  assert.deepEqual(requests[0].targets, [])
})
