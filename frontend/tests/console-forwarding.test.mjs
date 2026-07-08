import test from 'node:test'
import assert from 'node:assert/strict'
import { setupConsoleForwarding } from '../src/utils/consoleForwarding.js'

// setupConsoleForwarding 会永久替换 console 方法（忠实原实现不提供 restore），
// 每个用例自行保存/恢复原始方法，避免污染其它测试。

function withForwardedConsole(fn) {
  return async () => {
    const originals = {
      log: console.log,
      warn: console.warn,
      error: console.error,
      debug: console.debug,
    }
    try {
      await fn()
    } finally {
      console.log = originals.log
      console.warn = originals.warn
      console.error = originals.error
      console.debug = originals.debug
    }
  }
}

test('setupConsoleForwarding 把 console.log 转发给 logFn，level=info', withForwardedConsole(async () => {
  const calls = []
  setupConsoleForwarding((level, msg) => calls.push({ level, msg }))
  console.log('hello', 'world')
  assert.deepEqual(calls, [{ level: 'info', msg: 'hello world' }])
}))

test('setupConsoleForwarding warn→warn、error→error、debug→debug', withForwardedConsole(async () => {
  const calls = []
  setupConsoleForwarding((level, msg) => calls.push({ level, msg }))
  console.warn('w')
  console.error('e')
  console.debug('d')
  assert.deepEqual(calls, [
    { level: 'warn', msg: 'w' },
    { level: 'error', msg: 'e' },
    { level: 'debug', msg: 'd' },
  ])
}))

test('非字符串参数用 JSON.stringify 序列化后拼接', withForwardedConsole(async () => {
  const calls = []
  setupConsoleForwarding((level, msg) => calls.push({ level, msg }))
  console.log('ctx', { a: 1 }, [1, 2])
  assert.equal(calls[0].level, 'info')
  assert.equal(calls[0].msg, 'ctx {"a":1} [1,2]')
}))

test('原始 console 方法仍被调用（转发前先打印）', withForwardedConsole(async () => {
  let originalCalled = 0
  const original = console.log
  console.log = (...args) => { originalCalled += 1; original.apply(console, args) }
  setupConsoleForwarding(() => {})
  console.log('x')
  assert.equal(originalCalled, 1, '原始 console.log 应被调用一次')
}))

test('多参数用空格 join', withForwardedConsole(async () => {
  const calls = []
  setupConsoleForwarding((level, msg) => calls.push(msg))
  console.log('a', 'b', 'c')
  assert.equal(calls[0], 'a b c')
}))
