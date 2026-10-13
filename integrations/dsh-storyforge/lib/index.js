/**
 * dsh-plugin-storyforge — 宿主半边（PoC）
 *
 * P2 起 StoryForge 会话跑真实管线：适配器把本轮输入交给 Rust sidecar
 * （`crates/dsh-bridge`，见 `./bridge.js`），sidecar 跑与 Tauri 命令同一份生产代码；
 * 管线里每一次模型调用再反向发回插件，由插件用用户选定的底层模型执行
 * `ctx.llm.stream`。配置 `fakePipeline: true` 时退回 P0/P1 的假管线（不需要 Rust 与凭据）。
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
import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, statSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { basename, dirname, extname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { BridgeRpcError, createBridge } from './bridge.js'
import { MAX_CARD_BYTES, nextFreeName, readCardName, sanitizeCardFileName } from './cards.js'
import { renderableBlocks, splitForDsh } from './html.js'
import { panelModel, planInteraction } from './mvu.js'
import { classifyCall } from './roles.js'
import { fitSessionTitle, friendlyError, llmHint } from './text.js'

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

/**
 * DSH 历史里本轮输入之前已完成的写作轮数：一条真实用户输入后面跟着带正文的回复
 * 才算一轮（开场提示、注入的通知、失败或被中止没有正文的轮次都不算）。与 sidecar
 * 存档里已采纳的轮数对齐（`history_turns`）。
 */
