/**
 * dsh-plugin-storyforge — 宿主半边（PoC）
 *
 * P0 / P1 阶段用**假管线**：注册 provider `storyforge`，按脚本流式输出思考块（模拟
 * 导演 → 角色 → 编剧）和一段固定正文，不需要 Rust、不需要任何凭据。P2 换成 sidecar
 * 跑真实 StoryForge 管线。
 *
 * 设计要点
 * - 不 import 任何 DSH 包：插件源码在仓库里，Node 解析不到 profile 的 node_modules。
 *   适配器是鸭子类型对象（`registerAdapter` 不做 instanceof 检查，只调用方法）。
 * - `listModels` 返回空：StoryForge 不出现在任何会话的模型选择器里；核心路由接受未
 *   列出的 model id，StoryForge 会话由 `./agent` 插件钉到 storyforge/pipeline。
 * - 路由走 `connection.fetch`（/api 通道），继承 GUI 的认证门；不用 `webServer.register`。
 */

import { API_BASE, MODEL, PROVIDER, log } from './shared.js'

export const name = 'storyforge'

export const inject = ['llm', 'connection']

/** 足够大的上下文窗口：避免 DSH 按窗口占比触发压缩（StoryForge 自己管上下文）。 */
const CONTEXT_WINDOW = 1_000_000

/** 最近的适配器调用记录（调试路由读取，用于验收"预设无工具""模型被钉住"等）。 */
const MAX_CALL_LOG = 50

const sleep = (ms, signal) =>
  new Promise((resolve, reject) => {
    if (signal?.aborted) return reject(signal.reason ?? new Error('aborted'))
    const timer = setTimeout(resolve, ms)
    signal?.addEventListener(
      'abort',
      () => {
        clearTimeout(timer)
        reject(signal.reason ?? new Error('aborted'))
      },
      { once: true },
    )
  })

/** 取最后一条用户消息的纯文本（content 可能是字符串或内容块数组）。 */
function lastUserText(messages) {
  for (let i = (messages?.length ?? 0) - 1; i >= 0; i--) {
    const message = messages[i]
    if (message?.role !== 'user') continue
    const content = message.content
    if (typeof content === 'string') return content
    if (Array.isArray(content)) {
      return content
        .filter((block) => block?.type === 'text' && typeof block.text === 'string')
        .map((block) => block.text)
        .join('\n')
    }
  }
  return ''
}

/** 把一段文本按小块流式吐出（一个内容块：block-start → 若干 delta → block-end）。 */
async function* streamBlock(index, blockType, text, signal, chunkSize = 12, delayMs = 30) {
  yield { type: 'block-start', index, blockType }
  for (let offset = 0; offset < text.length; offset += chunkSize) {
    await sleep(delayMs, signal)
    const piece = text.slice(offset, offset + chunkSize)
    yield blockType === 'reasoning'
      ? { type: 'reasoning-delta', index, text: piece }
      : { type: 'text-delta', index, text: piece }
  }
  yield { type: 'block-end', index, block: { type: blockType, text } }
}

/** 假管线：按 purpose 分流，普通请求输出"阶段思考块 + 正文"。 */
async function* fakePipelineStream(options) {
  const { signal } = options
  if (options.purpose === 'session-title') {
    yield* streamBlock(0, 'text', 'StoryForge 测试故事', signal, 64, 0)
    yield { type: 'usage', usage: { inputTokens: 0, outputTokens: 0 } }
    yield { type: 'finish', reason: { kind: 'stop' } }
    return
  }
  if (options.purpose === 'compaction') {
    // StoryForge 自己管理上下文；PoC 先返回空摘要并记录，观察 DSH 是否真的会发起压缩。
    yield* streamBlock(0, 'text', '', signal)
    yield { type: 'usage', usage: { inputTokens: 0, outputTokens: 0 } }
    yield { type: 'finish', reason: { kind: 'stop' } }
    return
  }

  const intent = lastUserText(options.messages).trim() || '（无输入）'
  const stages = [
    '【导演】解析行动意图，规划本轮场景与出场角色。\n',
    '【角色】为在场角色分别生成反应（假数据）。\n',
    '【编剧】整合角色输出，润色成正文。\n',
  ].join('')
  const prose = [
    `你说：「${intent}」。`,
    '',
    '夜风穿过空荡的街道，灯笼在檐下轻轻摇晃。这是一段由 StoryForge 假管线生成的占位正文，',
    '用于验证 DSH 侧的流式显示、思考块折叠、取消与会话隔离。',
  ].join('\n')

  yield* streamBlock(0, 'reasoning', stages, signal, 8, 60)
  yield* streamBlock(1, 'text', prose, signal)
  yield { type: 'usage', usage: { inputTokens: 0, outputTokens: prose.length } }
  yield { type: 'finish', reason: { kind: 'stop' } }
}

function createAdapter(callLog) {
  return {
    providerInfo(provider) {
      return { id: provider, name: 'StoryForge' }
    },
    providerRetryPolicy() {
      return undefined
    },
    imageRequestPricing() {
      return undefined
    },
    listModels() {
      return Promise.resolve([])
    },
    resolveModel(provider, model) {
      return Promise.resolve({
        provider,
        id: model,
        name: 'StoryForge 管线',
        context: { contextWindow: CONTEXT_WINDOW },
      })
    },
    async prepareCall(provider, model, signal) {
      return {
        model: await this.resolveModel(provider, model, signal),
        stream: (options) => this.stream(options),
      }
    },
    stream(options) {
      const entry = {
        at: new Date().toISOString(),
        provider: options.provider,
        model: options.model,
        sessionId: options.sessionId ?? null,
        purpose: options.purpose ?? null,
        messageCount: options.messages?.length ?? 0,
        toolNames: (options.tools ?? []).map((tool) => tool.name),
        hasSystem: typeof options.system === 'string',
      }
      callLog.unshift(entry)
      callLog.length = Math.min(callLog.length, MAX_CALL_LOG)
      log(
        `stream: session=${entry.sessionId} purpose=${entry.purpose} tools=${entry.toolNames.length} messages=${entry.messageCount}`,
      )
      return fakePipelineStream(options)
    },
  }
}

export function apply(ctx) {
  const callLog = []

  // registerAdapter 内部经调用方的 ctx.effect 登记，插件卸载时自动撤销路由。
  ctx.llm.registerAdapter([PROVIDER], createAdapter(callLog))
  log(`adapter registered: provider=${PROVIDER} model=${MODEL}`)

  const json = (body, status = 200) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { 'content-type': 'application/json; charset=utf-8', 'cache-control': 'no-store' },
    })

  const routes = [
    {
      path: `${API_BASE}/ping`,
      methods: ['GET'],
      requestBody: 'buffered',
      fetch: async () => json({ ok: true, provider: PROVIDER, model: MODEL, pid: process.pid }),
    },
    {
      path: `${API_BASE}/debug/calls`,
      methods: ['GET'],
      requestBody: 'buffered',
      fetch: async () => json({ ok: true, calls: callLog }),
    },
  ]

  ctx.effect(() => {
    const disposers = routes.map((route) => ctx.connection.fetch.register(route))
    log(`protected routes ready: ${routes.map((route) => route.path).join(', ')}`)
    return () => {
      for (const dispose of disposers) {
        if (typeof dispose === 'function') dispose()
        else if (dispose && typeof dispose.dispose === 'function') dispose.dispose()
      }
    }
  }, 'storyforge: protected api routes')
}
