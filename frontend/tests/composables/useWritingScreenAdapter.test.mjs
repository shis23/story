import test from 'node:test'
import assert from 'node:assert/strict'
import { createPinia, setActivePinia } from 'pinia'
import { useWritingStore } from '../../src/stores/writing.js'
import { useCampaignStore } from '../../src/stores/campaign.js'
import { useWritingScreenAdapter } from '../../src/adapter/useWritingScreenAdapter.js'

function setupStores() {
  setActivePinia(createPinia())
  const writing = useWritingStore()
  const campaign = useCampaignStore()
  return { writing, campaign }
}

test('writing adapter maps empty none-mode props', () => {
  setupStores()
  const fakeContent = { name: 'FakeRichContent' }
  const { screenProps } = useWritingScreenAdapter({ contentComponent: fakeContent })
  assert.equal(screenProps.value.composerDisabled, true)
  assert.match(screenProps.value.composerPlaceholder, /导入角色卡|Campaign/)
  assert.equal(screenProps.value.messages.length, 0)
  assert.equal(screenProps.value.canBranch, false)
  assert.equal(screenProps.value.contentComponent, fakeContent)
})

test('writing adapter enables branch only in campaign mode with conversation', () => {
  const { writing, campaign } = setupStores()
  campaign.activeCampaign = { id: 'c1', name: '风起之地' }
  campaign.currentConversationId = 'conv-1'
  writing.messages = [
    {
      id: 'm1',
      role: 'assistant',
      role_label: 'AI',
      active_variant: 0,
      variants: [{ content: 'hello', display_content: 'hello', status: 'draft' }],
    },
  ]
  const { screenProps } = useWritingScreenAdapter({})
  assert.equal(screenProps.value.canBranch, true)
  assert.equal(screenProps.value.title, '风起之地')
  assert.equal(screenProps.value.composerDisabled, false)
})

test('writing adapter derives quality accept hint from pipeline.quality', () => {
  const { writing } = setupStores()
  writing.pipeline.quality = {
    passed: false,
    errorCount: 0,
    warningCount: 2,
    warnings: ['a', 'b'],
  }
  const { screenProps } = useWritingScreenAdapter({})
  assert.equal(screenProps.value.qualityAcceptHint, '质量警告 2')
})

test('writing adapter forwards start-writing and wraps delete-variant', async () => {
  setupStores()
  const calls = []
  const { screenEvents } = useWritingScreenAdapter({
    startWriting: (t) => calls.push(['start', t]),
    handleDeleteVariant: (p) => calls.push(['delete', p]),
  })
  screenEvents['start-writing']('续写：')
  await screenEvents['delete-variant']({ nodeId: 'n1' })
  assert.deepEqual(calls[0], ['start', '续写：'])
  assert.deepEqual(calls[1], ['delete', { nodeId: 'n1' }])
})