function completedTurns(messages, current) {
  let turns = 0
  let pending = false
  for (const message of messages ?? []) {
    if (message === current) break
    if (message?.role === 'user') {
      const kind = message.source?.kind
      if (kind === undefined || kind === USER_SOURCE_KIND) pending = true
    } else if (message?.role === 'assistant' && pending && textBlocks(message).some((text) => text.trim())) {
      turns += 1
      pending = false
    }
  }
  return turns
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

async function* finishStop(outputTokens = 0, replayState) {
  yield* finishWithUsage({ inputTokens: 0, outputTokens }, replayState)
}

/** `replayState` 随 finish 块交给 DSH，DSH 原样存进这条回复（`source.replayState`）。 */
async function* finishWithUsage(usage, replayState) {
  yield { type: 'usage', usage }
  yield replayState === undefined
    ? { type: 'finish', reason: { kind: 'stop' } }
    : { type: 'finish', reason: { kind: 'stop' }, replayState }
}

/** 回放数据版本：浏览器半边认不出的版本当作没有。 */
const REPLAY_VERSION = 1

/**
 * 这条回复在 StoryForge 存档里的位置，写进 DSH 回放数据；浏览器半边把它发布成 Turn 级
 * 数据，Turn 尾部据此取展示文本。`nodeIndex` 供分叉后兜底（分叉换了节点 id、顺序不变）；
 * `blocks` 是从正文里拿掉、交给 Turn 尾部渲染的 HTML 块数（0 时尾部不取数据）。
 */
function storyforgeReplay({ campaignId, conversationId, nodeId, nodeIndex, blocks }) {
  if (!conversationId) return undefined
  const ref = { v: REPLAY_VERSION, campaignId, conversationId, blocks }
  if (nodeId) ref.nodeId = nodeId
  if (Number.isInteger(nodeIndex)) ref.nodeIndex = nodeIndex
  return { response: { storyforge: ref } }
}

/** sidecar 的 UsageTotals → DSH usage（同为不重叠口径）。 */
function toDshUsage(totals) {
  return {
    inputTokens: totals.input_tokens ?? 0,
    outputTokens: totals.output_tokens ?? 0,
    cacheReadTokens: totals.cache_read_tokens ?? 0,
    cacheWriteTokens: totals.cache_write_tokens ?? 0,
  }
}

function describeUsage(totals) {
  if (!totals) return ''
  const cached = totals.cache_read_tokens ? `，缓存命中 ${totals.cache_read_tokens}` : ''
  const failed = totals.failed_calls ? `（${totals.failed_calls} 次失败）` : ''
  return `${totals.calls} 次底层调用${failed}，输入 ${totals.input_tokens} / 输出 ${totals.output_tokens} tokens${cached}`
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

/** 一份 JSON 文件里的键值表（会话映射、设置），整份原子替换写入。 */
function createJsonStore(file) {
  let entries = {}
  if (existsSync(file)) {
    try {
      entries = JSON.parse(readFileSync(file, 'utf8'))
    } catch (error) {
      log(`${basename(file)} unreadable, starting empty: ${error}`)
    }
  }
  const save = () => {
    mkdirSync(dirname(file), { recursive: true })
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

//#region 模型调用（sidecar → 插件 → ctx.llm.stream）

const MAX_REPLAY_ENTRIES = 200

/**
 * 工具循环里带推理的模型（如 DeepSeek 思考模式）要求把上一步的推理原样带回；
 * StoryForge 重发的历史里没有这些，所以按第一个 tool-call id 记下每次响应的
 * replayState 与推理正文，重发同一条 assistant 消息时补回去。
 */
function createReplayCache() {
  const entries = new Map()
  return {
    get: (toolCallId) => entries.get(toolCallId),
    set(toolCallId, value) {
      entries.set(toolCallId, value)
      while (entries.size > MAX_REPLAY_ENTRIES) entries.delete(entries.keys().next().value)
    },
  }
}

/** sidecar 的中性消息 → DSH 请求消息。开头连续的 system 合并进 `system`。 */
function toDshRequest(list, target, replayCache) {
  const systemParts = []
  const messages = []
  let leading = true
  for (const message of list ?? []) {
    const text = typeof message.content === 'string' ? message.content : ''
    if (message.role === 'system' && leading) {
      systemParts.push(text)
      continue
    }
    leading = false
    switch (message.role) {
      case 'system':
        // 中途的系统提示：DSH 适配器只读开头的 system，转成一条用户角色提示。
        messages.push({ role: 'user', content: [{ type: 'text', text: `【系统提示】\n${text}` }] })
        break
      case 'user':
        messages.push({ role: 'user', content: [{ type: 'text', text }] })
        break
      case 'assistant': {
        const calls = message.tool_calls ?? []
        const replay = calls.length > 0 ? replayCache.get(calls[0].id) : undefined
        const sameRoute = replay?.provider === target.provider && replay?.model === target.model
        const content = []
        if (sameRoute && replay.reasoning) content.push({ type: 'reasoning', text: replay.reasoning })
        if (text) content.push({ type: 'text', text })
        for (const call of calls) {
          content.push({ type: 'tool-call', id: call.id, name: call.name, arguments: call.arguments ?? '{}' })
        }
        if (content.length === 0) content.push({ type: 'text', text: '' })
        const source = { kind: 'model', provider: target.provider, model: target.model }
        if (sameRoute && replay.replayState !== undefined) source.replayState = replay.replayState
        messages.push({ id: randomUUID(), role: 'assistant', content, source })
        break
      }
      case 'tool':
        messages.push({
          id: randomUUID(),
          role: 'tool',
          toolCallId: message.tool_call_id,
          content: [{ type: 'text', text }],
          source: { kind: 'tool', callId: message.tool_call_id },
        })
        break
      default:
        throw new BridgeRpcError('bad_message', `未知消息角色 ${message.role}`)
    }
  }
  return { system: systemParts.join('\n\n'), messages }
}

const FINISH_REASONS = { stop: 'stop', 'tool-calls': 'tool_calls', 'max-tokens': 'length' }

/**
 * 执行 sidecar 的一次 `llm.stream`。重试由 sidecar 侧的 RetryingClient 负责：
 * 这里把 DSH 的失败（code / status / retry-after）原样交回去分类。
 */
async function serveLlm(ctx, target, replayCache, params, { signal, onChunk }) {
  if (!target?.provider || !target?.model) {
    throw new BridgeRpcError('no_model', '还没有选 StoryForge 的底层模型：到左侧栏 StoryForge 首页选择后再试。')
  }
  const { system, messages } = toDshRequest(params.messages, target, replayCache)
  const options = { provider: target.provider, model: target.model, messages, signal }
  if (system) options.system = system
  if (Array.isArray(params.tools) && params.tools.length > 0) {
    options.tools = params.tools.map((tool) => ({
      name: tool.name,
      description: tool.description ?? '',
      parameters: tool.parameters ?? { type: 'object' },
    }))
  }
  if (typeof params.temperature === 'number') options.temperature = params.temperature
  if (typeof params.max_tokens === 'number') options.maxTokens = params.max_tokens

  let deltaText = ''
  let blockText = null
  let reasoning = ''
  const toolCalls = []
  let usage = null
  let finish = null
  for await (const chunk of ctx.llm.stream(options)) {
    switch (chunk.type) {
      case 'text-delta':
        deltaText += chunk.text
        onChunk({ type: 'text', text: chunk.text })
        break
      case 'reasoning-delta':
        reasoning += chunk.text
        onChunk({ type: 'reasoning', text: chunk.text })
        break
      case 'block-end':
        if (chunk.block?.type === 'tool-call') {
          toolCalls.push({ id: chunk.block.id, name: chunk.block.name, arguments: chunk.block.arguments })
        } else if (chunk.block?.type === 'text') {
          blockText = (blockText ?? '') + chunk.block.text
        }
        break
      case 'usage':
        usage = chunk.usage
        break
      case 'finish':
        finish = chunk
        break
      default:
        break
    }
  }
  const kind = finish?.reason?.kind
  if (kind === 'error' || kind === 'aborted') {
    const failure = finish.reason.failure ?? {}
    throw new BridgeRpcError(kind === 'aborted' ? 'aborted' : failure.code || 'error', failure.message ?? kind, {
      status: failure.status,
      retryAfterMs: failure.providerRetryAfterMs,
    })
  }
  if (!finish) throw new BridgeRpcError('stream_incomplete', '模型流没有正常结束')
  if (finish.replayState !== undefined && toolCalls.length > 0) {
    replayCache.set(toolCalls[0].id, { ...target, replayState: finish.replayState, reasoning })
  }
  return {
    content: blockText ?? deltaText,
    reasoning: reasoning || null,
    tool_calls: toolCalls,
    finish_reason: FINISH_REASONS[kind] ?? kind ?? null,
    usage: usage && {
      input_tokens: usage.inputTokens ?? 0,
      output_tokens: usage.outputTokens ?? 0,
      cache_read_tokens: usage.cacheReadTokens ?? 0,
      cache_write_tokens: usage.cacheWriteTokens ?? 0,
      total_tokens: usage.totalTokens,
    },
  }
}

//#endregion

//#region 真实管线适配器（StoryForge 会话 → sidecar turn.run）

/** 管线事件 → 思考块里的一段进度（返回 null 表示不显示）。 */
function describeEvent(event) {
  switch (event?.type) {
    case 'Started':
      return '开始本轮写作\n'
    case 'DirectorStarted':
      return '【导演】规划本轮场景…\n'
    case 'DirectorDone':
      return `【导演】${event.scene_brief}（${event.subagent_count} 个角色出场）\n`
    case 'SubagentStarted':
      return `【角色】${event.character_id} 开始表演（${event.index + 1}/${event.total}）\n`
    case 'SubagentDone':
      return `【角色】${event.character_id}：\n${event.full_text}\n\n`
    case 'WriterStarted':
      return '【执笔】\n'
    case 'EditorStarted':
      return '【编剧】\n'
    case 'WriterProgress':
    case 'EditorProgress':
      return event.delta
    case 'QualityChecked':
      return event.passed
        ? '\n【质量门】通过\n'
        : `\n【质量门】${event.error_count} 个错误，${event.warning_count} 个提醒${event.warnings?.length ? `：${event.warnings.join('；')}` : ''}\n`
    case 'Error':
      return `\n【错误】${event.message}\n`
    default:
      return null
  }
}

/** 推 / 拉之间的异步队列：`close(value)` 结束迭代，`fail(error)` 让迭代抛出。 */
function createEventQueue() {
  const items = []
  let wake = null
  let done = null
  const notify = () => {
    const resolveWake = wake
    wake = null
    resolveWake?.()
  }
  return {
    push(item) {
      items.push(item)
      notify()
    },
    close(value) {
      done = { value }
      notify()
    },
    fail(error) {
      done = { error }
      notify()
    },
    async *[Symbol.asyncIterator]() {
      while (true) {
        if (items.length > 0) {
          yield items.shift()
          continue
        }
        if (done) {
          if (done.error) throw done.error
          return
        }
        await new Promise((resolveWake) => {
          wake = resolveWake
        })
      }
    },
    result: () => done?.value,
  }
}

function abortError(signal) {
  return signal?.reason instanceof Error ? signal.reason : new Error('aborted')
}

/**
 * StoryForge 会话的一次模型调用：开场返回卡的开场白；用户行动交给 sidecar 跑一轮，
 * 管线进度流进思考块，成文（质量门 / autofix 之后的最终正文）作为回复正文。
 */
function createPipelineStream({ sessions, bridge, runOptions, onTurnRan }) {
  /**
   * 分叉出的子会话第一次写作时，把它映射到来源存档的分支（StoryForge 只支持从最新
   * 一轮分叉；从更早的位置分叉会被 sidecar 以 out_of_sync 拒绝并说明原因）。
   */
  async function mapFork(sessionId, session, historyTurns) {
    const parent = sessions.get(session.forkedFrom)
    if (!parent?.campaignId) {
      throw new Error('分叉来源会话还没有关联 StoryForge 存档，无法建立分支。')
    }
    const forked = await bridge.request('campaign.fork', {
      campaign_id: parent.campaignId,
      history_turns: historyTurns,
      name: session.title,
    })
    sessions.set(sessionId, {
      ...session,
      campaignId: forked.campaign_id,
      conversationId: forked.conversation_id,
      cardName: parent.cardName,
      cardFile: parent.cardFile,
      sourceCampaignId: forked.source_campaign_id,
      forkTurns: forked.turns,
      turns: forked.turns,
    })
    log(`fork mapped: ${sessionId} -> campaign ${forked.campaign_id} (branch of ${forked.source_campaign_id} at turn ${forked.turns})`)
    return forked.campaign_id
  }

  return async function* pipelineStream(options) {
    const { signal } = options
    const session = options.sessionId ? sessions.get(options.sessionId) : undefined
    if (options.purpose === 'session-title') {
      yield* streamBlock(0, 'text', session?.title ?? 'StoryForge 故事', signal, 64, 0)
      yield* finishStop()
      return
    }
    if (options.purpose === 'compaction') {
      // StoryForge 自己管理上下文（摘要 / 远记忆）；DSH 的压缩不需要摘要正文。
      yield* finishStop()
      return
    }

    const turn = currentTurn(options.messages)
    if (turn.opening) {
      const split = splitForDsh(session?.openingDisplay ?? session?.opening)
      const opening = split.body || '（这张卡没有开场白，直接写下你的第一步行动吧。）'
      yield* streamBlock(0, 'text', opening, signal, 64, 0)
      yield* finishStop(
        opening.length,
        storyforgeReplay({
          campaignId: session?.campaignId,
          conversationId: session?.conversationId,
          nodeId: session?.openingNodeId,
          // 早于 openingNodeIndex 的会话：开场白总是对话的第一个节点。
          nodeIndex: session?.openingNodeIndex ?? 0,
          blocks: split.blocks,
        }),
      )
      return
    }
    const intent = textBlocks(turn.message).join('\n').trim()
    if (!intent) throw new Error('本轮输入为空')
    if (signal?.aborted) throw abortError(signal)
    const historyTurns = completedTurns(options.messages, turn.message)
    let campaignId = session?.campaignId
    if (!campaignId && session?.forkedFrom) {
      campaignId = await mapFork(options.sessionId, session, historyTurns)
    }
    if (!campaignId) {
      throw new Error(
        '这个会话没有关联 StoryForge 存档（从新会话页直接选了 StoryForge 预设）。请到左侧栏 StoryForge 首页新建故事。',
      )
    }

    // sidecar 一次只跑一轮（上一轮的后台收尾也算）：要排队时先在思考块里说明，免得像卡住了。
    const queued = await bridge
      .request('bridge.status')
      .then((status) => status?.busy === true)
      .catch(() => false)
    if (signal?.aborted) throw abortError(signal)

    const queue = createEventQueue()
    const unsubscribe = bridge.subscribe((method, params) => {
      if (method === 'turn.event' && params?.campaign_id === campaignId) queue.push(params.event)
    })
    // 只取消本会话的这一轮：排队中的放弃，成文中的中止；上一轮的收尾照常完成。
    const onAbort = () => {
      bridge.request('turn.cancel', { campaign_id: campaignId }).catch(() => {})
    }
    signal?.addEventListener('abort', onAbort, { once: true })
    const { toolMode, generationMode } = runOptions()
    bridge
      .request('bridge.configure', { tool_mode: toolMode })
      .then(() =>
        bridge.request('turn.run', {
          campaign_id: campaignId,
          intent,
          generation_mode: generationMode === 'auto' ? undefined : generationMode,
          history_turns: historyTurns,
        }),
      )
      .then(
        (result) => {
          onTurnRan?.(campaignId, historyTurns + 1)
          queue.close(result)
        },
        (error) => queue.fail(error),
      )
    try {
      let reasoning = ''
      yield { type: 'block-start', index: 0, blockType: 'reasoning' }
      if (queued) {
        reasoning = '【排队】写作引擎正忙（上一轮在后台收尾，或另一个故事正在写），空出来后自动开始本轮；现在停止只放弃这一轮。\n'
        yield { type: 'reasoning-delta', index: 0, text: reasoning }
      }
      try {
        for await (const event of queue) {
          const line = describeEvent(event)
          if (!line) continue
          reasoning += line
          yield { type: 'reasoning-delta', index: 0, text: line }
        }
      } catch (error) {
        if (signal?.aborted) throw abortError(signal)
        throw friendlyError(error)
      }
      const result = queue.result()
      const mode = GENERATION_MODE_LABELS[result?.generation_mode] ?? result?.generation_mode ?? '—'
      const summary = `\n【本轮】${mode}；${describeUsage(result?.usage)}。后处理在后台继续。\n`
      reasoning += summary
      yield { type: 'reasoning-delta', index: 0, text: summary }
      yield { type: 'block-end', index: 0, block: { type: 'reasoning', text: reasoning } }
      // 正文用展示正则之后的文本，去掉 Turn 尾部会渲染的 HTML 块。
      const split = splitForDsh(result?.display ?? result?.text)
      const text = split.body
      yield* streamBlock(1, 'text', text, signal, 64, 0)
      const replay = storyforgeReplay({
        campaignId: result?.campaign_id,
        conversationId: result?.conversation_id,
        nodeId: result?.node_id,
        nodeIndex: result?.node_index,
        blocks: split.blocks,
      })
      yield* result?.usage ? finishWithUsage(toDshUsage(result.usage), replay) : finishStop(text.length, replay)
    } finally {
      unsubscribe()
      signal?.removeEventListener('abort', onAbort)
    }
  }
}

//#endregion

const MAX_TURN_EVENTS = 200
const MAX_SETTLE_REPORTS = 20
const MAX_LLM_LOG = 100
/** 生成模式（auto = StoryForge 自动路由；其余显式选择，不再二次确认）。 */
const GENERATION_MODE_LABELS = {
  auto: '自动路由',
  continuation: '续写（单次执笔）',
  duet: '双人戏',
  big_scene: '群像（并行子 Agent）',
  sequential_crew: '群像（顺序子 Agent）',
}
/** 工具协议：原生 function calling，或 StoryForge 的文本降级（模型不支持工具时）。 */
const TOOL_MODE_LABELS = { native: '原生工具调用', text_fallback: '文本降级' }
const CARD_EXTENSIONS = new Set(['.png', '.json'])
const PLUGIN_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
/** 浏览器半边净化卡片 HTML 用的 DOMPurify（ES 模块，经 `/vendor/purify.js` 动态 import）。 */
const PURIFY_FILE = join(PLUGIN_ROOT, 'lib', 'vendor', 'purify.es.mjs')
/** 一次 `/display` 最多查的节点数（与 sidecar 的 MAX_DISPLAY_REFS 一致）。 */
const MAX_DISPLAY_REFS = 200

/** 浏览器半边发来的一条回放数据（存档位置）→ sidecar 查询；格式不对返回 null。 */
function displayQuery(ref) {
  if (typeof ref?.key !== 'string' || !ref.key || ref.key.length > 256) return null
  if (typeof ref.conversationId !== 'string' || !ref.conversationId) return null
  if (ref.nodeId !== undefined && typeof ref.nodeId !== 'string') return null
  if (ref.nodeIndex !== undefined && !(Number.isInteger(ref.nodeIndex) && ref.nodeIndex >= 0)) return null
  return { key: ref.key, conversation_id: ref.conversationId, node_id: ref.nodeId, node_index: ref.nodeIndex }
}
const DEFAULT_BRIDGE_BIN = resolve(
  PLUGIN_ROOT,
  '../../target/debug',
  process.platform === 'win32' ? 'storyforge-dsh-bridge.exe' : 'storyforge-dsh-bridge',
)

export function apply(ctx, config) {
  const dshHome = process.env.DSH_HOME || join(homedir(), '.dsh')
  const dataDir = config?.dataDir || join(dshHome, 'storyforge')
  const workspacePath = config?.workspacePath || join(dataDir, 'stories')
  const cardsDir = config?.cardsDir || join(dataDir, 'cards')
  const fake = config?.fakePipeline === true
  const callLog = []
  const sessions = createJsonStore(join(dataDir, 'sessions.json'))
  const settings = createJsonStore(join(dataDir, 'settings.json'))
  const replayCache = createReplayCache()
  /** 最近的管线事件与收尾报告（首页调试用）。 */
  const turnEvents = []
  /** 收尾报告落盘（首页「最近的收尾」与故事进度跨 DSH 重启保留）。 */
  const settleStore = createJsonStore(join(dataDir, 'settles.json'))
  const settleReports = Array.isArray(settleStore.get('reports')) ? settleStore.get('reports') : []
  /** 正文已出、等收尾的轮次：campaignId → 接受后的已采纳轮数。 */
  const pendingTurns = new Map()
  /** sidecar 经插件发出的每一次底层模型调用（DSH 侧的调用记录）。 */
  const llmLog = []
  let llmSeq = 0
  const runOptions = () => ({
    toolMode: TOOL_MODE_LABELS[settings.get('toolMode')] ? settings.get('toolMode') : 'native',
    generationMode: GENERATION_MODE_LABELS[settings.get('generationMode')] ? settings.get('generationMode') : 'auto',
  })

  async function serveLogged(params, io) {
    const target = settings.get('model')
    const entry = {
      seq: ++llmSeq,
      at: new Date().toISOString(),
      model: target ? `${target.provider}/${target.model}` : null,
      role: classifyCall(params?.messages),
      messages: params?.messages?.length ?? 0,
      tools: params?.tools?.length ?? 0,
      status: 'running',
    }
    llmLog.unshift(entry)
    llmLog.length = Math.min(llmLog.length, MAX_LLM_LOG)
    const started = Date.now()
    try {
      const result = await serveLlm(ctx, target, replayCache, params, io)
      Object.assign(entry, {
        status: 'ok',
        ms: Date.now() - started,
        finish: result.finish_reason,
        toolCalls: result.tool_calls.length,
        usage: result.usage,
      })
      return result
    } catch (error) {
      const aborted = error?.code === 'aborted' || error?.name === 'AbortError' || io.signal?.aborted
      Object.assign(entry, {
        status: aborted ? 'aborted' : 'error',
        ms: Date.now() - started,
        error: [error?.code, String(error?.message ?? error)].filter(Boolean).join(': '),
      })
      throw error
    }
  }

  const bridge = createBridge({
    binPath: config?.bridgeBin || DEFAULT_BRIDGE_BIN,
    dataDir: config?.bridgeDataDir || join(dataDir, 'bridge-data'),
    logFile: join(dataDir, 'logs', 'bridge.log'),
    serveLlm: serveLogged,
    log,
  })
  ctx.effect(
    () =>
      bridge.subscribe((method, params) => {
        if (method === 'turn.event') {
          turnEvents.unshift({ at: new Date().toISOString(), campaignId: params?.campaign_id, type: params?.event?.type })
          turnEvents.length = Math.min(turnEvents.length, MAX_TURN_EVENTS)
        } else if (method === 'turn.settled') {
          const report = { at: new Date().toISOString(), ...params?.report }
          settleReports.unshift(report)
          settleReports.length = Math.min(settleReports.length, MAX_SETTLE_REPORTS)
          settleStore.set('reports', settleReports)
          recordStoryProgress(report)
          log(`turn settled: ${JSON.stringify(params?.report)}`)
        }
      }),
    'storyforge: bridge event log',
  )
  ctx.effect(() => () => void bridge.stop(), 'storyforge: bridge process')

  /** 收尾后把轮数与最近一次结果记到故事登记里（首页「我的故事」不必拉起 sidecar）。 */
  function recordStoryProgress(report) {
    const campaignId = report.campaign_id
    if (!campaignId) return
    const turns = pendingTurns.get(campaignId)
    pendingTurns.delete(campaignId)
    const accepted = typeof report.accept === 'string' && report.accept.startsWith('accepted')
    for (const [sessionId, story] of Object.entries(sessions.all())) {
      if (story?.campaignId !== campaignId) continue
      sessions.set(sessionId, {
        ...story,
        ...(accepted && turns != null ? { turns } : {}),
        lastSettle: { at: report.at, accept: report.accept, postprocess: report.postprocess, accept_error: report.accept_error ?? null },
      })
    }
  }

  // registerAdapter 内部经调用方的 ctx.effect 登记，插件卸载时自动撤销路由。
  ctx.llm.registerAdapter(
    [PROVIDER],
    createAdapter({
      displayName: 'StoryForge',
      models: [],
      contextWindow: CONTEXT_WINDOW,
      streamImpl: fake
        ? fakePipelineStream
        : createPipelineStream({ sessions, bridge, runOptions, onTurnRan: (campaignId, turns) => pendingTurns.set(campaignId, turns) }),
      callLog,
    }),
  )
  log(`adapter registered: provider=${PROVIDER} model=${MODEL} pipeline=${fake ? 'fake' : 'sidecar'}`)

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

  /** 卡名缓存：文件名 → { mtimeMs, size, name }（只在文件变了时重新解析）。 */
  const cardNames = new Map()

  function cardName(file, stat) {
    const cached = cardNames.get(file)
    if (cached && cached.mtimeMs === stat.mtimeMs && cached.size === stat.size) return cached.name
    let name = null
    try {
      name = readCardName(readFileSync(join(cardsDir, file)), extname(file))
    } catch (error) {
      log(`card name unreadable: ${file}: ${error?.message ?? error}`)
    }
    cardNames.set(file, { mtimeMs: stat.mtimeMs, size: stat.size, name })
    return name
  }

  function listCards() {
    if (!existsSync(cardsDir)) return []
    return readdirSync(cardsDir)
      .filter((file) => CARD_EXTENSIONS.has(extname(file).toLowerCase()))
      .map((file) => {
        const stat = statSync(join(cardsDir, file))
        return { file, size: stat.size, name: cardName(file, stat) }
      })
  }

  /**
   * 首页「导入角色卡」：请求体是卡文件原始字节，`?name=` 是原文件名。只收能读出卡名的
   * PNG / JSON 卡；同名不覆盖，自动编号。
   */
  async function importCard(request) {
    const url = new URL(request.url)
    const file = sanitizeCardFileName(url.searchParams.get('name'))
    if (!file) throw new Error('只支持 .png 或 .json 角色卡')
    const declared = Number(request.headers.get('content-length'))
    if (Number.isFinite(declared) && declared > MAX_CARD_BYTES) throw new Error(`角色卡超过 ${MAX_CARD_BYTES / 1024 / 1024} MB`)
    const bytes = Buffer.from(await request.arrayBuffer())
    if (bytes.length === 0) throw new Error('文件是空的')
    if (bytes.length > MAX_CARD_BYTES) throw new Error(`角色卡超过 ${MAX_CARD_BYTES / 1024 / 1024} MB`)
    const name = readCardName(bytes, extname(file))
    if (!name) throw new Error('没有在文件里找到角色卡数据（PNG 需要带 chara / ccv3 信息，JSON 需要有角色名）')
    mkdirSync(cardsDir, { recursive: true })
    const saved = nextFreeName(file, (candidate) => existsSync(join(cardsDir, candidate)))
    writeFileSync(join(cardsDir, saved), bytes, { flag: 'wx' })
    log(`card imported: ${saved} (${name}, ${bytes.length} bytes)`)
    return { file: saved, name, size: bytes.length }
  }

  /** 「测试连接」：用选定的底层模型发一条很短的请求，报告耗时或可操作的错误。 */
  async function testModel(target) {
    const started = Date.now()
    const controller = new AbortController()
    const timer = setTimeout(() => controller.abort(new Error('60 秒内没有回应')), 60_000)
    try {
      const result = await serveLlm(
        ctx,
        target,
        replayCache,
        {
          messages: [
            { role: 'system', content: '这是连接测试。只回复两个字：收到' },
            { role: 'user', content: '测试连接' },
          ],
          max_tokens: 64,
        },
        { signal: controller.signal, onChunk: () => {} },
      )
      return { ok: true, ms: Date.now() - started, reply: String(result.content ?? '').slice(0, 40), usage: result.usage }
    } catch (error) {
      const message = [error?.code, String(error?.message ?? error)].filter(Boolean).join(': ')
      return { ok: false, ms: Date.now() - started, error: llmHint(message) ?? message, detail: message }
    } finally {
      clearTimeout(timer)
    }
  }

  /** 首页传来的卡文件名 → cardsDir 下的绝对路径（只接受目录里真实存在的文件名）。 */
  function resolveCard(file) {
    if (typeof file !== 'string' || !listCards().some((card) => card.file === file)) {
      throw new Error(`角色卡不在 ${cardsDir} 里：${file}`)
    }
    return join(cardsDir, file)
  }

  /** 可选的底层模型（排除 StoryForge 自己与调试回显）。 */
  async function listModelChoices() {
    const providers = ctx.llm.listProviders().filter((info) => info.id !== PROVIDER && info.id !== ECHO_PROVIDER)
    return Promise.all(
      providers.map(async (info) => {
        const name = info.name ?? info.id
        try {
          const models = await ctx.llm.listModels(info.id)
          return { provider: info.id, name, models: models.map((model) => ({ id: model.id, name: model.name ?? model.id })) }
        } catch (error) {
          return { provider: info.id, name, models: [], error: String(error?.message ?? error) }
        }
      }),
    )
  }

  /** 程序化建一个 StoryForge 会话并投递开场消息。 */
  async function createStorySession(title, story) {
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
        ctx.sessionTitle.rename(handle.agent.session, fitSessionTitle(title))
        sessions.set(sessionId, { title, createdAt: new Date().toISOString(), ...story })
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
      log(`story session created: ${sessionId} (${title}) campaign=${story.campaignId ?? '-'}`)
      return sessionId
    } finally {
      // 预设作用域租约只需覆盖创建过程（与 webhook 一致）。
      const release = lease?.[Symbol.asyncDispose] ?? lease?.[Symbol.dispose]
      if (typeof release === 'function') await release.call(lease)
    }
  }

  /**
   * Turn 尾部的展示数据：浏览器半边把可见 Turn 的回放数据（存档位置）合批发来，这里经
   * sidecar 取当前展示文本、切出要渲染的 HTML 块。展示正则的深度随对话变长而变，不缓存。
   * 分叉继承的轮次由 sidecar 按位置到本会话当前存档（分支）的对话里取。
   */
  async function displayItems(sessionId, refs) {
    const session = typeof sessionId === 'string' ? sessions.get(sessionId) : undefined
    const queries = refs.map(displayQuery).filter(Boolean)
    if (fake || !session || queries.length === 0) return {}
    const { items } = await bridge.request('conversation.display', {
      refs: queries,
      campaign_id: session.campaignId,
    })
    return Object.fromEntries(
      Object.entries(items ?? {}).map(([key, item]) => {
        const blocks = item ? renderableBlocks(item.display_content) : []
        return [key, blocks.length > 0 ? { blocks } : null]
      }),
    )
  }

  /** MVU 面板对应的存档：本会话已关联存档时返回 id，否则返回 null（分叉子会话第一次发送前）。 */
  function mvuCampaign(sessionId) {
    const session = typeof sessionId === 'string' ? sessions.get(sessionId) : undefined
    if (!session) throw httpError('不是 StoryForge 故事会话', 404)
    return session.campaignId ?? null
  }

  function httpError(message, status) {
    const error = new Error(message)
    error.status = status
    return error
  }

  /** 这次操作里最近一条底层模型调用错误（分析器失败时会静默降级，这里把原因找回来）。 */
  function llmErrorSince(seqBefore) {
    const detail = llmLog.find((entry) => entry.seq > seqBefore && entry.status === 'error')?.error ?? null
    return detail ? (llmHint(detail) ?? detail) : null
  }

  async function mvuPanel(sessionId) {
    const campaignId = mvuCampaign(sessionId)
    if (fake) return { state: 'none', reason: 'fake' }
    if (!campaignId) return { state: 'none', reason: 'unlinked' }
    return panelModel(await bridge.request('mvu.status', { campaign_id: campaignId }))
  }

  async function mvuAnalyze(sessionId) {
    const campaignId = mvuCampaign(sessionId)
    if (fake || !campaignId) throw httpError('这个会话还没有关联存档', 400)
    await bridge.request('bridge.configure', { tool_mode: runOptions().toolMode })
    const seqBefore = llmSeq
    const analyzed = await bridge.request('mvu.analyze', { campaign_id: campaignId })
    const error = analyzed.empty ? llmErrorSince(seqBefore) : null
    log(`mvu analyzed: campaign=${campaignId} empty=${analyzed.empty}${error ? ` error=${error}` : ''}`)
    return { model: panelModel(analyzed.status), empty: analyzed.empty, usage: analyzed.usage, error }
  }

  async function mvuApply(sessionId) {
    const campaignId = mvuCampaign(sessionId)
    if (fake || !campaignId) throw httpError('这个会话还没有关联存档', 400)
    const status = await bridge.request('mvu.apply', { campaign_id: campaignId })
    log(`mvu schema applied: campaign=${campaignId}`)
    return { model: panelModel(status) }
  }

  /**
   * 点交互按钮：按最新变量重新算计划（桌面 `dispatchMvuInteraction` 同口径），写入交给
   * sidecar。浏览器只报按钮位置与文字——写哪些变量、写什么值都由卡的翻译产物决定。
   * 有写入失败时不返回下一轮提示（桌面同样跳过下一轮生成）。
   */
  async function mvuInteract(sessionId, index, label) {
    const campaignId = mvuCampaign(sessionId)
    if (fake || !campaignId) throw httpError('这个会话还没有关联存档', 400)
    const status = await bridge.request('mvu.status', { campaign_id: campaignId })
    const mapping = status.translation?.interactions?.[index]
    if (!mapping || mapping.element_label !== label) throw httpError('交互按钮已经变了，刷新面板后再试', 409)
    const { plan, writes, rejected } = planInteraction(status, mapping)
    let written = { results: [], status }
    if (writes.length > 0) {
      written = await bridge.request('variables.set', {
        campaign_id: campaignId,
        writes: writes.map((write) => ({
          scope: write.scope,
          instance_id: write.instanceId ?? undefined,
          key: write.key,
          value: write.value,
        })),
      })
    }
    const results = [
      ...written.results.map((result) => ({ ok: result.ok, scope: result.scope, key: result.key, error: result.error ?? null })),
      ...rejected.map((result) => ({ ok: false, scope: result.scope, key: result.key, error: result.error })),
    ]
    const failed = results.filter((result) => !result.ok)
    log(
      `mvu interaction "${plan.label}": campaign=${campaignId} writes=${results.length} failed=${failed.length} hints=${plan.hints.length} skipped=${plan.skipped.length}`,
    )
    return {
      label: plan.label,
      results,
      failed: failed.length,
      hints: failed.length > 0 ? [] : plan.hints,
      skipped: plan.skipped,
      model: panelModel(written.status),
    }
  }

  const mvuRoute = (suffix, handler) => ({
    path: `${API_BASE}/mvu${suffix}`,
    methods: ['POST'],
    requestBody: 'buffered',
    fetch: async (request) => {
      const body = await request.json().catch(() => ({}))
      try {
        return json({ ok: true, ...(await handler(body)) })
      } catch (error) {
        log(`mvu${suffix || ''} failed: ${error?.message ?? error}`)
        return failure(error, error?.status ?? 500)
      }
    },
  })

  let purifySource = null

  const json = (body, status = 200) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { 'content-type': 'application/json; charset=utf-8', 'cache-control': 'no-store' },
    })
  const failure = (error, status = 500) => json({ ok: false, error: String(error?.message ?? error) }, status)

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
          const requestedTitle = typeof body?.title === 'string' ? body.title.trim() : ''
          if (fake) {
            const sessionId = await createStorySession(requestedTitle || '新的故事', {})
            return json({ ok: true, sessionId })
          }
          await bridge.request('bridge.configure', { tool_mode: runOptions().toolMode })
          const seqBefore = llmSeq
          const created = await bridge.request('campaign.create', {
            card_path: resolveCard(body?.card),
            name: requestedTitle || undefined,
          })
          // 角色识别降级时，把这次开档里最后一个底层调用错误记成原因（首页与右栏显示）。
          const extractionErrorDetail =
            created.extraction_status === 'fallback'
              ? (llmLog.find((entry) => entry.seq > seqBefore && entry.status === 'error')?.error ?? null)
              : null
          const extractionError = extractionErrorDetail && (llmHint(extractionErrorDetail) ?? extractionErrorDetail)
          const title = requestedTitle || created.card_name || '新的故事'
          const sessionId = await createStorySession(title, {
            campaignId: created.campaign_id,
            conversationId: created.conversation_id,
            cardName: created.card_name,
            cardFile: body.card,
            opening: created.opening,
            openingDisplay: created.opening_display,
            openingNodeId: created.opening_node_id ?? undefined,
            openingNodeIndex: created.opening_node_index ?? undefined,
            extractionStatus: created.extraction_status,
            extractionError,
            extractionErrorDetail,
            turns: 0,
          })
          return json({
            ok: true,
            sessionId,
            campaign: created,
            warning: extractionError ? `故事已建好，但角色识别失败，先用兜底定义开档。${extractionError}` : null,
          })
        } catch (error) {
          log(`create session failed: ${error?.stack ?? error}`)
          return failure(error)
        }
      },
    },
    {
      path: `${API_BASE}/cards`,
      methods: ['GET'],
      requestBody: 'buffered',
      fetch: async () => json({ ok: true, cardsDir, cards: listCards(), maxBytes: MAX_CARD_BYTES }),
    },
    {
      path: `${API_BASE}/cards/import`,
      methods: ['POST'],
      requestBody: 'buffered',
      fetch: async (request) => {
        try {
          return json({ ok: true, card: await importCard(request) })
        } catch (error) {
          log(`card import failed: ${error?.message ?? error}`)
          return failure(error, 400)
        }
      },
    },
    {
      path: `${API_BASE}/models/test`,
      methods: ['POST'],
      requestBody: 'buffered',
      fetch: async () => {
        const target = settings.get('model')
        if (!target?.provider || !target?.model) return failure('先选底层模型', 400)
        const result = await testModel(target)
        log(`model test ${target.provider}/${target.model}: ${result.ok ? `ok ${result.ms}ms` : result.detail}`)
        return json({ ...result, ok: true, passed: result.ok, model: `${target.provider}/${target.model}` })
      },
    },
    {
      path: `${API_BASE}/models`,
      methods: ['GET'],
      requestBody: 'buffered',
      fetch: async () => {
        try {
          return json({
            ok: true,
            selected: settings.get('model') ?? null,
            ...runOptions(),
            generationModes: GENERATION_MODE_LABELS,
            toolModes: TOOL_MODE_LABELS,
            providers: await listModelChoices(),
          })
        } catch (error) {
          return failure(error)
        }
      },
    },
    {
      path: `${API_BASE}/settings`,
      methods: ['GET', 'POST'],
      requestBody: 'buffered',
      fetch: async (request) => {
        if (request.method === 'GET') return json({ ok: true, settings: settings.all() })
        const body = await request.json().catch(() => ({}))
        if (body?.generationMode !== undefined) {
          if (!GENERATION_MODE_LABELS[body.generationMode]) return failure(`未知生成模式 ${body.generationMode}`, 400)
          settings.set('generationMode', body.generationMode)
          log(`generation mode set: ${body.generationMode}`)
        }
        if (body?.toolMode !== undefined) {
          if (!TOOL_MODE_LABELS[body.toolMode]) return failure(`未知工具协议 ${body.toolMode}`, 400)
          settings.set('toolMode', body.toolMode)
          log(`tool mode set: ${body.toolMode}`)
        }
        if (body?.provider !== undefined || body?.model !== undefined) {
          const provider = typeof body?.provider === 'string' ? body.provider : ''
          const model = typeof body?.model === 'string' ? body.model : ''
          if (!provider || !model || provider === PROVIDER) return failure('需要 provider 与 model', 400)
          settings.set('model', { provider, model })
          log(`underlying model set: ${provider}/${model}`)
        }
        return json({ ok: true, settings: settings.all() })
      },
    },
    {
      path: `${API_BASE}/status`,
      methods: ['GET'],
      requestBody: 'buffered',
      fetch: async () => {
        const info = bridge.info()
        let status = null
        let statusError = null
        if (info.running) {
          try {
            status = await bridge.request('bridge.status')
          } catch (error) {
            statusError = String(error?.message ?? error)
          }
        }
        return json({
          ok: true,
          pipeline: fake ? 'fake' : 'sidecar',
          bridge: info,
          status,
          statusError,
          model: settings.get('model') ?? null,
          ...runOptions(),
          settles: settleReports,
          sessions: sessions.all(),
          events: turnEvents.slice(0, 50),
          llmCalls: llmLog.slice(0, 50),
          llmCallCount: llmSeq,
        })
      },
    },
    {
      path: `${API_BASE}/display`,
      methods: ['POST'],
      requestBody: 'buffered',
      fetch: async (request) => {
        const body = await request.json().catch(() => ({}))
        if (!Array.isArray(body?.refs)) return failure('refs 应为数组', 400)
        if (body.refs.length > MAX_DISPLAY_REFS) return failure(`一次最多查 ${MAX_DISPLAY_REFS} 条`, 400)
        try {
          return json({ ok: true, items: await displayItems(body.sessionId, body.refs) })
        } catch (error) {
          log(`display failed: ${error?.message ?? error}`)
          return failure(error)
        }
      },
    },
    mvuRoute('', (body) => mvuPanel(body?.sessionId).then((model) => ({ model }))),
    mvuRoute('/analyze', (body) => mvuAnalyze(body?.sessionId)),
    mvuRoute('/apply', (body) => mvuApply(body?.sessionId)),
    mvuRoute('/interact', (body) => {
      if (!Number.isInteger(body?.index) || body.index < 0 || typeof body?.label !== 'string') {
        throw httpError('需要 index（非负整数）与 label', 400)
      }
      return mvuInteract(body.sessionId, body.index, body.label)
    }),
    {
      path: `${API_BASE}/vendor/purify.js`,
      methods: ['GET'],
      requestBody: 'buffered',
      fetch: async () => {
        purifySource ??= readFileSync(PURIFY_FILE, 'utf8')
        return new Response(purifySource, {
          headers: { 'content-type': 'text/javascript; charset=utf-8', 'cache-control': 'private, max-age=3600' },
        })
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
