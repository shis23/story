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
 * - 建会话照 DSH 自带 webhook 的流程：解析预设 → 工作区 → agents.create（挂预设）→
 *   挂到工作区 → 改标题 → followup 开场消息。不写任何合成会话事件。
 */

import { randomUUID } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'

import {
  API_BASE,
  MODEL,
  OPENING_SOURCE_KIND,
  PRESET,
  PROVIDER,
  SESSION_PREFIX,
  lifecycleEvents,
  log,
  setForkObserver,
} from './shared.js'

export const name = 'storyforge'

export const inject = ['llm', 'connection', 'agents', 'agentPresets', 'workspaceRegistry', 'sessionTitle']

/** 足够大的上下文窗口：避免 DSH 按窗口占比触发压缩（StoryForge 自己管上下文）。 */
const CONTEXT_WINDOW = 1_000_000

/** 仅调试用的本地回显 provider：在无凭据的临时实例里模拟"普通会话"，不发任何外部请求。 */
const ECHO_PROVIDER = 'sf-debug-echo'
const ECHO_MODEL = 'echo'

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

function deepFreeze(value) {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    Object.freeze(value)
    for (const child of Object.values(value)) deepFreeze(child)
  }
  return value
}

/** 等价 dsh-llm 的 createUserMessage：带新身份、冻结的 user 消息。 */
function createUserMessage(text, source) {
  return deepFreeze({ id: randomUUID(), role: 'user', content: [{ type: 'text', text }], source })
}

/** 用户亲手输入的消息的来源 kind；DSH 注入的 user 角色消息各有自己的 kind。 */
const USER_SOURCE_KIND = 'user'

/**
 * 本轮的输入：最后一条 assistant 之后、最后一条用户亲手输入的消息（或我们的开场）。
 *
 * DSH 会以 user 角色注入各种消息，按 `source.kind` 区分（0.2.0-rc.1 实测）：
 * `runtime-context`（运行时快照）、`skill-catalog`（skills 目录）、`model-selection`
 * （换模型通知，持久写入会话，StoryForge 会话被钉模型后可能每轮都有）。它们都不是用户的
 * 行动。没有 source 的消息按用户输入处理。
 * @returns 本轮消息（可能没有）、是否开场、跳过的注入消息 kind。
 */
function currentTurn(messages) {
  const injected = []
  for (let i = (messages?.length ?? 0) - 1; i >= 0; i--) {
    const message = messages[i]
    if (message?.role === 'assistant') break
    if (message?.role !== 'user') continue
    const kind = message.source?.kind
    if (kind === OPENING_SOURCE_KIND) return { message, opening: true, injected }
    if (kind === undefined || kind === USER_SOURCE_KIND) return { message, opening: false, injected }
    injected.push(kind)
  }
  return { message: undefined, opening: false, injected }
}

/** 消息的文本块（content 可能是字符串或内容块数组）。 */
function textBlocks(message) {
  const content = message?.content
  if (typeof content === 'string') return [content]
  if (!Array.isArray(content)) return []
  return content.filter((block) => block?.type === 'text' && typeof block.text === 'string').map((block) => block.text)
}

/** 末尾几条消息的结构摘要（调试用：看 DSH 往请求里注入了什么）。 */
function describeTail(messages, count) {
  return (messages ?? []).slice(-count).map((message) => ({
    role: message?.role,
    source: message?.source ? `${message.source.kind ?? '?'}/${message.source.form ?? '-'}` : null,
    blocks: Array.isArray(message?.content) ? message.content.map((block) => block?.type) : typeof message?.content,
    head: textBlocks(message).join(' ').slice(0, 40),
  }))
}

/** 把一段文本按小块流式吐出（一个内容块：block-start → 若干 delta → block-end）。 */
async function* streamBlock(index, blockType, text, signal, chunkSize = 12, delayMs = 30) {
  yield { type: 'block-start', index, blockType }
  for (let offset = 0; offset < text.length; offset += chunkSize) {
    if (delayMs > 0) await sleep(delayMs, signal)
    const piece = text.slice(offset, offset + chunkSize)
    yield blockType === 'reasoning'
      ? { type: 'reasoning-delta', index, text: piece }
      : { type: 'text-delta', index, text: piece }
  }
  yield { type: 'block-end', index, block: { type: blockType, text } }
}

async function* finishStop(outputTokens = 0) {
  yield { type: 'usage', usage: { inputTokens: 0, outputTokens } }
  yield { type: 'finish', reason: { kind: 'stop' } }
}

