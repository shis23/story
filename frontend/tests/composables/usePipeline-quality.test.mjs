import test from 'node:test'
import assert from 'node:assert/strict'
import { createPinia, setActivePinia } from 'pinia'
import { useWritingStore } from '../../src/stores/writing.js'
import { usePipeline } from '../../src/composables/usePipeline.js'

// usePipeline 依赖 usePluginBridge（异步 prompt hook）。这里只测 quality_checked 分支，
// 不发起真实插件调用；Pinia 必须先激活。
function setupPipeline() {
  setActivePinia(createPinia())
  const writing = useWritingStore()
  writing.pipeline.editor = { status: 'done', detail: '成文完成', output: 'draft' }
  writing.pipeline.stateLabel = '已产出'
  const { handlePipelineEvent } = usePipeline({ scrollToBottom: () => {} })
  return { writing, handlePipelineEvent }
}

test('quality_checked 通过时写入 pipeline.quality', () => {
  const { writing, handlePipelineEvent } = setupPipeline()
  handlePipelineEvent({
    event_type: 'quality_checked',
    data: { passed: true, warning_count: 0, warnings: [] },
  })
  assert.equal(writing.pipeline.quality.passed, true)
  assert.equal(writing.pipeline.quality.warningCount, 0)
  assert.equal(writing.pipeline.quality.status, 'ok')
  assert.equal(writing.pipeline.stateLabel, '已产出')
})

test('quality_checked 有警告时更新 label 与 editor.detail（warn-only）', () => {
  const { writing, handlePipelineEvent } = setupPipeline()
  handlePipelineEvent({
    event_type: 'quality_checked',
    data: {
      passed: false,
      warning_count: 2,
      warnings: ['检测到 n-gram 重复', '字数过短'],
    },
  })
  assert.equal(writing.pipeline.quality.passed, false)
  assert.equal(writing.pipeline.quality.warningCount, 2)
  assert.deepEqual(writing.pipeline.quality.warnings, ['检测到 n-gram 重复', '字数过短'])
  assert.equal(writing.pipeline.quality.status, 'warn')
  assert.match(writing.pipeline.stateLabel, /质量警告 2/)
  assert.match(writing.pipeline.editor.detail, /质量警告 2/)
})
