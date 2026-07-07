import test from 'node:test'
import assert from 'node:assert/strict'
import { getTrustedMvuRuntimeMessage } from '../src/mvu-runtime-bridge.js'

test('accepts MVU runtime messages only from the owned iframe window', () => {
  const runtimeWindow = { postMessage() {} }
  const pluginWindow = { postMessage() {} }
  const message = { type: 'mvu:execute_result', request_id: 'req-1' }

  assert.equal(
    getTrustedMvuRuntimeMessage({ source: runtimeWindow, origin: 'null', data: message }, runtimeWindow),
    message,
  )
  assert.equal(
    getTrustedMvuRuntimeMessage({ source: pluginWindow, origin: 'null', data: message }, runtimeWindow),
    null,
  )
  assert.equal(
    getTrustedMvuRuntimeMessage({ source: runtimeWindow, origin: 'null', data: { type: 'sf:ready' } }, runtimeWindow),
    null,
  )
})

test('rejects MVU runtime messages before the iframe window is available', () => {
  assert.equal(
    getTrustedMvuRuntimeMessage({ source: {}, data: { type: 'mvu:ready' } }, null),
    null,
  )
})