const FAKE_GREETING = [
  '客栈的门在风里吱呀作响。柜台后的老板抬起头，打量着你这个深夜来客。',
  '',
  '「打尖还是住店？」',
  '',
  '（这是 StoryForge 假管线的开场白占位。P2 起这里会是角色卡的开场。）',
].join('\n')

/** 假管线：按 purpose 分流；开场返回开场白；普通请求输出"阶段思考块 + 正文"。 */
async function* fakePipelineStream(options) {
  const { signal } = options
  if (options.purpose === 'session-title') {
    yield* streamBlock(0, 'text', 'StoryForge 测试故事', signal, 64, 0)
    yield* finishStop()
    return
  }
  if (options.purpose === 'compaction') {
    // StoryForge 自己管理上下文；PoC 先返回空摘要并记录，观察 DSH 是否真的会发起压缩。
    yield* finishStop()
    return
  }

  const turn = currentTurn(options.messages)
  if (turn.opening) {
    yield* streamBlock(0, 'text', FAKE_GREETING, signal)
    yield* finishStop(FAKE_GREETING.length)
    return
  }

  const intent = textBlocks(turn.message).join('\n').trim() || '（无输入）'
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
  yield* finishStop(prose.length)
}

/** 调试回显：回显本轮用户输入，并报告收到的工具数与跳过的注入消息。 */
async function* echoStream(options) {
  if (options.purpose === 'session-title') {
    yield* streamBlock(0, 'text', '普通会话（回显）', options.signal, 64, 0)
    yield* finishStop()
    return
  }
  const turn = currentTurn(options.messages)
  const text = [
    `回显：${textBlocks(turn.message).join('\n').trim() || '（空）'}`,
    `（调试回显模型，收到 ${options.tools?.length ?? 0} 个工具${turn.injected.length ? `；跳过注入：${turn.injected.join('、')}` : ''}）`,
  ].join('\n')
  yield* streamBlock(0, 'text', text, options.signal, 16, 15)
  yield* finishStop(text.length)
}

function createAdapter({ displayName, models, contextWindow, streamImpl, callLog }) {
  return {
    providerInfo(provider) {
      return { id: provider, name: displayName }
    },
    providerRetryPolicy() {
      return undefined
    },
    imageRequestPricing() {
      return undefined
    },
    listModels(provider) {
      return Promise.resolve(models.map((model) => ({ provider, ...model })))
    },
    resolveModel(provider, model) {
      const listed = models.find((entry) => entry.id === model)
      return Promise.resolve({
        provider,
        id: model,
        name: listed?.name ?? model,
        context: { contextWindow },
      })
    },
    async prepareCall(provider, model, signal) {
      return {
        model: await this.resolveModel(provider, model, signal),
        stream: (options) => this.stream(options),
      }
    },
    stream(options) {
      const turn = currentTurn(options.messages)
      const entry = {
        at: new Date().toISOString(),
        provider: options.provider,
        model: options.model,
        sessionId: options.sessionId ?? null,
        purpose: options.purpose ?? null,
        messageCount: options.messages?.length ?? 0,
        toolNames: (options.tools ?? []).map((tool) => tool.name),
        hasSystem: typeof options.system === 'string',
        opening: turn.opening,
        injected: turn.injected,
        tail: describeTail(options.messages, 4),
      }
      callLog.unshift(entry)
      callLog.length = Math.min(callLog.length, MAX_CALL_LOG)
      log(
        `stream[${options.provider}]: session=${entry.sessionId} purpose=${entry.purpose} opening=${entry.opening} tools=${entry.toolNames.length} messages=${entry.messageCount} injected=[${entry.injected.join(',')}]`,
      )
      return streamImpl(options)
    },
  }
}

/** sessionId → 会话元数据（P2 起加 campaignId）。整份 JSON，原子替换写入。 */
function createSessionMap(dataDir) {
  const file = join(dataDir, 'sessions.json')
  let entries = {}
  if (existsSync(file)) {
    try {
      entries = JSON.parse(readFileSync(file, 'utf8'))
    } catch (error) {
      log(`sessions.json unreadable, starting empty: ${error}`)
    }
  }
  const save = () => {
    mkdirSync(dataDir, { recursive: true })
    const tmp = `${file}.tmp`
    writeFileSync(tmp, JSON.stringify(entries, null, 2))
    renameSync(tmp, file)
  }
  return {
    get: (id) => entries[id],
    all: () => ({ ...entries }),
    set(id, value) {
      entries[id] = value
      save()
    },
  }
}

