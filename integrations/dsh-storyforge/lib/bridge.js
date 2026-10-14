/**
 * sidecar 管理：拉起 `storyforge-dsh-bridge`，按行收发 JSON-RPC（协议见
 * crates/dsh-bridge/src/lib.rs）。
 *
 * - 懒启动：第一次请求时拉起，等 `bridge.ready`；进程退出后下一次请求重新拉起。
 * - sidecar 发来的 `llm.stream` 请求交给 `serveLlm` 执行，块以 `llm.chunk` 通知回传，
 *   `llm.cancel` 中止对应调用。
 * - 其余通知（`turn.event` / `turn.settled`）广播给订阅者。
 * - sidecar 的 stderr 追加到日志文件，不进 DSH 日志。
 */

import { spawn } from 'node:child_process'
import { createWriteStream, existsSync, mkdirSync } from 'node:fs'
import { dirname } from 'node:path'
import { createInterface } from 'node:readline'

/** 插件抛给 sidecar 的错误（带协议错误码）。 */
export class BridgeRpcError extends Error {
  constructor(code, message, extra = {}) {
    super(message)
    this.code = code
    this.status = extra.status
    this.retryAfterMs = extra.retryAfterMs
    /** sidecar 错误的结构化附带信息（如 `confirm_generation_mode` 的建议模式）。 */
    this.data = extra.data
  }
}

function toRpcError(error) {
  if (error instanceof BridgeRpcError) {
    const out = { code: error.code, message: error.message }
    if (typeof error.status === 'number') out.status = error.status
    if (typeof error.retryAfterMs === 'number') out.retry_after_ms = error.retryAfterMs
    return out
  }
  if (error?.name === 'AbortError') return { code: 'aborted', message: String(error.message ?? 'aborted') }
  return { code: 'plugin_error', message: String(error?.message ?? error) }
}

const READY_TIMEOUT_MS = 30_000
const SHUTDOWN_TIMEOUT_MS = 3_000

export function createBridge({ binPath, dataDir, logFile, serveLlm, log }) {
  let child = null
  let ready = null
  let nextId = 1
  let lastExit = null
  let readyInfo = null
  /** 本端发出的请求：id → { resolve, reject } */
  const pending = new Map()
  /** sidecar 发来的模型调用：id → AbortController */
  const llmCalls = new Map()
  const listeners = new Set()

  const write = (message) => {
    if (!child?.stdin?.writable) return false
    child.stdin.write(`${JSON.stringify(message)}\n`)
    return true
  }

  function failAll(error) {
    for (const { reject } of pending.values()) reject(error)
    pending.clear()
    for (const controller of llmCalls.values()) controller.abort(error)
    llmCalls.clear()
  }

  function serveLlmRequest(id, params) {
    const controller = new AbortController()
    llmCalls.set(id, controller)
    const onChunk = (chunk) => write({ method: 'llm.chunk', params: { id, chunk } })
    Promise.resolve()
      .then(() => serveLlm(params ?? {}, { signal: controller.signal, onChunk }))
      .then(
        (result) => write({ id, result }),
        (error) => write({ id, error: toRpcError(error) }),
      )
      .finally(() => llmCalls.delete(id))
  }

  function onLine(line) {
    if (!line.trim()) return
    let message
    try {
      message = JSON.parse(line)
    } catch {
      log(`bridge: non-protocol stdout line ignored: ${line.slice(0, 200)}`)
      return
    }
    const { id, method, params } = message
    if (method && id != null) {
      if (method === 'llm.stream') serveLlmRequest(id, params)
      else write({ id, error: { code: 'unknown_method', message: `插件不支持 ${method}` } })
      return
    }
    if (method) {
      if (method === 'llm.cancel') llmCalls.get(params?.id)?.abort(new Error('cancelled by sidecar'))
      for (const listener of listeners) {
        try {
          listener(method, params)
        } catch (error) {
          log(`bridge listener failed: ${error?.stack ?? error}`)
        }
      }
      return
    }
    const entry = pending.get(id)
    if (!entry) return
    pending.delete(id)
    if (message.error) {
      entry.reject(
        new BridgeRpcError(message.error.code ?? 'error', message.error.message ?? 'sidecar error', { data: message.error.data }),
      )
    } else {
      entry.resolve(message.result)
    }
  }

  function start() {
    if (!existsSync(binPath)) {
      throw new BridgeRpcError(
        'bridge_missing',
        `找不到 StoryForge sidecar：${binPath}。先在仓库根目录运行 cargo build -p storyforge-dsh-bridge，或在插件配置里指定 bridgeBin。`,
      )
    }
    mkdirSync(dataDir, { recursive: true })
    mkdirSync(dirname(logFile), { recursive: true })
    const stderrLog = createWriteStream(logFile, { flags: 'a' })
    const proc = spawn(binPath, ['--data-dir', dataDir], { stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true })
    child = proc
    readyInfo = null
    proc.stderr.pipe(stderrLog)
    createInterface({ input: proc.stdout }).on('line', onLine)

    ready = new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new BridgeRpcError('bridge_timeout', 'sidecar 启动超时')), READY_TIMEOUT_MS)
      const unsubscribe = subscribe((method, params) => {
        if (method === 'bridge.ready') {
          readyInfo = params
          log(`bridge ready: pid=${params?.pid} storage=${params?.storage ?? '?'}`)
          clearTimeout(timer)
          unsubscribe()
          resolve(params)
        } else if (method === 'bridge.fatal') {
          clearTimeout(timer)
          unsubscribe()
          reject(new BridgeRpcError('bridge_fatal', params?.message ?? 'sidecar 初始化失败'))
        }
      })
      proc.once('exit', () => {
        clearTimeout(timer)
        unsubscribe()
        reject(new BridgeRpcError('bridge_exited', `sidecar 启动后退出，详见 ${logFile}`))
      })
    })
    // 未被等待时不要变成 unhandledRejection。
    ready.catch(() => {})

    proc.on('error', (error) => log(`bridge spawn error: ${error?.stack ?? error}`))
    proc.on('exit', (code, signal) => {
      lastExit = { at: new Date().toISOString(), code, signal }
      log(`bridge exited: code=${code} signal=${signal}`)
      if (child === proc) {
        child = null
        ready = null
      }
      failAll(new BridgeRpcError('bridge_exited', `sidecar 已退出（code=${code}），详见 ${logFile}`))
      stderrLog.end()
    })
    log(`bridge started: pid=${proc.pid} bin=${binPath} data=${dataDir}`)
  }

  async function ensure() {
    if (!child) start()
    return ready
  }

  async function request(method, params = {}) {
    await ensure()
    const id = `p-${nextId++}`
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject })
      if (!write({ id, method, params })) {
        pending.delete(id)
        reject(new BridgeRpcError('bridge_exited', 'sidecar 不可写'))
      }
    })
  }

  function subscribe(listener) {
    listeners.add(listener)
    return () => listeners.delete(listener)
  }

  async function stop() {
    const proc = child
    if (!proc) return
    const exited = new Promise((resolve) => proc.once('exit', resolve))
    if (ready) {
      request('bridge.shutdown').catch(() => {})
    }
    const timer = setTimeout(() => proc.kill(), SHUTDOWN_TIMEOUT_MS)
    await exited
    clearTimeout(timer)
  }

  return {
    request,
    subscribe,
    stop,
    info: () => ({
      running: child !== null,
      pid: child?.pid ?? null,
      ready: readyInfo,
      lastExit,
      pendingRequests: pending.size,
      activeLlmCalls: llmCalls.size,
      binPath,
      dataDir,
      logFile,
    }),
  }
}
