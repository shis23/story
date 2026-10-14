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
import { cacheName, openingContentHash } from '../lib/shell.js'
import { pngCard } from './support/png.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const bin = resolve(
  here,
  '../../../target/debug',
  process.platform === 'win32' ? 'storyforge-dsh-bridge.exe' : 'storyforge-dsh-bridge',
)
const OPENING = '林间的光斑落在木屋门前，Seraphina 抬起头望向你。'
const PROSE = '木门吱呀一声推开，炉火映着 Seraphina 的侧脸。她示意你坐下，把一碗热汤推到你面前。'
/** 开场白末尾的状态栏：DSH 正文里拿掉，交给 Turn 尾部渲染。 */
const STATUS_BAR = '<details><summary>状态</summary><ul><li>体力：80</li></ul></details>'
const FIRST_MES = `${OPENING}\n${STATUS_BAR}`
const JSON_HEADERS = { 'content-type': 'application/json' }

function fakeModel(calls, models = ['writer']) {
  return async function* stream(options) {
    calls.push(options)
    assert.equal(options.provider, 'fake')
    assert.ok(models.includes(options.model), `unexpected model ${options.model}`)
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
  const titles = []
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
    sessionTitle: { rename: (_session, title) => titles.push(title) },
  }
  return { ctx, adapters, routes, titles, dispose: () => Promise.all(disposers.map((d) => d())) }
}

async function call(routes, path, init) {
  // DSH 按 pathname 匹配受保护路由（查询串不参与）。
  const route = routes.get(`/api/storyforge${path.split('?')[0]}`)
  assert.ok(route, `route ${path}`)
  const response = await route.fetch(new Request(`http://localhost/api/storyforge${path}`, init))
  return response.json()
}

/**
 * 收尾：卸载插件、等 sidecar 退出，再删临时目录。插件卸载本身不等 sidecar，而 Windows
 * 上 SQLite 库文件被占用时删不掉（EPERM）。
 */
