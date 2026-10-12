// 宿主半边冒烟：用假的 DSH ctx 加载插件，拉起真实 sidecar，跑「新建故事 → 开场 →
// 一轮写作 → 后处理后自动接受」。底层模型是一个只回固定文字的假模型（不发任何外部
// 请求），覆盖协议两端的消息转换、块转发、思考块进度与正文输出。
//
// 需要先构建 sidecar：cargo build -p storyforge-dsh-bridge（找不到二进制时跳过）。
// 运行：node --test integrations/dsh-storyforge/test/*.test.mjs

import assert from 'node:assert/strict'
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync, mkdirSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'

import { apply } from '../lib/index.js'
import { notifyFork } from '../lib/shared.js'

const here = dirname(fileURLToPath(import.meta.url))
const bin = resolve(
  here,
  '../../../target/debug',
  process.platform === 'win32' ? 'storyforge-dsh-bridge.exe' : 'storyforge-dsh-bridge',
)
const OPENING = '林间的光斑落在木屋门前，Seraphina 抬起头望向你。'
const PROSE = '木门吱呀一声推开，炉火映着 Seraphina 的侧脸。她示意你坐下，把一碗热汤推到你面前。'

function fakeModel(calls) {
  return async function* stream(options) {
    calls.push(options)
    assert.equal(options.provider, 'fake')
    assert.equal(options.model, 'writer')
    for (const message of options.messages) {
      if (message.role === 'assistant') assert.equal(message.source?.kind, 'model')
      if (message.role === 'tool') assert.ok(message.toolCallId)
    }
    yield { type: 'block-start', index: 0, blockType: 'text' }
    for (const piece of [PROSE.slice(0, 10), PROSE.slice(10)]) {
      yield { type: 'text-delta', index: 0, text: piece }
    }
    yield { type: 'block-end', index: 0, block: { type: 'text', text: PROSE } }
    yield { type: 'usage', usage: { inputTokens: 10, outputTokens: 20 } }
    yield { type: 'finish', reason: { kind: 'stop' } }
  }
}

function fakeCtx(llmCalls) {
  const adapters = new Map()
  const routes = new Map()
  const disposers = []
  const ctx = {
    effect(fn) {
      const dispose = fn()
      if (typeof dispose === 'function') disposers.push(dispose)
    },
    llm: {
      registerAdapter(providers, adapter) {
        for (const provider of providers) adapters.set(provider, adapter)
      },
      stream: fakeModel(llmCalls),
      listProviders: () => [{ id: 'fake', name: 'Fake' }, { id: 'storyforge', name: 'StoryForge' }],
      listModels: async () => [{ id: 'writer', name: 'Writer' }],
    },
    connection: {
      fetch: {
        register(route) {
          routes.set(route.path, route)
          return () => routes.delete(route.path)
        },
      },
    },
    agentPresets: {
      resolve: async (id) => ({ id }),
      acquireScope: async () => ({}),
      mount: async () => {},
    },
    workspaceRegistry: {
      create: async (path) => ({ path, attachSession: async () => {}, detachSession: async () => {} }),
    },
    agents: {
      create: async () => ({ agent: { session: {}, followup: () => {} }, dispose: async () => {} }),
    },
    sessionTitle: { rename: () => {} },
  }
  return { ctx, adapters, routes, dispose: () => Promise.all(disposers.map((d) => d())) }
}

async function call(routes, path, init) {
  const route = routes.get(`/api/storyforge${path}`)
  assert.ok(route, `route ${path}`)
  const response = await route.fetch(new Request(`http://localhost/api/storyforge${path}`, init))
  return response.json()
}

async function collect(stream) {
  const chunks = []
  for await (const chunk of stream) chunks.push(chunk)
  return chunks
}

const userMessage = (text, kind) => ({
  id: crypto.randomUUID(),
  role: 'user',
  content: [{ type: 'text', text }],
  source: { kind },
})

