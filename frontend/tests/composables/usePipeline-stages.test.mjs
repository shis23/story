import test from 'node:test'
import assert from 'node:assert/strict'
import { createPinia, setActivePinia } from 'pinia'
import { useWritingStore } from '../../src/stores/writing.js'
import { usePipeline } from '../../src/composables/usePipeline.js'

function setupPipeline() {
  setActivePinia(createPinia())
  const writing = useWritingStore()
  const { handlePipelineEvent } = usePipeline({
    scrollToBottom: () => {},
    pluginBridge: {
      broadcastPluginPipelineEvent: () => {},
      handlePromptHookRequest: () => {},
    },
  })
  return { writing, handlePipelineEvent }
}

test('continuation keeps the real writer role and the complete draft text', () => {
  const { writing, handlePipelineEvent } = setupPipeline()

  handlePipelineEvent({ event_type: 'writer_started', data: {} })
  handlePipelineEvent({ event_type: 'writer_progress', data: { delta: '流式前半段' } })
  handlePipelineEvent({
    event_type: 'draft_ready',
    data: { text: '这是完整正文，不能只剩最后几个字。' },
  })
  handlePipelineEvent({ event_type: 'writer_progress', data: { delta: '迟到 token' } })

  assert.equal(writing.pipeline.editor.role, 'writer')
  assert.equal(writing.pipeline.editor.status, 'done')
  assert.equal(writing.pipeline.editor.output, '这是完整正文，不能只剩最后几个字。')
})

test('postprocess start records the two enabled model calls separately', () => {
  const { writing, handlePipelineEvent } = setupPipeline()

  handlePipelineEvent({
    event_type: 'postprocess_started',
    data: { summarizer_enabled: true, postprocessor_enabled: true },
  })
  assert.equal(writing.pipeline.summary.status, 'running')
  assert.equal(writing.pipeline.postprocess.status, 'running')

  handlePipelineEvent({ event_type: 'summary_done', data: { char_count: 128 } })
  handlePipelineEvent({
    event_type: 'postprocess_done',
    data: { knowledge_count: 2, variable_count: 1, task_count: 0 },
  })

  assert.equal(writing.pipeline.summary.status, 'done')
  assert.match(writing.pipeline.summary.detail, /128/)
  assert.equal(writing.pipeline.postprocess.status, 'done')
  assert.match(writing.pipeline.postprocess.detail, /知识 2/)
})

test('disabled summarizer does not fabricate a summary model call', () => {
  const { writing, handlePipelineEvent } = setupPipeline()

  handlePipelineEvent({
    event_type: 'postprocess_started',
    data: { summarizer_enabled: false, postprocessor_enabled: true },
  })

  assert.equal(writing.pipeline.summary.status, 'idle')
  assert.equal(writing.pipeline.postprocess.status, 'running')
})

test('disabled state ledger stays hidden after its explicit skipped event', () => {
  const { writing, handlePipelineEvent } = setupPipeline()

  handlePipelineEvent({
    event_type: 'postprocess_started',
    data: { summarizer_enabled: true, postprocessor_enabled: false },
  })
  handlePipelineEvent({ event_type: 'summary_done', data: { char_count: 64 } })
  handlePipelineEvent({
    event_type: 'postprocess_skipped',
    data: { reason: 'enable_postprocess=false（已按配置跳过后处理 Agent）' },
  })

  assert.equal(writing.pipeline.summary.status, 'done')
  assert.equal(writing.pipeline.postprocess.status, 'idle')
})