async function teardown(routes, dispose, root) {
  const pid = await call(routes, '/status', { method: 'GET' })
    .then((status) => status.bridge?.pid)
    .catch(() => undefined)
  await dispose()
  await waitForExit(pid)
  // 进程刚退出时杀毒 / 索引服务可能还短暂占着新建的库文件：EPERM / EBUSY 稍等重试。
  for (let attempt = 1; ; attempt++) {
    try {
      rmSync(root, { recursive: true, force: true })
      return
    } catch (error) {
      if (attempt >= 30 || !['EPERM', 'EBUSY'].includes(error?.code)) throw error
      await new Promise((done) => setTimeout(done, 100))
    }
  }
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
        first_mes: FIRST_MES,
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
  const { ctx, adapters, routes, titles, dispose } = fakeCtx(llmCalls)
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
    assert.deepEqual(cards.cards.map((card) => [card.file, card.name]), [['seraphina.json', 'Seraphina']])

    // 首页导入角色卡：PNG 原始字节 + 原文件名；读不出卡名的拒收，同名自动编号、不覆盖。
    const importCard = (name, body) =>
      call(routes, `/cards/import?name=${encodeURIComponent(name)}`, {
        method: 'POST',
        headers: { 'content-type': 'application/octet-stream' },
        body,
      })
    const png = pngCard({ chara: { spec: 'chara_card_v2', data: { name: '林间小屋' } } })
    const imported = await importCard(String.raw`C:\Downloads\林间.png`, png)
    assert.deepEqual(imported.card, { file: '林间.png', name: '林间小屋', size: png.length })
    assert.equal((await importCard('林间.png', png)).card.file, '林间 (2).png')
    assert.equal((await importCard('notes.txt', png)).ok, false)
    const notACard = await importCard('plain.png', Buffer.from('not a card'))
    assert.equal(notACard.ok, false)
    assert.match(notACard.error, /没有在文件里找到角色卡数据/)
    const listed = await call(routes, '/cards')
    assert.deepEqual(listed.cards.map((card) => card.name).sort(), ['Seraphina', '林间小屋', '林间小屋'])
    const created = await call(routes, '/sessions', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ card: 'seraphina.json' }),
    })
    assert.equal(created.ok, true, created.error)
    assert.equal(created.campaign.opening, FIRST_MES)
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
    assert.equal(openingText, OPENING, '状态栏不进 DSH 正文')
    // 回放数据：开场白在存档里的位置 + 交给 Turn 尾部的块数（DSH 原样存进这条回复）。
    const openingRef = openingChunks.at(-1).replayState?.response?.storyforge
    assert.deepEqual(openingRef, {
      v: 1,
      campaignId: created.campaign.campaign_id,
      conversationId: created.campaign.conversation_id,
      nodeId: created.campaign.opening_node_id,
      nodeIndex: 0,
      blocks: 1,
      contentHash: openingContentHash(FIRST_MES),
    })

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
    const turnRef = chunks.at(-1).replayState?.response?.storyforge
    assert.equal(turnRef.conversationId, created.campaign.conversation_id)
    assert.equal(turnRef.nodeIndex, 2, '开场 0、用户 1、回复 2')
    assert.equal(turnRef.blocks, 0)
    assert.ok(turnRef.nodeId)
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

    // Turn 尾部按回放数据合批取展示数据：开场白有一个状态栏块，本轮没有；格式不对的忽略。
    const shown = await call(routes, '/display', {
      method: 'POST',
      headers: JSON_HEADERS,
      body: JSON.stringify({
        sessionId,
        refs: [{ key: 'opening', ...openingRef }, { key: 'turn', ...turnRef }, { key: 'bad', conversationId: 7 }],
      }),
    })
    assert.equal(shown.ok, true, shown.error)
    assert.deepEqual(shown.items, {
      opening: { parts: [{ type: 'html', blocks: [{ html: STATUS_BAR, text: '状态\n\n- 体力：80' }] }] },
      turn: null,
    })
    const notArray = await call(routes, '/display', { method: 'POST', headers: JSON_HEADERS, body: '{"refs":1}' })
    assert.equal(notArray.ok, false)
    const purify = await routes
      .get('/api/storyforge/vendor/purify.js')
      .fetch(new Request('http://localhost/api/storyforge/vendor/purify.js'))
    assert.match(purify.headers.get('content-type'), /javascript/)
    assert.match(await purify.text(), /export \{ purify as default \}/)

    // DSH 重新生成上一轮（历史里少一轮）：存档已采纳的轮次不能重写，明确拒绝。
    await assert.rejects(collect(adapter.stream({ sessionId, messages })), /不能重新生成/)

    // 从最后一条回复分叉：子会话第一次写作时映射到存档分支，两边各自继续。
    const history = [
      ...messages,
      { id: crypto.randomUUID(), role: 'assistant', content: [{ type: 'text', text }], source: { kind: 'model', provider: 'storyforge', model: 'pipeline' } },
    ]
    notifyFork('session-child', sessionId)
    // 子会话第一次发送前还没有存档：卡内状态面板给出说明，不报错。
    const unlinked = await call(routes, '/mvu', {
      method: 'POST',
      headers: JSON_HEADERS,
      body: JSON.stringify({ sessionId: 'session-child' }),
    })
    assert.deepEqual(unlinked.model, { state: 'none', reason: 'unlinked' })
    const childChunks = await collect(
      adapter.stream({ sessionId: 'session-child', messages: [...history, userMessage('我和她一起出发。', 'user')] }),
    )
    assert.equal(childChunks.at(-1).type, 'finish')
    const mapped = (await call(routes, '/sessions')).sessions['session-child']
    assert.ok(mapped.campaignId && mapped.campaignId !== created.campaign.campaign_id, JSON.stringify(mapped))
    assert.equal(mapped.forkTurns, 1)
    // 分叉后继承的开场白：sidecar 按位置到分支存档的对话里取（来源对话不在也一样）。
    const viaFork = await call(routes, '/display', {
      method: 'POST',
      headers: JSON_HEADERS,
      body: JSON.stringify({ sessionId: 'session-child', refs: [{ key: 'opening', conversationId: 'gone', nodeIndex: 0 }] }),
    })
    assert.equal(viaFork.items.opening?.parts[0].blocks[0].html, STATUS_BAR, JSON.stringify(viaFork))
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

    // 「测试连接」：用选定的底层模型发一条短请求。
    const probe = await call(routes, '/models/test', { method: 'POST' })
    assert.equal(probe.passed, true, JSON.stringify(probe))
    assert.equal(probe.model, 'fake/writer')

    // 底层模型没配密钥：测试连接、开档与写作都给出可操作的提示，而不是原始错误。
    ctx.llm.stream = async function* () {
      yield {
        type: 'finish',
        reason: { kind: 'error', failure: { code: 'MISSING_CREDENTIAL', message: 'llm-fake: no API key for provider route "fake"' } },
      }
    }
    const failedProbe = await call(routes, '/models/test', { method: 'POST' })
    assert.equal(failedProbe.passed, false)
    assert.match(failedProbe.error, /还没有配置 API Key/)
    assert.match(failedProbe.detail, /MISSING_CREDENTIAL/)
    const longTitle = '我的假小子黑皮青梅不可能变成清冷校花，更别提她还有个美艳继母'
    const unconfigured = await call(routes, '/sessions', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ card: '林间.png', title: longTitle }),
    })
    assert.equal(unconfigured.ok, true, unconfigured.error)
    assert.equal(unconfigured.campaign.extraction_status, 'fallback')
    assert.match(unconfigured.warning, /角色识别失败.*还没有配置 API Key/)
    const story = (await call(routes, '/sessions')).sessions[unconfigured.sessionId]
    assert.equal(story.title, longTitle, '插件登记里保留完整标题')
    assert.match(story.extractionErrorDetail, /MISSING_CREDENTIAL/)
    // 给 DSH 的标题预先截断，分叉时 DSH 追加的 " (1)" 不会被 80 字节上限截成半个括号。
    assert.ok(titles.at(-1).endsWith('…') && Buffer.byteLength(`${titles.at(-1)} (1)`) <= 80, titles.at(-1))
    await assert.rejects(
      collect(adapter.stream({ sessionId: unconfigured.sessionId, messages: [opening, userMessage('你好。', 'user')] })),
      /底层模型还没有配置 API Key[\s\S]*原始错误/,
    )
  } finally {
    await teardown(routes, dispose, root)
  }
})

/** 假分析结果：一个血条、一个交互按钮（加血 + 下一轮提示）、一个要原卡脚本的兜底片段。 */
const MVU_TRANSLATION = {
  variable_schema: [
    { key: 'hp', label: '生命值', value_type: 'int', default: 100 },
    { key: '金币', label: '金币', value_type: 'int', default: 5 },
  ],
  ui_bindings: [{ element: 'hp_bar', variable_key: 'hp', display: { kind: 'bar', max: 100 } }],
  update_rules: ['受伤时 hp 减少伤害值'],
  interactions: [
    {
      element_label: '休息',
      actions: [
        { kind: 'modify_variable', key: 'hp', value_expr: '+10' },
        { kind: 'trigger_next_turn', hint: '我在炉火边休息片刻' },
      ],
    },
  ],
  fallback_fragments: [{ description: '开场动画', js_snippet: 'playIntro()', reason: '需要 DOM' }],
  routing: { kind: 'native' },
  analysis_confidence: 0.9,
  notes: [],
}

/** MVU 分析器走终止工具 `emit_mvu_translation`；其它调用照常回固定正文。 */
function mvuModel(calls) {
  const prose = fakeModel(calls)
  return async function* stream(options) {
    if (!options.system?.includes('卡内状态栏分析')) {
      yield* prose(options)
      return
    }
    calls.push(options)
    assert.ok(options.tools?.some((tool) => tool.name === 'emit_mvu_translation'))
    const block = { type: 'tool-call', id: 'call-mvu', name: 'emit_mvu_translation', arguments: JSON.stringify(MVU_TRANSLATION) }
    yield { type: 'block-start', index: 0, blockType: 'tool-call' }
    yield { type: 'block-end', index: 0, block }
    yield { type: 'usage', usage: { inputTokens: 30, outputTokens: 40 } }
    yield { type: 'finish', reason: { kind: 'tool-calls' } }
  }
}

