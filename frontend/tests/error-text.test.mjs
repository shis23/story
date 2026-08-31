import test from 'node:test'
import assert from 'node:assert/strict'
import { errorText } from '../src/utils/errorText.js'

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
  // Error 实例保留类别前缀（"Error: real message"），DTO 只取 message
  const err = new Error('real message')
  assert.equal(errorText(err), 'Error: real message')
})
