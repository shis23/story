// 2026-09-01 全量审查补测：reroll 防重入、编辑失败回传、小票拉取失败降级直采。
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

test('handleReroll is a no-op while a generation is already running', async () => {
  const { writing } = setup()
  writing.isWriting = true
  const regenerates = []
  const variants = useMessageVariants({
    regenerateApi: async (...args) => { regenerates.push(args) },
  })

  await variants.handleReroll({ messageId: 'node-1', kind: 'all', hint: null })

  assert.equal(regenerates.length, 0)
  assert.equal(writing.pipeline.state, 'idle')
})

test('handleEditVariant returns false and alerts on backend failure (editor stays open)', async () => {
  const { writing } = setup()
  const alerts = []
  const variants = useMessageVariants({
    editVariantApi: async () => { throw { type: 'validation', message: '变体不可编辑' } },
    alertDialog: async (msg) => alerts.push(msg),
  })

  const ok = await variants.handleEditVariant({ nodeId: 'node-1', newContent: '改写' })

  assert.equal(ok, false)
  assert.equal(alerts.length, 1)
  assert.match(alerts[0], /变体不可编辑/)
  // 未刷新成功路径，正文保持原状
  assert.equal(writing.messages[0].variants[0].content, '正文')
})

test('handleEditVariant returns true on success', async () => {
  const { writing } = setup()
  const edits = []
  const variants = useMessageVariants({
    editVariantApi: async (...args) => { edits.push(args) },
    applyConversation: (conv) => {
      writing.messages = conv.nodes.map((node) => ({
        id: node.id,
        role: 'assistant',
        role_label: 'AI',
        active_variant: node.active_variant,
        variants: node.variants.map((v) => ({
          id: v.id,
          content: v.content,
          display_content: v.display_content ?? v.content,
          status: 'draft',
          provenance: null,
        })),
      }))
    },
    getConversationApi: async () => ({
      id: 'conversation-1',
      nodes: [{
        id: 'node-1',
        active_variant: 0,
        variants: [{ id: 'variant-1', content: '新正文', display_content: '新正文', status: 'Draft', provenance: null }],
      }],
    }),
  })

  const ok = await variants.handleEditVariant({ nodeId: 'node-1', newContent: '新正文' })

  assert.equal(ok, true)
  assert.deepEqual(edits, [['conversation-1', 'node-1', '新正文']])
  assert.equal(writing.messages[0].variants[0].content, '新正文')
})

test('receipt fetch failure falls through to direct accept instead of swallowing the click', async () => {
  const { writing } = setup()
  const accepts = []
  const alerts = []
  const variants = useMessageVariants({
    getActiveTurnReceiptApi: async () => { throw { type: 'pipeline', message: '小票服务不可用' } },
    acceptVariantApi: async (...args) => { accepts.push(args) },
    alertDialog: async (msg) => alerts.push(msg),
  })

  await variants.handleAcceptVariant({ nodeId: 'node-1' })

  // 降级路径：小票失败 → 提示一次 → 直接发起普通采纳
  assert.equal(accepts.length, 1)
  assert.equal(alerts.length, 1)
  assert.match(alerts[0], /小票/)
  assert.equal(writing.pendingReceipt, null)
})