test('mvu panel: analyze, apply and interaction buttons go through the sidecar', { skip: !existsSync(bin) && `sidecar not built: ${bin}`, timeout: 120_000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'sf-dsh-mvu-'))
  const cardsDir = join(root, 'cards')
  mkdirSync(cardsDir)
  writeFileSync(
    join(cardsDir, 'mvu.json'),
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
        extensions: { stat_data: { hp: 80 } },
      },
    }),
  )
  const llmCalls = []
  const { ctx, routes, dispose } = fakeCtx(llmCalls)
  ctx.llm.stream = mvuModel(llmCalls)
  const post = (path, body) => call(routes, path, { method: 'POST', headers: JSON_HEADERS, body: JSON.stringify(body) })
  try {
    apply(ctx, { dataDir: join(root, 'data'), cardsDir })
    assert.equal((await post('/settings', { provider: 'fake', model: 'writer', toolMode: 'native' })).ok, true)
    const created = await post('/sessions', { card: 'mvu.json' })
    assert.equal(created.ok, true, created.error)
    const { sessionId } = created

    // 不是故事会话：404；还没分析：提示手动分析（D44 不自动跑）。
    const unknown = await post('/mvu', { sessionId: 'nope' })
    assert.equal(unknown.ok, false)
    const before = await post('/mvu', { sessionId })
    assert.equal(before.ok, true, before.error)
    assert.equal(before.model.state, 'unanalyzed')
    assert.equal(before.model.cardName, 'Seraphina')

    const callsBefore = llmCalls.length
    const analyzed = await post('/mvu/analyze', { sessionId })
    assert.equal(analyzed.ok, true, analyzed.error)
    assert.equal(llmCalls.length, callsBefore + 1, '分析只调用一次底层模型')
    assert.equal(analyzed.empty, false)
    assert.equal(analyzed.error, null)
    assert.ok(analyzed.usage.calls >= 1, JSON.stringify(analyzed.usage))
    const ready = analyzed.model
    assert.equal(ready.state, 'ready')
    assert.deepEqual(ready.interactions, [{ index: 0, label: '休息' }])
    assert.deepEqual(ready.fallbacks, [{ description: '开场动画', reason: '需要 DOM' }])
    assert.ok(ready.pendingApply.some((pending) => pending.added.includes('金币')), JSON.stringify(ready.pendingApply))
    const status = await call(routes, '/status')
    assert.ok(status.llmCalls.some((entry) => entry.role === 'MVU 分析'), '调用表认得出分析器')

    const applied = await post('/mvu/apply', { sessionId })
    assert.equal(applied.ok, true, applied.error)
    assert.deepEqual(applied.model.pendingApply, [])
    const hpRow = () => applied.model.sections[0]?.rows.find((row) => row.element === 'hp_bar')
    assert.equal(hpRow()?.kind, 'bar', JSON.stringify(applied.model))
    const hpBefore = hpRow().value

    // 交互：参数不对 400；按钮文字与当前翻译对不上 409（面板过期）；对上了按最新变量写入。
    const bad = await post('/mvu/interact', { sessionId, index: -1, label: '休息' })
    assert.equal(bad.ok, false)
    assert.match(bad.error, /index/)
    const stale = await post('/mvu/interact', { sessionId, index: 0, label: '攻击' })
    assert.equal(stale.ok, false)
    assert.match(stale.error, /刷新面板/)
    const rested = await post('/mvu/interact', { sessionId, index: 0, label: '休息' })
    assert.equal(rested.ok, true, rested.error)
    assert.equal(rested.failed, 0, JSON.stringify(rested.results))
    assert.deepEqual(rested.results.map((result) => [result.ok, result.scope, result.key]), [[true, 'instance', 'hp']])
    assert.deepEqual(rested.hints, ['我在炉火边休息片刻'])
    const hpAfter = rested.model.sections[0].rows.find((row) => row.element === 'hp_bar').value
    assert.equal(hpAfter, hpBefore + 10)
  } finally {
    await teardown(routes, dispose, root)
  }
})

const SHELL_STATUS_URL = 'https://testingcf.jsdelivr.net/gh/storyforge-test/shell@1.0.0/dist/status/index.html'
/** 卡清单里没有的远程界面（开场白直接写的 `.load`，不经卡正则）。 */
const SHELL_UNLISTED_URL = 'https://files.catbox.moe/unlisted.html'
const SHELL_INLINE_HTML =
  "<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head><body><div id=\"sf-inline\">加载中</div><script>document.getElementById('sf-inline').textContent = '内联界面已运行';</script></body></html>"
const SHELL_STATUS_PAGE =
  "<!doctype html><html><head><title>status page</title></head><body><div id=\"s\"></div><script type=\"module\">import mount from './mount.js'; mount();</script></body></html>"

function shellRegex(id, name, find, replace) {
  return { id, scriptName: name, findRegex: find, replaceString: replace, placement: [2], disabled: false, markdownOnly: true, promptOnly: false, runOnEdit: true }
}

/** 预置宿主代取的磁盘缓存：测试不发外部请求（首取即冻结，与桌面相同）。 */
function seedShellCache(dataDir, url, body) {
  const dir = join(dataDir, 'card-shell-cache')
  mkdirSync(dir, { recursive: true })
  writeFileSync(join(dir, cacheName(url)), body)
  writeFileSync(join(dir, `${cacheName(url)}.meta.json`), JSON.stringify({ url, content_type: 'text/html' }))
}