test('host half drives the real sidecar with models served through ctx.llm', { skip: !existsSync(bin) && `sidecar not built: ${bin}`, timeout: 120_000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'sf-dsh-host-'))
  const cardsDir = join(root, 'cards')
  mkdirSync(cardsDir)
  writeFileSync(
    join(cardsDir, 'seraphina.json'),
    JSON.stringify({
      spec: 'chara_card_v2',
      spec_version: '2.0',
      data: {
        name: 'Seraphina',
        description: '守护森林的女法师。',
        personality: '温柔',
        scenario: '旅人在森林里迷路。',
        first_mes: OPENING,
        mes_example: '',
        alternate_greetings: [],
        tags: [],
        creator: 'host-smoke',
        character_version: '1',
        extensions: {},
      },
    }),
  )
  const llmCalls = []
  const { ctx, adapters, routes, dispose } = fakeCtx(llmCalls)
  try {
    apply(ctx, { dataDir: join(root, 'data'), cardsDir })
    const adapter = adapters.get('storyforge')
    assert.ok(adapter)

    // 没选底层模型时，模型调用以 no_model 失败（识别降级、开档仍可完成）。
    const models = await call(routes, '/models')
    assert.deepEqual(models.providers.map((p) => p.provider), ['fake'])
    const saved = await call(routes, '/settings', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ provider: 'fake', model: 'writer' }),
    })
    assert.equal(saved.ok, true)
    const modes = await call(routes, '/settings', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ generationMode: 'auto', toolMode: 'native' }),
    })
    assert.equal(modes.ok, true)
    assert.deepEqual(modes.settings.model, { provider: 'fake', model: 'writer' })
    const badMode = await call(routes, '/settings', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ toolMode: 'telepathy' }),
    })
    assert.equal(badMode.ok, false)

    const cards = await call(routes, '/cards')
    assert.deepEqual(cards.cards.map((card) => card.file), ['seraphina.json'])
    const created = await call(routes, '/sessions', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ card: 'seraphina.json' }),
    })
    assert.equal(created.ok, true, created.error)
    assert.equal(created.campaign.opening, OPENING)
    const { sessionId } = created

    // 路径穿越被拒。
    const traversal = await call(routes, '/sessions', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ card: '../data/sessions.json' }),
    })
    assert.equal(traversal.ok, false)

    const opening = userMessage('（开始新故事）', 'storyforge')
    const openingChunks = await collect(adapter.stream({ sessionId, messages: [opening] }))
    const openingText = openingChunks.filter((c) => c.type === 'text-delta').map((c) => c.text).join('')
    assert.equal(openingText, OPENING)

    const callsBeforeTurn = llmCalls.length
    const messages = [
      opening,
      { id: crypto.randomUUID(), role: 'assistant', content: [{ type: 'text', text: OPENING }], source: { kind: 'model', provider: 'storyforge', model: 'pipeline' } },
      userMessage('[model changed: x]', 'model-selection'),
      userMessage('我推开木屋的门。', 'user'),
    ]
    const chunks = await collect(adapter.stream({ sessionId, messages, signal: new AbortController().signal }))
    const blockTypes = chunks.filter((c) => c.type === 'block-start').map((c) => c.blockType)
    assert.deepEqual(blockTypes, ['reasoning', 'text'])
    const reasoning = chunks.filter((c) => c.type === 'reasoning-delta').map((c) => c.text).join('')
    assert.match(reasoning, /开始本轮写作/)
    assert.match(reasoning, /【本轮】.*次底层调用/)
    const text = chunks.filter((c) => c.type === 'text-delta').map((c) => c.text).join('')
    assert.ok(text.trim().length > 0)
    assert.equal(chunks.at(-1).type, 'finish')
    assert.ok(llmCalls.length > callsBeforeTurn, '本轮应经 ctx.llm 调用底层模型')
    // DSH 拿到的是底层调用的真实用量（假模型每次 10 入 / 20 出），而不是正文字数估算。
    const usage = chunks.find((c) => c.type === 'usage').usage
    assert.ok(usage.outputTokens >= 20 && usage.outputTokens % 20 === 0, JSON.stringify(usage))
    assert.equal(usage.inputTokens, (usage.outputTokens / 20) * 10)

    // 收尾：后处理落定后自动接受。
    let status
    for (let i = 0; i < 100; i++) {
      status = await call(routes, '/status')
      if (status.settles.length > 0) break
      await new Promise((r) => setTimeout(r, 200))
    }
    assert.equal(status.settles.length, 1, JSON.stringify(status))
    assert.match(status.settles[0].accept, /^accepted/)
    assert.equal(status.status.active_campaign, created.campaign.campaign_id)
    assert.equal(status.status.tool_mode, 'native')
    assert.equal(status.llmCallCount, llmCalls.length)
    assert.ok(status.llmCalls.every((entry) => entry.status === 'ok' && entry.model === 'fake/writer'))
    assert.ok(status.settles[0].usage.calls >= 1, '后处理用量应计入收尾报告')
    // 每次底层调用都按真实系统提示词标出管线角色（首页「底层模型调用」表的「角色」列）。
    const roles = new Set(status.llmCalls.map((entry) => entry.role))
    assert.ok(!roles.has('其它') && roles.has('角色识别') && roles.has('执笔') && roles.has('后处理'), [...roles].join('、'))
    // 首页「我的故事」从 /status 一次拿到故事登记。
    assert.equal(status.sessions[sessionId].campaignId, created.campaign.campaign_id)
    // 收尾后故事登记记下轮数与最近一次结果，收尾报告落盘（跨 DSH 重启保留）。
    assert.equal(status.sessions[sessionId].turns, 1)
    assert.match(status.sessions[sessionId].lastSettle.accept, /^accepted/)
    assert.equal(JSON.parse(readFileSync(join(root, 'data', 'settles.json'), 'utf8')).reports.length, 1)

    // DSH 重新生成上一轮（历史里少一轮）：存档已采纳的轮次不能重写，明确拒绝。
    await assert.rejects(collect(adapter.stream({ sessionId, messages })), /不能重新生成/)

    // 从最后一条回复分叉：子会话第一次写作时映射到存档分支，两边各自继续。
    const history = [
      ...messages,
      { id: crypto.randomUUID(), role: 'assistant', content: [{ type: 'text', text }], source: { kind: 'model', provider: 'storyforge', model: 'pipeline' } },
    ]
    notifyFork('session-child', sessionId)
    const childChunks = await collect(
      adapter.stream({ sessionId: 'session-child', messages: [...history, userMessage('我和她一起出发。', 'user')] }),
    )
    assert.equal(childChunks.at(-1).type, 'finish')
    const mapped = (await call(routes, '/sessions')).sessions['session-child']
    assert.ok(mapped.campaignId && mapped.campaignId !== created.campaign.campaign_id, JSON.stringify(mapped))
    assert.equal(mapped.forkTurns, 1)
    const parentChunks = await collect(
      adapter.stream({ sessionId, messages: [...history, userMessage('我留在木屋里。', 'user')] }),
    )
    assert.equal(parentChunks.at(-1).type, 'finish')

    // 从开场处分叉（不是最新一轮）：拒绝并说明。
    notifyFork('session-early', sessionId)
    await assert.rejects(
      collect(adapter.stream({ sessionId: 'session-early', messages: [opening, messages[1], userMessage('换个走向。', 'user')] })),
      /只能从最新一轮分叉/,
    )

    // 未登记会话：明确报错，而不是悄悄跑到别的存档。
    await assert.rejects(
      collect(adapter.stream({ sessionId: 'session-unknown', messages: [userMessage('hi', 'user')] })),
      /没有关联 StoryForge 存档/,
    )
  } finally {
    await dispose()
    rmSync(root, { recursive: true, force: true })
  }
})
