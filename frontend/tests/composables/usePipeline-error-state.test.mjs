// 2026-09-01 全量审查补测：pipeline error 事件置 error 态；取消失败可见；
// 成功路径不覆盖事件流已置的 error 态。
import test from 'node:test'
import assert from 'node:assert/strict'
import { createPinia, setActivePinia } from 'pinia'
import { useWritingStore } from '../../src/stores/writing.js'
import { useCampaignStore } from '../../src/stores/campaign.js'
import { useUiStore } from '../../src/stores/ui.js'
import { usePipeline } from '../../src/composables/usePipeline.js'
import { useWriting } from '../../src/composables/useWriting.js'

function setupStores() {
  setActivePinia(createPinia())
  return {
    writing: useWritingStore(),
    campaign: useCampaignStore(),
    ui: useUiStore(),
  }
}

test('pipeline error event sets state=error so the error banner can render', () => {
  const { writing } = setupStores()
  const { handlePipelineEvent } = usePipeline({})

  handlePipelineEvent({ event_type: 'error', data: { message: '连接中断' } })

  assert.equal(writing.pipeline.state, 'error')
  assert.equal(writing.pipeline.stateLabel, '错误: 连接中断')
})

test('cancelWriting failure surfaces in stateLabel instead of being swallowed', async () => {
  const { writing } = setupStores()
  const { cancelWriting } = useWriting({
    cancelWritingApi: async () => { throw { type: 'pipeline', message: '无运行中的流水线' } },
  })

  await cancelWriting()

  assert.equal(writing.pipeline.state, 'error')
  assert.match(writing.pipeline.stateLabel, /停止失败/)
  assert.match(writing.pipeline.stateLabel, /无运行中的流水线/)
})

test('Tauri cancelled DTO stops writing without an error banner', async () => {
  const { writing, campaign } = setupStores()
  campaign.activeCampaign = { id: 'campaign-1', name: 'Test' }
  campaign.currentConversationId = 'conversation-1'
  writing.activeConnection = { id: 'conn-1' }
  const { startWriting } = useWriting({
    startWritingApi: async () => { throw { type: 'cancelled' } },
  })
  await startWriting('Continue')
  assert.equal(writing.pipeline.state, 'idle')
  assert.equal(writing.pipeline.stateLabel, '已停止')
  assert.equal(writing.isWriting, false)
  assert.equal(writing.showPipeline, false)
})

test('late cancel acknowledgement does not overwrite the terminal stopped label', async () => {
  const { writing } = setupStores()
  writing.isWriting = true
  const { cancelWriting } = useWriting({
    cancelWritingApi: async () => {
      writing.isWriting = false
      writing.pipeline.stateLabel = '已停止'
    },
  })
  await cancelWriting()
  assert.equal(writing.pipeline.stateLabel, '已停止')
})

test('startWriting success does not overwrite an error state set by the event stream', async () => {
  const { writing, campaign } = setupStores()
  campaign.activeCampaign = { id: 'campaign-1', name: '测试档案' }
  campaign.currentConversationId = 'conversation-1'
  writing.activeConnection = { id: 'conn-1', name: '测试连接' }
  writing.writingMode = 'campaign'

  let pipelineHook = null
  let resolveApi = null
  const apiPending = new Promise((resolve) => { resolveApi = resolve })
  const { handlePipelineEvent } = usePipeline({})
  const { startWriting } = useWriting({
    handlePipelineEvent,
    startWritingApi: async (intent, charId, onEvent) => {
      pipelineHook = onEvent
      // 挂起直到错误事件注入，保证 error 先于 resolve 到达
      await apiPending
      return { text: '成文', conversation_id: 'conversation-1', node_id: 'node-9' }
    },
    getConversationApi: async () => null,
  })

  const promise = startWriting('继续')
  await new Promise((resolve) => setTimeout(resolve, 0))
  assert.ok(pipelineHook, 'onEvent 回调应已注入')
  pipelineHook({ event_type: 'error', data: { message: '子 Agent 超时' } })
  resolveApi()
  await promise

  assert.equal(writing.pipeline.state, 'error')
  assert.match(writing.pipeline.stateLabel, /子 Agent 超时/)
  assert.notEqual(writing.pipeline.stateLabel, '已完成')
})