test('card shells: tail parts, server-built documents, approval and the host fetch', { skip: !existsSync(bin) && `sidecar not built: ${bin}`, timeout: 120_000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'sf-dsh-shell-'))
  const cardsDir = join(root, 'cards')
  const dataDir = join(root, 'data')
  mkdirSync(cardsDir)
  const loadGlue = `\`\`\`html\n<body><script>$('body').load('${SHELL_STATUS_URL}')</script></body>\n\`\`\``
  writeFileSync(
    join(cardsDir, 'shell.json'),
    JSON.stringify({
      spec: 'chara_card_v2',
      spec_version: '2.0',
      data: {
        name: 'Seraphina',
        description: '守护森林的女法师。',
        personality: '温柔',
        scenario: '旅人在森林里迷路。',
        first_mes: `木屋里很暖和。\n<StatusPlaceHolderImpl/>\n<sf-inline/>\n$('body').load('${SHELL_UNLISTED_URL}')`,
        mes_example: '',
        alternate_greetings: [],
        tags: [],
        creator: 'host-smoke',
        character_version: '1',
        extensions: {
          regex_scripts: [
            shellRegex('status-shell', '状态栏界面', '<StatusPlaceHolderImpl/>', loadGlue),
            shellRegex('inline-shell', '内联界面', '<sf-inline/>', SHELL_INLINE_HTML),
          ],
        },
      },
    }),
  )
  seedShellCache(dataDir, SHELL_STATUS_URL, SHELL_STATUS_PAGE)
  seedShellCache(dataDir, SHELL_UNLISTED_URL, '<p id="unlisted">别处的页面</p>')
  const llmCalls = []
  const { ctx, adapters, routes, dispose } = fakeCtx(llmCalls)
  const post = (path, body) => call(routes, path, { method: 'POST', headers: JSON_HEADERS, body: JSON.stringify(body) })
  const getDoc = (token) =>
    routes.get('/api/storyforge/shell/doc').fetch(new Request(`http://localhost/api/storyforge/shell/doc?t=${encodeURIComponent(token)}`))
  try {
    apply(ctx, { dataDir, cardsDir })
    assert.equal((await post('/settings', { provider: 'fake', model: 'writer', toolMode: 'native' })).ok, true)
    const created = await post('/sessions', { card: 'shell.json' })
    assert.equal(created.ok, true, created.error)
    const { sessionId } = created

    // 开场白：界面不进 DSH 正文，回放数据记下界面数。
    const openingChunks = await collect(adapters.get('storyforge').stream({ sessionId, messages: [userMessage('（开始新故事）', 'storyforge')] }))
    assert.equal(openingChunks.filter((c) => c.type === 'text-delta').map((c) => c.text).join(''), '木屋里很暖和。')
    const ref = openingChunks.at(-1).replayState?.response?.storyforge
    assert.equal(ref.shells, 3, JSON.stringify(ref))
    assert.equal(ref.blocks, 0)

    // 尾部：卡清单里的 `.load` 与命中卡正则的内联文档自动挂，开场白里直接写的 URL 要确认。
    const shown = await post('/display', { sessionId, refs: [{ key: 'opening', ...ref }] })
    assert.equal(shown.ok, true, shown.error)
    const parts = shown.items.opening.parts
    assert.deepEqual(
      parts.map((part) => [part.type, part.mode, part.kind, part.url ?? 'inline', part.trusted]),
      [
        ['shell', 'load', 'status', SHELL_STATUS_URL, true],
        ['shell', 'inline', 'message_html', 'inline', true],
        ['shell', 'load', 'message_html', SHELL_UNLISTED_URL, false],
      ],
    )
    assert.equal(parts[0].label, '状态栏界面')
    assert.equal('html' in parts[1], false, '展示数据不带界面正文')

    // 挂清单里的状态栏：服务端取页面（这里命中预置缓存）、拼壳文档、登记一次性路径。
    const opened = await post('/shell/open', { sessionId, ref, shell: { mode: 'load', url: SHELL_STATUS_URL } })
    assert.equal(opened.ok, true, opened.error)
    assert.equal(opened.kind, 'status')
    assert.equal(opened.trusted, true)
    assert.match(opened.bridgeSession, /^sfshell:/)
    assert.match(opened.pluginId, /^card-shell-/)
    const doc = await getDoc(opened.token)
    assert.equal(doc.status, 200)
    assert.match(doc.headers.get('content-security-policy'), /^sandbox allow-scripts; default-src 'none';.*https:\/\/testingcf\.jsdelivr\.net/)
    assert.equal(doc.headers.get('cache-control'), 'no-store')
    const docHtml = await doc.text()
    assert.match(docHtml, /<title>status page<\/title>/)
    assert.ok(docHtml.includes(JSON.stringify(opened.bridgeSession)), '桥接会话写进文档')
    assert.match(docHtml, /import mount from 'https:\/\/testingcf\.jsdelivr\.net\/gh\/storyforge-test\/shell@1\.0\.0\/dist\/status\/mount\.js'/)
    assert.match(docHtml, /data-sf-module="m1"/)
    // 同一个 token 可以重复取（卡调 location.reload()）；不认识的 token 404。
    assert.equal((await getDoc(opened.token)).status, 200)
    assert.equal((await getDoc('nope')).status, 404)

    // 内联文档按起点定位，正文由服务端从存档重新取。
    const inline = await post('/shell/open', { sessionId, ref, shell: { mode: 'inline', start: parts[1].start } })
    assert.equal(inline.ok, true, inline.error)
    assert.match(await (await getDoc(inline.token)).text(), /内联界面已运行/)

    // 不在清单里的：没有用户确认 403，确认后照常挂；存档里没有的界面 409（浏览器不能指定任意 URL）。
    const unapproved = await post('/shell/open', { sessionId, ref, shell: { mode: 'load', url: SHELL_UNLISTED_URL } })
    assert.equal(unapproved.ok, false)
    assert.match(unapproved.error, /需要先确认/)
    const approved = await post('/shell/open', { sessionId, ref, shell: { mode: 'load', url: SHELL_UNLISTED_URL }, approved: true })
    assert.equal(approved.ok, true, approved.error)
    assert.equal(approved.trusted, false)
    assert.match(await (await getDoc(approved.token)).text(), /别处的页面/)
    const forged = await post('/shell/open', { sessionId, ref, shell: { mode: 'load', url: 'https://files.catbox.moe/other.html' }, approved: true })
    assert.equal(forged.ok, false)
    assert.match(forged.error, /没有这个卡片界面/)
    assert.equal((await post('/shell/open', { sessionId, ref, shell: { mode: 'eval' } })).ok, false)
    assert.equal((await post('/shell/open', { sessionId: 'nope', ref, shell: { mode: 'load', url: SHELL_STATUS_URL } })).ok, false)

    // 宿主代取：白名单内的走缓存；白名单外的拒绝。
    const fetched = await post('/shell/fetch', { sessionId, url: SHELL_STATUS_URL })
    assert.equal(fetched.ok, true, fetched.error)
    assert.equal(fetched.resource.from_cache, true)
    assert.equal(fetched.resource.body_text, SHELL_STATUS_PAGE)
    const outside = await post('/shell/fetch', { sessionId, url: 'https://evil.example/x.js' })
    assert.equal(outside.ok, false)
    assert.match(outside.error, /allowlist/)

    // 壳自己的界面状态：写回存档，下一次挂载时注入文档（重启后的情况见本测试末尾）。
    const saved = await post('/shell/variables', { sessionId, selector: { type: 'chat' }, variables: { theme: 'night-sky' }, mode: 'replace' })
    assert.deepEqual(saved, { ok: true, persisted: true })
    assert.equal((await post('/shell/variables', { sessionId, selector: { type: 'chat' }, variables: [], mode: 'replace' })).ok, false)
    assert.equal((await post('/shell/variables', { sessionId, selector: { type: 'chat' }, variables: {}, mode: 'eval' })).ok, false)
    const reopened = await post('/shell/open', { sessionId, ref, shell: { mode: 'load', url: SHELL_STATUS_URL } })
    assert.match(await (await getDoc(reopened.token)).text(), /night-sky/)

    // Mvu 只读底座与用户确认后的变量写入。
    const mvuData = await post('/shell/mvu-data', { sessionId })
    assert.equal(mvuData.ok, true, mvuData.error)
    assert.equal(typeof mvuData.data, 'object')
    const written = await post('/shell/var-write', { sessionId, key: '好感', value: 42 })
    assert.equal(written.ok, true, written.error)
    assert.deepEqual(written.result, { ok: true, scope: 'campaign', key: '好感', error: null })
    const reserved = await post('/shell/var-write', { sessionId, key: '__storyforge_card_shell_variables', value: {} })
    assert.equal(reserved.result.ok, false)
    assert.ok(Object.hasOwn((await post('/shell/mvu-data', { sessionId })).data, '好感'))

    // 浏览器半边用的桌面模块打包。
    const vendor = await routes.get('/api/storyforge/vendor/card-shell.js').fetch(new Request('http://localhost/api/storyforge/vendor/card-shell.js'))
    assert.match(vendor.headers.get('content-type'), /javascript/)
    assert.match(await vendor.text(), /createHostHandler/)

    // 卸载插件（先等界面状态落盘再停 sidecar）后重新启用：同一个界面挂起来仍带着它。
    const firstPid = (await call(routes, '/status', { method: 'GET' })).bridge.pid
    await dispose()
    await waitForExit(firstPid)
    const again = fakeCtx(llmCalls)
    apply(again.ctx, { dataDir, cardsDir })
    try {
      const reopenedAfterRestart = await call(again.routes, '/shell/open', {
        method: 'POST',
        headers: JSON_HEADERS,
        body: JSON.stringify({ sessionId, ref, shell: { mode: 'load', url: SHELL_STATUS_URL } }),
      })
      assert.equal(reopenedAfterRestart.ok, true, reopenedAfterRestart.error)
      const restartedDoc = await again.routes
        .get('/api/storyforge/shell/doc')
        .fetch(new Request(`http://localhost/api/storyforge/shell/doc?t=${encodeURIComponent(reopenedAfterRestart.token)}`))
      assert.match(await restartedDoc.text(), /night-sky/)
    } finally {
      const pid = (await call(again.routes, '/status', { method: 'GET' })).bridge.pid
      await again.dispose()
      await waitForExit(pid)
    }
  } finally {
    await teardown(routes, dispose, root)
  }
})

