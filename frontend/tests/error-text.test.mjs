import test from 'node:test'
import assert from 'node:assert/strict'
import { errorText, isCancelledError } from '../src/utils/errorText.js'

test('errorText unwraps structured Tauri command error DTOs', () => {
  const dto = { type: 'validation', message: '当前有未完成的轮次，请先 Accept' }
  assert.equal(errorText(dto), '当前有未完成的轮次，请先 Accept')
})

test('errorText keeps plain strings and Error instances readable', () => {
  assert.equal(errorText('boom'), 'boom')
  assert.equal(errorText(new TypeError('writeBinaryFile is not a function')), 'TypeError: writeBinaryFile is not a function')
})

test('errorText handles nullish and message-less objects without crashing', () => {
  assert.equal(errorText(null), '未知错误')
  assert.equal(typeof errorText({}), 'string')
  const err = new Error('real message')
  assert.equal(errorText(err), 'Error: real message')
})

test('Tauri cancellation has a non-failure terminal representation', () => {
  assert.equal(isCancelledError({ type: 'cancelled' }), true)
  assert.equal(isCancelledError({ type: 'pipeline', message: 'cancelled operation failed' }), false)
  assert.equal(errorText({ type: 'cancelled' }), '已停止')
  assert.equal(errorText({ type: 'storage' }), '存储失败')
  assert.equal(errorText({ unknown: true }), '未知错误')
})
