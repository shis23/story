import test from 'node:test'
import assert from 'node:assert/strict'
import { taskStatusClass, taskStatusText } from '../src/utils/taskStatus.js'

test('renders known task statuses as their raw status labels', () => {
  assert.equal(taskStatusText('pending'), 'pending')
  assert.equal(taskStatusText('active'), 'active')
  assert.equal(taskStatusText('completed'), 'completed')
  assert.equal(taskStatusText('abandoned'), 'abandoned')
})

test('renders non-string task statuses with existing fallbacks', () => {
  assert.equal(taskStatusText('blocked'), 'blocked')
  assert.equal(taskStatusText({ likely_completed: 0.756 }), '可能完成 (76%)')
  assert.equal(taskStatusText({ state: 'review' }), '{"state":"review"}')
  assert.equal(taskStatusText(null), 'null')
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
  assert.equal(taskStatusClass({ likely_completed: 0.2 }), 'bg-warn/10 text-warn')
})