const SHELL_HOME_URL = 'https://testingcf.jsdelivr.net/gh/storyforge-test/shell@1.0.0/dist/home/index.html'
const SHELL_CUSTOM_URL = 'https://testingcf.jsdelivr.net/gh/storyforge-test/shell@1.0.0/dist/custom_start/index.html'
const HOME_GREETING = '【首页】'
const CUSTOM_GREETING = '<customized>自定义开局</customized>'
const RAIN_GREETING = '雨夜，{{char}}正在擦拭酒杯。'

function lore(id, comment, key, enabled) {
  return {
    id,
    keys: [key],
    secondary_keys: [],
    comment,
    content: `${comment} 的设定正文。`,
    constant: false,
    selective: true,
    insertion_order: 100,
    enabled,
    position: 'before_char',
    extensions: {},
  }
}

test('opening shells: mount before the first turn, switch the opening, toggle the worldbook, close afterwards', { skip: !existsSync(bin) && `sidecar not built: ${bin}`, timeout: 120_000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'sf-dsh-opening-'))
  const cardsDir = join(root, 'cards')
  const dataDir = join(root, 'data')
  mkdirSync(cardsDir)
  const glue = (url) => `<body><script>$('body').load('${url}')</script></body>`
  writeFileSync(
    join(cardsDir, 'opening.json'),
    JSON.stringify({
      spec: 'chara_card_v2',
      spec_version: '2.0',
      data: {
        name: 'Seraphina',
        description: '守护森林的女法师。',
        personality: '温柔',
        scenario: '旅人在森林里迷路。',
        first_mes: HOME_GREETING,
        mes_example: '',
        alternate_greetings: [CUSTOM_GREETING, RAIN_GREETING],
        tags: [],
        creator: 'host-smoke',
        character_version: '1',
        character_book: {
          name: '命定之书',
          entries: [lore(0, '命定系统-魔法', '魔法', true), lore(1, '命定系统-科技', '科技', false)],
        },
        extensions: {
          regex_scripts: [
            shellRegex('home-shell', '首页', HOME_GREETING, glue(SHELL_HOME_URL)),
            shellRegex('custom-shell', '自定义开局', '<customized>[\\s\\S]*?</customized>', glue(SHELL_CUSTOM_URL)),
          ],
        },
      },
    }),
  )
  seedShellCache(dataDir, SHELL_HOME_URL, '<!doctype html><html><head><title>home</title></head><body>首页</body></html>')
  seedShellCache(dataDir, SHELL_CUSTOM_URL, '<!doctype html><html><head><title>custom</title></head><body>自定义开局</body></html>')
  const llmCalls = []
  const { ctx, adapters, routes, dispose } = fakeCtx(llmCalls)
  const post = (path, body) => call(routes, path, { method: 'POST', headers: JSON_HEADERS, body: JSON.stringify(body) })
  const getDoc = async (token) =>
    (await routes.get('/api/storyforge/shell/doc').fetch(new Request(`http://localhost/api/storyforge/shell/doc?t=${encodeURIComponent(token)}`))).text()
  try {
    apply(ctx, { dataDir, cardsDir })
    assert.equal((await post('/settings', { provider: 'fake', model: 'writer', toolMode: 'native' })).ok, true)
    const created = await post('/sessions', { card: 'opening.json' })
    assert.equal(created.ok, true, created.error)
    const { sessionId } = created
    const campaignId = created.campaign.campaign_id
    const worldbook = `storyforge:campaign:${campaignId}`

    // 开场轮的回放数据记下开场白指纹；开局前首页界面标 open。
    const opening = userMessage('（开始新故事）', 'storyforge')
    const openingChunks = await collect(adapters.get('storyforge').stream({ sessionId, messages: [opening] }))
    const ref = openingChunks.at(-1).replayState?.response?.storyforge
    assert.equal(ref.contentHash, openingContentHash(HOME_GREETING), JSON.stringify(ref))
    const tail = async () => {
      const shown = await post('/display', { sessionId, refs: [{ key: 'opening', ...ref }] })
      assert.equal(shown.ok, true, shown.error)
      return shown.items.opening.parts
    }
    const before = await tail()
    assert.deepEqual(
      before.map((part) => [part.type, part.kind, part.trusted, part.opening]),
      [['shell', 'opening_home', true, 'open']],
    )

    // 挂首页：文档带开场种子（卡的问候语）与本局世界书名。
    const home = await post('/shell/open', { sessionId, ref, shell: { mode: 'load', url: SHELL_HOME_URL } })
    assert.equal(home.ok, true, home.error)
    assert.equal(home.opening, true)
    const homeDoc = await getDoc(home.token)
    assert.match(homeDoc, /<title>home<\/title>/)
    const seed = JSON.parse(homeDoc.match(/var OPENING_CHAT_SEED = (\{.*?\});\n/)[1])
    assert.equal(seed.swipes.length, 3)
    assert.equal(seed.swipes[0], HOME_GREETING)
    assert.ok(homeDoc.includes(worldbook))

    // 世界书：只认本局书名；读出条目名与启用状态，按名切换启用。
    const book = await post('/shell/worldbook', { sessionId, name: worldbook })
    assert.equal(book.ok, true, book.error)
    const enabledOf = (entries, name) => entries.find((entry) => entry.name === name)?.enabled
    assert.equal(enabledOf(book.entries, '命定系统-魔法'), true)
    assert.equal(enabledOf(book.entries, '命定系统-科技'), false)
    assert.equal((await post('/shell/worldbook', { sessionId, name: 'storyforge:campaign:other' })).ok, false)
    const toggled = await post('/shell/worldbook-update', {
      sessionId,
      name: worldbook,
      entries: [
        { name: '命定系统-魔法', enabled: false },
        { name: '命定系统-科技', enabled: true },
        { name: '不存在的条目', enabled: true },
      ],
    })
    assert.equal(toggled.ok, true, toggled.error)
    assert.equal(toggled.updated, 2)
    const after = (await post('/shell/worldbook', { sessionId, name: worldbook })).entries
    assert.equal(enabledOf(after, '命定系统-魔法'), false)
    assert.equal(enabledOf(after, '命定系统-科技'), true)
    assert.equal((await post('/shell/worldbook-update', { sessionId, name: worldbook, entries: {} })).ok, false)

    // 选开场：只认本故事挂过的开场界面的桥会话。
    const forged = await post('/shell/opening', { sessionId, bridgeSession: 'sfshell:forged', swipe_id: 2, mes: RAIN_GREETING })
    assert.equal(forged.ok, false)
    assert.match(forged.error, /不是本故事的开场界面/)
    const rain = await post('/shell/opening', { sessionId, bridgeSession: home.bridgeSession, swipe_id: 2, mes: RAIN_GREETING })
    assert.equal(rain.ok, true, rain.error)
    assert.equal(rain.greetingIndex, 2)
    // 存档里的开场白变了：尾部补上新开场的正文（宏已展开），首页界面随原开场白一起不在了。
    const rainParts = await tail()
    assert.equal(rainParts.length, 1, JSON.stringify(rainParts))
    assert.equal(rainParts[0].type, 'text')
    assert.match(rainParts[0].text, /^雨夜，Seraphina正在擦拭酒杯。$/)

    // 还在开场态：同一个界面可以再选；只有界面的开场不补正文。
    const custom = await post('/shell/opening', { sessionId, bridgeSession: home.bridgeSession, swipe_id: 1, mes: CUSTOM_GREETING })
    assert.equal(custom.ok, true, custom.error)
    const customParts = await tail()
    assert.deepEqual(
      customParts.map((part) => [part.type, part.kind, part.opening]),
      [['shell', 'opening_custom', 'open']],
    )
    const customShell = await post('/shell/open', { sessionId, ref, shell: { mode: 'load', url: SHELL_CUSTOM_URL } })
    assert.equal(customShell.ok, true, customShell.error)
    assert.match(await getDoc(customShell.token), /<title>custom<\/title>/)

    // 写过一轮：开场界面关闭（尾部只留说明），挂载与改写开场白都拒绝。
    const messages = [
      opening,
      { id: crypto.randomUUID(), role: 'assistant', content: [{ type: 'text', text: '（开场界面）' }], source: { kind: 'model', provider: 'storyforge', model: 'pipeline' } },
      userMessage('我推开木屋的门。', 'user'),
    ]
    const chunks = await collect(adapters.get('storyforge').stream({ sessionId, messages, signal: new AbortController().signal }))
    assert.equal(chunks.at(-1).type, 'finish')
    let status
    for (let i = 0; i < 100; i++) {
      status = await call(routes, '/status')
      if (status.settles.length > 0) break
      await new Promise((r) => setTimeout(r, 200))
    }
    assert.equal(status.settles.length, 1, JSON.stringify(status))
    const closedParts = await tail()
    assert.deepEqual(
      closedParts.map((part) => [part.type, part.kind, part.opening]),
      [['shell', 'opening_custom', 'closed']],
    )
    const late = await post('/shell/open', { sessionId, ref, shell: { mode: 'load', url: SHELL_CUSTOM_URL } })
    assert.equal(late.ok, false)
    assert.match(late.error, /故事已经开始/)
    const lateApply = await post('/shell/opening', { sessionId, bridgeSession: home.bridgeSession, swipe_id: 2, mes: RAIN_GREETING })
    assert.equal(lateApply.ok, false)
  } finally {
    await teardown(routes, dispose, root)
  }
})

