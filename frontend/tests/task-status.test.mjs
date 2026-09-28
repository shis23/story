import test from 'node:test'
import assert from 'node:assert/strict'
import { isLikelyCompleted, likelyCompletedConfidence, taskStatusClass, taskStatusText } from '../src/utils/taskStatus.js'

test('renders known task statuses as their raw status labels', () => {
  assert.equal(taskStatusText('pending'), 'pending')
  assert.equal(taskStatusText('active'), 'active')
  assert.equal(taskStatusText('completed'), 'completed')
  assert.equal(taskStatusText('abandoned'), 'abandoned')
})

test('renders non-string task statuses with existing fallbacks', () => {
  assert.equal(taskStatusText('blocked'), 'blocked')
  // F-08：真实 serde 形状是外部标签枚举 {"likely_completed":{"confidence":0.5}}
  // （crates/domain/src/story_task.rs），旧夹具冻结了扁平形状 {likely_completed:0.5}，
  // 于是真实数据渲染成 NaN%。两种形状都必须正确。
  assert.equal(taskStatusText({ likely_completed: { confidence: 0.756 } }), '可能完成 (76%)')
  assert.equal(taskStatusText({ likely_completed: 0.756 }), '可能完成 (76%)')
  assert.equal(taskStatusText({ likely_completed: { confidence: 0 } }), '可能完成 (0%)')
  assert.equal(taskStatusText({ state: 'review' }), '{"state":"review"}')
  assert.equal(taskStatusText(null), 'null')
})

test('likely_completed 判定与置信度提取（F-08/F-07 共用）', () => {
  assert.equal(isLikelyCompleted({ likely_completed: { confidence: 0.2 } }), true)
  assert.equal(isLikelyCompleted({ likely_completed: 0.2 }), true)
  assert.equal(isLikelyCompleted('pending'), false)
  assert.equal(isLikelyCompleted({ state: 'review' }), false)
  assert.equal(isLikelyCompleted(null), false)
  assert.equal(likelyCompletedConfidence({ likely_completed: { confidence: 0.25 } }), 0.25)
  assert.equal(likelyCompletedConfidence({ likely_completed: {} }), null)
  assert.equal(likelyCompletedConfidence(null), null)
})

test('maps known task statuses to their existing badge classes', () => {
  assert.equal(taskStatusClass('pending'), 'bg-wait/10 text-wait')
  assert.equal(taskStatusClass('active'), 'bg-running/10 text-running')
  assert.equal(taskStatusClass('completed'), 'bg-ok/10 text-ok')
  assert.equal(taskStatusClass('abandoned'), 'bg-ink-soft/10 text-ink-soft')
})

test('uses warning badge classes for unknown and non-string task statuses', () => {
  assert.equal(taskStatusClass('blocked'), 'bg-warn/10 text-warn')
  assert.equal(taskStatusClass(null), 'bg-warn/10 text-warn')
  assert.equal(taskStatusClass({ likely_completed: { confidence: 0.2 } }), 'bg-warn/10 text-warn')
  assert.equal(taskStatusClass({ likely_completed: 0.2 }), 'bg-warn/10 text-warn')
})