export function apply(ctx, config) {
  const dshHome = process.env.DSH_HOME || join(homedir(), '.dsh')
  const dataDir = config?.dataDir || join(dshHome, 'storyforge')
  const workspacePath = config?.workspacePath || join(dataDir, 'stories')
  const callLog = []
  const sessions = createSessionMap(dataDir)

  // registerAdapter 内部经调用方的 ctx.effect 登记，插件卸载时自动撤销路由。
  ctx.llm.registerAdapter(
    [PROVIDER],
    createAdapter({
      displayName: 'StoryForge',
      models: [],
      contextWindow: CONTEXT_WINDOW,
      streamImpl: fakePipelineStream,
      callLog,
    }),
  )
  log(`adapter registered: provider=${PROVIDER} model=${MODEL}`)

  if (config?.debugEcho) {
    ctx.llm.registerAdapter(
      [ECHO_PROVIDER],
      createAdapter({
        displayName: '调试回显（PoC）',
        models: [{ id: ECHO_MODEL, name: '调试回显' }],
        contextWindow: 128_000,
        streamImpl: echoStream,
        callLog,
      }),
    )
    log(`debug echo adapter registered: provider=${ECHO_PROVIDER} model=${ECHO_MODEL}`)
  }

  // 分叉子会话继承预设：agent 作用域插件发现后在这里补映射（P3 对应到 Campaign 分支）。
  ctx.effect(
    () =>
      setForkObserver((childId, parentId) => {
        if (sessions.get(childId)) return
        const parent = sessions.get(parentId)
        sessions.set(childId, {
          title: parent ? `${parent.title}（分叉）` : '分叉的故事',
          createdAt: new Date().toISOString(),
          forkedFrom: parentId,
        })
        log(`fork mapped: ${childId} <- ${parentId}`)
      }),
    'storyforge: fork observer',
  )

  /** 程序化建一个 StoryForge 会话并投递开场消息。 */
  async function createStorySession(title) {
    const preset = await ctx.agentPresets.resolve(PRESET)
    const lease = await ctx.agentPresets.acquireScope(preset.id)
    try {
      mkdirSync(workspacePath, { recursive: true })
      const workspace = await ctx.workspaceRegistry.create(workspacePath)
      const sessionId = `${SESSION_PREFIX}${randomUUID()}`
      const handle = await ctx.agents.create({
        sessionId,
        meta: { cwd: workspace.path, agentPreset: preset.id },
        agentOptions: { provider: PROVIDER, model: MODEL },
        setup: async (agentCtx) => {
          await ctx.agentPresets.mount(agentCtx, preset.id)
        },
      })
      let attached = false
      try {
        await workspace.attachSession(sessionId)
        attached = true
        ctx.sessionTitle.rename(handle.agent.session, title)
        sessions.set(sessionId, { title, createdAt: new Date().toISOString() })
        handle.agent.followup(
          createUserMessage('（开始新故事）', {
            kind: OPENING_SOURCE_KIND,
            form: 'notice',
            summary: '开始新故事',
          }),
        )
      } catch (error) {
        if (attached) await workspace.detachSession(sessionId).catch(() => {})
        await handle.dispose().catch(() => {})
        throw error
      }
      log(`story session created: ${sessionId} (${title})`)
      return sessionId
    } finally {
      // 预设作用域租约只需覆盖创建过程（与 webhook 一致）。
      const release = lease?.[Symbol.asyncDispose] ?? lease?.[Symbol.dispose]
      if (typeof release === 'function') await release.call(lease)
    }
  }

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
      path: `${API_BASE}/sessions`,
      methods: ['GET', 'POST'],
      requestBody: 'buffered',
      fetch: async (request) => {
        if (request.method === 'GET') return json({ ok: true, sessions: sessions.all() })
        try {
          const body = await request.json().catch(() => ({}))
          const title = typeof body?.title === 'string' && body.title.trim() ? body.title.trim() : '新的故事'
          const sessionId = await createStorySession(title)
          return json({ ok: true, sessionId })
        } catch (error) {
          log(`create session failed: ${error?.stack ?? error}`)
          return json({ ok: false, error: String(error?.message ?? error) }, 500)
        }
      },
    },
    {
      path: `${API_BASE}/debug/calls`,
      methods: ['GET'],
      requestBody: 'buffered',
      fetch: async () => json({ ok: true, calls: callLog }),
    },
    {
      path: `${API_BASE}/debug/lifecycle`,
      methods: ['GET'],
      requestBody: 'buffered',
      fetch: async () => json({ ok: true, events: lifecycleEvents }),
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