test('role models: each call goes to the model chosen for its group', { skip: !existsSync(bin) && `sidecar not built: ${bin}`, timeout: 120_000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'sf-dsh-roles-'))
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
  ctx.llm.stream = fakeModel(llmCalls, ['writer', 'strong', 'fast'])
  const post = (path, body) => call(routes, path, { method: 'POST', headers: JSON_HEADERS, body: JSON.stringify(body) })
  const strong = { provider: 'fake', model: 'strong' }
  const fast = { provider: 'fake', model: 'fast' }
  try {
    apply(ctx, { dataDir: join(root, 'data'), cardsDir })
    assert.equal((await post('/settings', { provider: 'fake', model: 'writer', toolMode: 'native' })).ok, true)

    // 未知分组、缺字段、指向 StoryForge 自己、整份不是对象：拒绝，且一条都不写（含同批合法的）。
    for (const roleModels of [
      { narrator: strong },
      { prose: { provider: 'fake' } },
      { prose: { provider: 'storyforge', model: 'pipeline' } },
      ['prose'],
      { prose: strong, bogus: null },
    ]) {
      assert.equal((await post('/settings', { roleModels })).ok, false, JSON.stringify(roleModels))
    }
    assert.deepEqual((await call(routes, '/models')).roleModels, {})

    assert.equal((await post('/settings', { roleModels: { prose: strong, settle: fast, director: fast } })).ok, true)
    // null = 这一组改回跟随底层模型，其它组不动。
    const cleared = await post('/settings', { roleModels: { director: null } })
    assert.deepEqual(cleared.settings.roleModels, { prose: strong, settle: fast })
    const models = await call(routes, '/models')
    assert.deepEqual(models.roleModels, { prose: strong, settle: fast })
    assert.deepEqual(Object.keys(models.roleGroups), ['director', 'actor', 'prose', 'settle', 'analysis'])

    // 测试连接按组：指定了的测指定模型，跟随底层模型的组与未知组报错。
    const probe = await post('/models/test', { group: 'prose' })
    assert.equal(probe.passed, true, JSON.stringify(probe))
    assert.equal(probe.model, 'fake/strong')
    assert.equal((await post('/models/test', { group: 'director' })).ok, false)
    assert.equal((await post('/models/test', { group: 'nope' })).ok, false)
    assert.equal((await post('/models/test', {})).model, 'fake/writer')

    const created = await post('/sessions', { card: 'seraphina.json' })
    assert.equal(created.ok, true, created.error)
    const { sessionId } = created
    const adapter = adapters.get('storyforge')
    const opening = userMessage('（开始新故事）', 'storyforge')
    await collect(adapter.stream({ sessionId, messages: [opening] }))
    const chunks = await collect(
      adapter.stream({
        sessionId,
        messages: [
          opening,
          { id: crypto.randomUUID(), role: 'assistant', content: [{ type: 'text', text: OPENING }], source: { kind: 'model', provider: 'storyforge', model: 'pipeline' } },
          userMessage('我推开木屋的门。', 'user'),
        ],
        signal: new AbortController().signal,
      }),
    )
    assert.equal(chunks.at(-1).type, 'finish')

    let status
    for (let i = 0; i < 100; i++) {
      status = await call(routes, '/status')
      if (status.settles.length > 0) break
      await new Promise((r) => setTimeout(r, 200))
    }
    assert.equal(status.settles.length, 1, JSON.stringify(status))
    // sidecar 的数据目录是全新的：直接用 SQLite。
    assert.equal(status.bridge.ready.storage, 'sqlite')
    // 调用表里每次调用的模型 = 本组指定的模型；没指定的组（角色识别）用底层模型。
    const expected = { 执笔: 'fake/strong', 编剧: 'fake/strong', 定稿: 'fake/strong', 摘要: 'fake/fast', 后处理: 'fake/fast', 纪要压缩: 'fake/fast' }
    const pipelineCalls = status.llmCalls.filter((entry) => entry.role !== '其它')
    assert.ok(pipelineCalls.length >= 3, JSON.stringify(status.llmCalls))
    for (const entry of pipelineCalls) {
      assert.equal(entry.status, 'ok', JSON.stringify(entry))
      assert.equal(entry.model, expected[entry.role] ?? 'fake/writer', `${entry.role} → ${entry.model}`)
    }
    const roles = new Set(pipelineCalls.map((entry) => entry.role))
    assert.ok(roles.has('执笔') && roles.has('后处理') && roles.has('角色识别'), [...roles].join('、'))
    // 实际发给 DSH 的请求与调用表一致。
    const sentModels = new Set(llmCalls.map((options) => options.model))
    assert.ok(sentModels.has('strong') && sentModels.has('fast') && sentModels.has('writer'), [...sentModels].join('、'))
  } finally {
    await teardown(routes, dispose, root)
  }
})

