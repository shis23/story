// sidecar 启动失败的三种形态都不允许挂死：
//   a) 同步 throw（Windows 对非可执行文件报 EFTYPE）
//   b) 异步 error + close 事件（ENOENT / 杀软拦截 / 缺执行位，无 exit 事件）
//   c) 启动后 exit
// 修复前 b) 路径的 ready 永远 pending、child 永不清空：所有请求等 READY_TIMEOUT
// 超时且无法自愈，只能重启插件。
// 运行：node --test integrations/dsh-storyforge/test/bridge-spawn.test.mjs

import assert from 'node:assert/strict'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { PassThrough } from 'node:stream'
import { test } from 'node:test'

import { createBridge } from '../lib/bridge.js'

const here = dirname(fileURLToPath(import.meta.url))

function makeBridge(overrides = {}) {
  const lines = []
  const bridge = createBridge({
    // 存在但不可执行的文件（本测试源码自身）：Windows CreateProcess 同步 throw。
    binPath: resolve(here, 'bridge-spawn.test.mjs'),
    dataDir: join(tmpdir(), 'sf-bridge-spawn-test'),
    logFile: join(tmpdir(), 'sf-bridge-spawn-test', 'bridge.log'),
    serveLlm: async () => {
      throw new Error('spawn 失败路径不应有模型调用')
    },
    log: (line) => lines.push(String(line)),
    ...overrides,
  })
  return { bridge, lines }
}

/** 异步失败形态：只发 error（随后 close），不发 exit——修复前会挂死。 */
function asyncFailingSpawn(calls) {
  return () => {
    calls.push(Date.now())
    const proc = new PassThrough()
    proc.stdin = { writable: false, write: () => false }
    proc.stdout = new PassThrough()
    proc.stderr = new PassThrough()
    proc.pid = 4242
    queueMicrotask(() => {
      proc.emit('error', Object.assign(new Error('spawn ENOENT (simulated)'), { code: 'ENOENT' }))
      proc.emit('close', null, 'ENOENT (simulated)')
    })
    return proc
  }
}

test('异步 spawn 失败：请求快速落定而非 30s 超时，child 清空', async () => {
  const calls = []
  const { bridge, lines } = makeBridge({ spawnFn: asyncFailingSpawn(calls) })
  const startedAt = Date.now()
  await assert.rejects(
    bridge.request('bridge.ping'),
    (err) => err.code === 'bridge_spawn_failed',
    '应落定为 bridge_spawn_failed；若只能等 bridge_timeout 说明回到挂死路径',
  )
  assert.ok(Date.now() - startedAt < 10_000, 'spawn 失败应快速落定，而非等待 READY_TIMEOUT')
  assert.equal(bridge.info().running, false, 'child 必须清空，否则下次 ensure() 复用死进程')
  assert.ok(lines.some((l) => l.includes('spawn error')), '应记录 spawn error 日志')
})

test('异步 spawn 失败后再次请求走重试而非复用死进程', async () => {
  const calls = []
  const { bridge } = makeBridge({ spawnFn: asyncFailingSpawn(calls) })
  await assert.rejects(bridge.request('bridge.ping'), () => true)
  assert.equal(calls.length, 1)
  // 修复前：第二次 request 复用 pending 的 ready，挂到 30s 超时
  const startedAt = Date.now()
  await assert.rejects(bridge.request('bridge.ping'), () => true)
  assert.ok(Date.now() - startedAt < 10_000, '重试也必须快速落定')
  assert.equal(calls.length, 2, '第二次请求应重新 spawn（自愈路径）')
})

test('同步 spawn 失败（EFTYPE 等）：包装为 bridge_spawn_failed 而非裸 Error', async () => {
  const { bridge } = makeBridge()
  await assert.rejects(
    bridge.request('bridge.ping'),
    (err) => err.code === 'bridge_spawn_failed' && /EFTYPE|启动失败/.test(err.message),
    '同步 throw 也应统一为 BridgeRpcError 语义',
  )
  assert.equal(bridge.info().running, false)
})