/** 角色识别走终止工具 `emit_characters` 认出四个角色；其它调用照常回固定正文。 */
const CREW = ['林医生', '陈警官', '苏沁', '王曼离']
function crewModel(calls) {
  const prose = fakeModel(calls)
  return async function* stream(options) {
    if (!options.system?.includes('卡内角色识别')) {
      yield* prose(options)
      return
    }
    calls.push(options)
    const characters = CREW.map((name) => ({
      name,
      persona_prompt: `你是${name}。`,
      behavior_rules: '',
      base_backstory: [],
      role_type: 'supporting',
      group: '旅伴',
    }))
    const block = { type: 'tool-call', id: 'call-crew', name: 'emit_characters', arguments: JSON.stringify({ characters }) }
    yield { type: 'block-start', index: 0, blockType: 'tool-call' }
    yield { type: 'block-end', index: 0, block }
    yield { type: 'usage', usage: { inputTokens: 30, outputTokens: 40 } }
    yield { type: 'finish', reason: { kind: 'tool-calls' } }
  }
}

test('auto route escalation: asks in the composer, then writes with the chosen mode', { skip: !existsSync(bin) && `sidecar not built: ${bin}`, timeout: 120_000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'sf-dsh-escalate-'))
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
  ctx.llm.stream = crewModel(llmCalls)
  // DSH 的 userQuestions：记下每次提问，按排好的回答作答。
  const asks = []
  const replies = []
  let questions = undefined
  const agent = { id: 'live-agent' }
  let sessionId
  ctx.get = (service) => (service === 'userQuestions' ? questions : undefined)
  ctx.agents.get = (id) => (id === sessionId ? agent : undefined)
  const post = (path, body) => call(routes, path, { method: 'POST', headers: JSON_HEADERS, body: JSON.stringify(body) })
  try {
    apply(ctx, { dataDir: join(root, 'data'), cardsDir })
    assert.equal((await post('/settings', { provider: 'fake', model: 'writer', toolMode: 'native', generationMode: 'auto' })).ok, true)
    const created = await post('/sessions', { card: 'seraphina.json' })
    assert.equal(created.ok, true, created.error)
    sessionId = created.sessionId
    const adapter = adapters.get('storyforge')
    const opening = userMessage('（开始新故事）', 'storyforge')
    await collect(adapter.stream({ sessionId, messages: [opening] }))
    const turn = () =>
      collect(
        adapter.stream({
          sessionId,
          messages: [
            opening,
            { id: crypto.randomUUID(), role: 'assistant', content: [{ type: 'text', text: OPENING }], source: { kind: 'model', provider: 'storyforge', model: 'pipeline' } },
            userMessage(`${CREW.join('、')}围坐在炉火边，商量明天怎么走出森林。`, 'user'),
          ],
          signal: new AbortController().signal,
        }),
      )
    const writerCalls = () => llmCalls.filter((options) => !options.system?.includes('卡内角色识别')).length

    // 没有可问的界面：照旧报错，指到首页设置；没有调用写作模型。
    await assert.rejects(turn(), /管线设置/)
    assert.equal(writerCalls(), 0)

    questions = {
      ask: async (request) => {
        asks.push(request)
        const [question] = request.questions
        const choice = replies.shift()
        return { answers: [{ id: question.id, selected: [question.options[choice].label] }] }
      },
    }
    // 取消：本轮不写。问题挂在本会话的 agent 上，三个选项。
    replies.push(2)
    await assert.rejects(turn(), /已取消这一轮/)
    assert.equal(asks.length, 1)
    assert.equal(asks[0].agent, agent)
    assert.ok(asks[0].signal)
    assert.equal(asks[0].questions[0].options.length, 3)
    assert.match(asks[0].questions[0].question, /群像（顺序子 Agent）/)
    assert.equal(writerCalls(), 0)

    // 选「这轮按续写」：带显式模式重发，写完一轮。
    replies.push(1)
    const chunks = await turn()
    assert.equal(asks.length, 2)
    assert.equal(chunks.at(-1).type, 'finish')
    const reasoning = chunks.filter((chunk) => chunk.type === 'reasoning-delta').map((chunk) => chunk.text).join('')
    assert.match(reasoning, /【确认】自动路由建议这一轮用群像（顺序子 Agent）/)
    assert.match(reasoning, /【确认】这一轮用续写（单次执笔）/)
    assert.match(reasoning, /【本轮】续写（单次执笔）/)
    const text = chunks.filter((chunk) => chunk.type === 'text-delta').map((chunk) => chunk.text).join('')
    assert.equal(text, PROSE)
    assert.ok(writerCalls() >= 1)

    let status
    for (let i = 0; i < 100; i++) {
      status = await call(routes, '/status')
      if (status.settles.length > 0) break
      await new Promise((r) => setTimeout(r, 200))
    }
    assert.equal(status.settles.length, 1, JSON.stringify(status))
  } finally {
    await teardown(routes, dispose, root)
  }
})

/** 等 sidecar 进程退出（插件卸载不等它，重启同一数据目录前要等）。 */
async function waitForExit(pid, timeoutMs = 15_000) {
  if (!pid) return
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      process.kill(pid, 0)
    } catch {
      return
    }
    await new Promise((done) => setTimeout(done, 100))
  }
  assert.fail(`sidecar ${pid} 没有退出`)
}
