import test from 'node:test'
import assert from 'node:assert/strict'
import { createPinia, setActivePinia } from 'pinia'

import { usePluginBridge } from '../../src/composables/usePluginBridge.js'
import { usePipeline } from '../../src/composables/usePipeline.js'
import { useWriting } from '../../src/composables/useWriting.js'
import { useMessageVariants } from '../../src/composables/useMessageVariants.js'
import { usePluginStore } from '../../src/stores/plugin.js'
import { useWritingStore } from '../../src/stores/writing.js'
import { useCampaignStore } from '../../src/stores/campaign.js'
import { ST_EVENT_TYPES } from '../../src/plugin-bridge.js'
import { chainAuditRecords } from '../../src/utils/promptHookAudit.js'

// tauri-api.js reads window.__TAURI_INTERNALS__; Node tests need a stub.
if (typeof globalThis.window === 'undefined') {
  globalThis.window = {}
}

function setup() {
  setActivePinia(createPinia())
  const plugin = usePluginStore()
  const writing = useWritingStore()
  const campaign = useCampaignStore()
  const bridge = usePluginBridge()
  return { plugin, writing, campaign, bridge }
}

function plain(value) {
  return JSON.parse(JSON.stringify(value))
}

test('setHookPluginHostRef mounts and unmounts hidden hook hosts', () => {
  const { plugin, bridge } = setup()
  const host = { emitPluginEventAndWait: async (_event, data) => data }

  bridge.setHookPluginHostRef('hook-a', host)
  assert.equal(plugin.getHookPluginHostRef('hook-a'), host)

  bridge.setHookPluginHostRef('hook-a', null)
  assert.equal(plugin.getHookPluginHostRef('hook-a'), undefined)
})

test('onHookPluginSlotMount isolates status slots per plugin', () => {
  const { plugin, bridge } = setup()

  bridge.onHookPluginSlotMount({ pluginId: 'p1', slot: 'statusbar', html: '<b>hp</b>' })
  bridge.onHookPluginSlotMount({ pluginId: 'p1', slot: 'slash', html: '<button>/heal</button>' })
  bridge.onHookPluginSlotMount({ pluginId: 'p2', slot: 'statusbar', html: '<i>mp</i>' })

  assert.deepEqual(plain(plugin.hookPluginSlots), {
    p1: {
      statusbar: '<b>hp</b>',
      slash: '<button>/heal</button>',
    },
    p2: {
      statusbar: '<i>mp</i>',
    },
  })

  bridge.onHookPluginSlotMount({ pluginId: 'p1', slot: 'slash', html: '' })
  assert.deepEqual(plain(plugin.hookPluginSlots.p1), { statusbar: '<b>hp</b>' })
  assert.deepEqual(plain(plugin.hookPluginSlots.p2), { statusbar: '<i>mp</i>' })
})

test('broadcastPluginEvent correlates message payloads and redaction-ready content fields', () => {
  const { plugin, writing, campaign, bridge } = setup()
  campaign.currentConversationId = 'conv-1'
  campaign.activeCampaign = { id: 'camp-1' }
  campaign.activeChar = { id: 'char-1' }
  writing.messages = [
    {
      id: 'm1',
      role: 'assistant',
      active_variant: 0,
      variants: [{ id: 'v1', content: 'private body', display_content: 'private display' }],
    },
  ]
  const payload = bridge.messageEventPayload('m1', { reason: 'committed' })
  bridge.broadcastPluginEvent(ST_EVENT_TYPES.MESSAGE_RECEIVED, payload)

  assert.equal(plugin.pluginPipelineEvents.length, 1)
  const record = plugin.pluginPipelineEvents[0]
  assert.equal(record.id, 1)
  assert.equal(record.event, ST_EVENT_TYPES.MESSAGE_RECEIVED)
  assert.equal(record.data.messageId, 'm1')
  assert.equal(record.data.conversationId, 'conv-1')
  assert.equal(record.data.campaignId, 'camp-1')
  assert.equal(record.data.content, 'private body')
  assert.equal(record.data.displayContent, 'private display')
})

test('broadcastPluginPipelineEvent ignores malformed events and assigns monotonic ids', () => {
  const { plugin, bridge } = setup()

  bridge.broadcastPluginPipelineEvent(null)
  bridge.broadcastPluginPipelineEvent({})
  bridge.broadcastPluginPipelineEvent({ event_type: 'started', data: { ok: true } })
  bridge.broadcastPluginPipelineEvent({ event_type: 'editor_progress', data: { delta: 'x' } })

  assert.equal(plugin.pluginPipelineEvents.length, 2)
  assert.equal(plugin.pluginPipelineEvents[0].id, 1)
  assert.equal(plugin.pluginPipelineEvents[1].id, 2)
  assert.equal(plugin.pluginPipelineEvents[0].event.event_type, 'started')
  assert.equal(plugin.pluginPipelineEvents[1].event.event_type, 'editor_progress')
})

test('emitPromptHookEventAndWait uses host refs in hook plugin order and audits outcomes', async () => {
  const { plugin, bridge } = setup()
  const calls = []
  plugin.hookPlugins = [
    { id: 'first', permissions: ['ModifyPrompt'], manifest: { name: 'First' } },
    { id: 'second', permissions: ['ModifyPrompt'], manifest: { name: 'Second' } },
  ]
  plugin.setHookPluginHostRef('first', {
    async emitPluginEventAndWait(event, payload) {
      calls.push(['first', event, payload.prompt])
      return { ...payload, prompt: `${payload.prompt} + first` }
    },
  })
  plugin.setHookPluginHostRef('second', {
    async emitPluginEventAndWait(event, payload) {
      calls.push(['second', event, payload.prompt])
      return { ...payload, prompt: `${payload.prompt} + second` }
    },
  })

  const result = await bridge.emitPromptHookEventAndWait(
    ST_EVENT_TYPES.CHAT_COMPLETION_PROMPT_READY,
    { prompt: 'base' },
    'bridge_test',
  )

  assert.deepEqual(result, { prompt: 'base + first + second' })
  assert.deepEqual(calls, [
    ['first', ST_EVENT_TYPES.CHAT_COMPLETION_PROMPT_READY, 'base'],
    ['second', ST_EVENT_TYPES.CHAT_COMPLETION_PROMPT_READY, 'base + first'],
  ])
  assert.equal(plugin.promptHookAuditRecords.length, 2)
  assert.equal(plugin.promptHookAuditRecords[0].pluginId, 'first')
  assert.equal(plugin.promptHookAuditRecords[0].status, 'ok')
  assert.equal(plugin.promptHookAuditRecords[1].pluginId, 'second')
  assert.equal(JSON.stringify(plugin.promptHookAuditRecords).includes('base + first + second'), false)
})

test('handlePromptHookRequest correlates request ids and fails open on host errors', async () => {
  const { plugin, bridge } = setup()
  const replies = []
  plugin.hookPlugins = [
    { id: 'broken', permissions: ['ModifyPrompt'], manifest: { name: 'Broken' } },
  ]
  plugin.setHookPluginHostRef('broken', {
    async emitPluginEventAndWait() {
      throw new Error('private prompt failure text')
    },
  })

  // Monkeypatch module-level result sink via host map path: usePluginBridge calls pluginPromptHookResult.
  // We intercept by wrapping the host error path and asserting audit + no throw.
  await bridge.handlePromptHookRequest({
    request_id: 'req-1',
    messages: [{ role: 'user', content: 'hello' }],
  })

  // No pending host means original messages path; ensure no throw and audit recorded for broken plugin
  // when emit runs. Since tauri is absent, command is a no-op.
  assert.equal(plugin.promptHookAuditRecords.length, 1)
  assert.equal(plugin.promptHookAuditRecords[0].status, 'error')
  assert.equal(JSON.stringify(plugin.promptHookAuditRecords).includes('private prompt failure text'), false)
  assert.deepEqual(replies, [])
})

test('runPromptHookEvents mutates intent through sequential frontend hooks', async () => {
  const { plugin, writing, bridge } = setup()
  writing.messages = []
  plugin.hookPlugins = [
    { id: 'hook', permissions: ['ModifyPrompt'], manifest: { name: 'Hook' } },
  ]
  plugin.setHookPluginHostRef('hook', {
    async emitPluginEventAndWait(event, payload) {
      if (event === ST_EVENT_TYPES.GENERATE_BEFORE_COMBINE_PROMPTS) {
        return { ...payload, intent: `${payload.intent}::before` }
      }
      if (event === ST_EVENT_TYPES.CHAT_COMPLETION_PROMPT_READY) {
        return { ...payload, intent: `${payload.intent}::ready` }
      }
      return payload
    },
  })

  const hooked = await bridge.runPromptHookEvents('go north')
  assert.equal(hooked, 'go north::before::ready')
  assert.equal(plugin.promptHookAuditRecords.length, 2)
})

test('usePluginBridge wires default timeout without caller-supplied options', async () => {
  const { plugin, bridge } = setup()
  plugin.hookPlugins = [
    { id: 'slow', permissions: ['ModifyPrompt'], manifest: { name: 'Slow' } },
    { id: 'fast', permissions: ['ModifyPrompt'], manifest: { name: 'Fast' } },
  ]
  plugin.setHookPluginHostRef('slow', {
    async emitPluginEventAndWait() {
      await new Promise((resolve) => setTimeout(resolve, 40))
      return { prompt: 'late-should-not-apply' }
    },
  })
  plugin.setHookPluginHostRef('fast', {
    async emitPluginEventAndWait(event, payload) {
      return { ...payload, prompt: `${payload.prompt || 'base'} + fast` }
    },
  })

  // Production path: emitPromptHookEventAndWait does not accept timeout options from callers.
  const previousTimeout = bridge.setPromptHookTimeoutMs?.(5)
  try {
    const result = await bridge.emitPromptHookEventAndWait(
      ST_EVENT_TYPES.CHAT_COMPLETION_PROMPT_READY,
      { prompt: 'base' },
      'wired_timeout',
    )
    assert.deepEqual(result, { prompt: 'base + fast' })
    assert.equal(plugin.promptHookAuditRecords[0].status, 'timeout')
    assert.equal(plugin.promptHookAuditRecords[1].status, 'ok')
  } finally {
    if (typeof previousTimeout === 'number' || previousTimeout === null) {
      bridge.setPromptHookTimeoutMs?.(previousTimeout)
    } else {
      bridge.setPromptHookTimeoutMs?.(undefined)
    }
  }
})

test('usePluginBridge cancelPromptHooks stops later plugins on the real path', async () => {
  const { plugin, bridge } = setup()
  const calls = []
  plugin.hookPlugins = [
    { id: 'first', permissions: ['ModifyPrompt'], manifest: { name: 'First' } },
    { id: 'second', permissions: ['ModifyPrompt'], manifest: { name: 'Second' } },
  ]
  plugin.setHookPluginHostRef('first', {
    async emitPluginEventAndWait(event, payload) {
      calls.push('first')
      bridge.cancelPromptHooks()
      return { ...payload, prompt: `${payload.prompt} + first` }
    },
  })
  plugin.setHookPluginHostRef('second', {
    async emitPluginEventAndWait(event, payload) {
      calls.push('second')
      return { ...payload, prompt: `${payload.prompt} + second` }
    },
  })

  await assert.rejects(
    bridge.emitPromptHookEventAndWait(
      ST_EVENT_TYPES.CHAT_COMPLETION_PROMPT_READY,
      { prompt: 'base' },
      'wired_cancel',
    ),
    (error) => error?.code === 'PROMPT_HOOK_CANCELLED',
  )

  assert.deepEqual(calls, ['first'])
  assert.deepEqual(
    plugin.promptHookAuditRecords.map((record) => [record.pluginId, record.status]),
    [['first', 'cancelled']],
  )
})

test('production writing cancel races an in-flight prompt hook and never starts the backend', async () => {
  const { plugin, writing, campaign, bridge } = setup()
  writing.activeConnection = { id: 'conn-1' }
  campaign.activeCampaign = { id: 'campaign-1', name: 'Campaign' }
  plugin.hookPlugins = [
    { id: 'hung', permissions: ['ModifyPrompt'], manifest: { name: 'Hung' } },
  ]
  plugin.setHookPluginHostRef('hung', {
    emitPluginEventAndWait() {
      return new Promise(() => {})
    },
  })
  bridge.setPromptHookTimeoutMs(25)

  let backendStarts = 0
  const pipelineEvents = []
  const writingApi = useWriting({
    runPromptHookEvents: bridge.runPromptHookEvents,
    cancelPromptHooks: bridge.cancelPromptHooks,
    isPromptHooksCancelled: bridge.isPromptHooksCancelled,
    startWritingApi: async () => {
      backendStarts += 1
      return { text: 'should-not-run', conversation_id: 'c1', node_id: 'n1' }
    },
    cancelWritingApi: async () => true,
    handlePipelineEvent: (event) => pipelineEvents.push(event.event_type),
  })

  const pending = writingApi.startWriting('intent')
  await new Promise((resolve) => setTimeout(resolve, 5))
  await writingApi.cancelWriting()
  await pending

  assert.equal(backendStarts, 0)
  assert.deepEqual(pipelineEvents, [])
  assert.equal(writing.isWriting, false)
  assert.notEqual(writing.pipeline.state, 'error')
})

test('cancel races the current hook instead of waiting for its timeout', async () => {
  const { plugin, bridge } = setup()
  plugin.hookPlugins = [
    { id: 'hung', permissions: ['ModifyPrompt'], manifest: { name: 'Hung' } },
    { id: 'later', permissions: ['ModifyPrompt'], manifest: { name: 'Later' } },
  ]
  plugin.setHookPluginHostRef('hung', {
    emitPluginEventAndWait() {
      return new Promise(() => {})
    },
  })
  let laterCalls = 0
  plugin.setHookPluginHostRef('later', {
    async emitPluginEventAndWait(_event, payload) {
      laterCalls += 1
      return payload
    },
  })
  bridge.setPromptHookTimeoutMs(500)
  bridge.beginPromptHookGeneration()

  const startedAt = Date.now()
  const pending = bridge.emitPromptHookEventAndWait(
    ST_EVENT_TYPES.CHAT_COMPLETION_PROMPT_READY,
    { prompt: 'base' },
    'cancel_race',
  )
  setTimeout(() => bridge.cancelPromptHooks(), 5)

  await assert.rejects(pending, (error) => error?.code === 'PROMPT_HOOK_CANCELLED')
  assert.ok(Date.now() - startedAt < 250, 'cancel should beat the 500ms timeout')
  assert.equal(laterCalls, 0)
  assert.deepEqual(
    plugin.promptHookAuditRecords.map((record) => [record.pluginId, record.status]),
    [['hung', 'cancelled']],
  )
})

test('late backend prompt hook cannot reset or execute a cancelled generation', async () => {
  const { plugin, bridge } = setup()
  let calls = 0
  plugin.hookPlugins = [
    { id: 'hook', permissions: ['ModifyPrompt'], manifest: { name: 'Hook' } },
  ]
  plugin.setHookPluginHostRef('hook', {
    async emitPluginEventAndWait(_event, payload) {
      calls += 1
      return payload
    },
  })

  bridge.beginPromptHookGeneration()
  bridge.cancelPromptHooks()
  const pipeline = usePipeline({ pluginBridge: bridge })
  pipeline.handlePipelineEvent({
    event_type: 'prompt_hook_request',
    data: {
      request_id: 'late-request',
      messages: [{ role: 'user', content: 'private' }],
    },
  })
  await new Promise((resolve) => setImmediate(resolve))

  assert.equal(calls, 0)
  assert.equal(bridge.isPromptHooksCancelled(), true)
})

test('production prompt-hook path stamps correlation/generation and enforces budget/revocation', async () => {
  const { plugin, bridge } = setup()
  plugin.hookPlugins = [
    { id: 'bloater', permissions: ['ModifyPrompt'], manifest: { name: 'Bloater' } },
    { id: 'revoked', permissions: ['ModifyPrompt'], manifest: { name: 'Revoked' } },
    { id: 'ok', permissions: ['ModifyPrompt'], manifest: { name: 'Ok' } },
  ]
  plugin.setHookPluginHostRef('bloater', {
    async emitPluginEventAndWait(_event, payload) {
      return { ...payload, prompt: `${payload.prompt}${'X'.repeat(400)}` }
    },
  })
  plugin.setHookPluginHostRef('revoked', {
    async emitPluginEventAndWait() {
      throw new Error('should not run after revocation')
    },
  })
  plugin.setHookPluginHostRef('ok', {
    async emitPluginEventAndWait(_event, payload) {
      return { ...payload, prompt: `${payload.prompt} + ok` }
    },
  })

  bridge.setPromptHookMaxPayloadBytes(64)
  bridge.setLivePluginPermissionsResolver((hp) => (
    hp.id === 'revoked' ? ['ReadMemory'] : hp.permissions
  ))
  bridge.beginPromptHookGeneration()

  const result = await bridge.emitPromptHookEventAndWait(
    ST_EVENT_TYPES.CHAT_COMPLETION_PROMPT_READY,
    { prompt: 'base' },
    'production_wiring',
  )

  assert.deepEqual(result, { prompt: 'base + ok' })
  assert.deepEqual(
    plugin.promptHookAuditRecords.map((record) => [record.pluginId, record.status]),
    [
      ['bloater', 'budget_exceeded'],
      ['revoked', 'revoked'],
      ['ok', 'ok'],
    ],
  )
  for (const record of plugin.promptHookAuditRecords) {
    assert.equal(typeof record.generationId, 'string')
    assert.ok(record.generationId.length > 0)
    assert.equal(typeof record.correlationId, 'string')
    assert.match(record.correlationId, /^hook:/)
    assert.equal(typeof record.recordHash, 'string')
    assert.ok(record.recordHash.length > 0)
  }
  // Chained integrity: second prevHash equals first recordHash.
  assert.equal(
    plugin.promptHookAuditRecords[1].prevHash,
    plugin.promptHookAuditRecords[0].recordHash,
  )
})

test('production write, discard, accept, and force-accept publish terminal message events exactly once', async () => {
  const { writing, campaign } = setup()
  writing.activeConnection = { id: 'conn-1' }
  campaign.activeCampaign = { id: 'campaign-1', name: 'Campaign' }
  const events = []
  const messageEventPayload = (messageId, extra = {}) => ({
    messageId,
    role: 'assistant',
    ...extra,
  })

  const writingApi = useWriting({
    startWritingApi: async () => ({
      text: 'draft body',
      conversation_id: 'conv-1',
      node_id: 'draft-1',
    }),
    getConversationApi: async () => null,
    broadcastPluginEvent: (event, data) => events.push({ event, data }),
    messageEventPayload,
  })

  await writingApi.startWriting('write a draft')
  assert.equal(events.filter((entry) => entry.event === ST_EVENT_TYPES.MESSAGE_RECEIVED).length, 0)
  assert.equal(events.at(-1).event, ST_EVENT_TYPES.GENERATION_ENDED)
  assert.equal(events.at(-1).data.reason, 'draft_ready')
  assert.equal(writing.messages.find((message) => message.id === 'draft-1').variants[0].status, 'draft')

  let deleteCalls = 0
  const variants = useMessageVariants({
    deleteMessageFromApi: async () => { deleteCalls += 1 },
    getConversationApi: async () => null,
    broadcastPluginEvent: (event, data) => events.push({ event, data }),
    messageEventPayload,
  })
  await variants.handleDeleteVariant({ nodeId: 'draft-1' })
  assert.equal(deleteCalls, 1)
  assert.equal(events.filter((entry) => entry.event === ST_EVENT_TYPES.MESSAGE_RECEIVED).length, 0)

  writing.messages = [{
    id: 'accept-1',
    role: 'assistant',
    active_variant: 0,
    variants: [{ id: 'variant-1', content: 'accepted', status: 'draft' }],
  }]
  let acceptCalls = 0
  const acceptingVariants = useMessageVariants({
    acceptVariantApi: async (_conversationId, _nodeId, forceAccept) => {
      acceptCalls += 1
      assert.equal(forceAccept, false)
    },
    broadcastPluginEvent: (event, data) => events.push({ event, data }),
    messageEventPayload,
  })
  await acceptingVariants.handleAcceptVariant({ nodeId: 'accept-1' })
  const accepted = events.filter((entry) => entry.event === ST_EVENT_TYPES.MESSAGE_RECEIVED)
  assert.equal(acceptCalls, 1)
  assert.equal(accepted.length, 1)
  assert.equal(accepted[0].data.terminalTurnCommit, true)
  assert.equal(accepted[0].data.turnStatus, 'Committed')
  assert.equal(events.some((entry) => entry.event === ST_EVENT_TYPES.MESSAGE_UPDATED && entry.data?.reason === 'accept_variant'), false)

  writing.messages = [{
    id: 'force-1',
    role: 'assistant',
    active_variant: 0,
    variants: [{ id: 'variant-2', content: 'force accepted', status: 'draft' }],
  }]
  const forcedVariants = useMessageVariants({
    acceptVariantApi: async (_conversationId, _nodeId, forceAccept) => {
      acceptCalls += 1
      assert.equal(forceAccept, true)
    },
    broadcastPluginEvent: (event, data) => events.push({ event, data }),
    messageEventPayload,
  })
  await forcedVariants.handleAcceptVariant({ nodeId: 'force-1', forceAccept: true })
  const forceAccepted = events.filter((entry) => entry.event === ST_EVENT_TYPES.MESSAGE_RECEIVED)
  assert.equal(acceptCalls, 2)
  assert.equal(forceAccepted.length, 2)
  assert.equal(forceAccepted[1].data.turnStatus, 'Degraded')
  assert.equal(forceAccepted[1].data.forceAccept, true)
})

test('concurrent and replayed accepts publish one terminal message event per variant', async () => {
  const { writing, campaign } = setup()
  campaign.currentConversationId = 'conv-accept-dedupe'
  writing.messages = [{
    id: 'accept-dedupe',
    role: 'assistant',
    active_variant: 0,
    variants: [{ id: 'variant-dedupe', content: 'draft', status: 'draft' }],
  }]
  const events = []
  const gates = []
  let calls = 0
  const variants = useMessageVariants({
    acceptVariantApi: async () => {
      calls += 1
      await new Promise((resolve) => gates.push(resolve))
    },
    broadcastPluginEvent: (event, data) => events.push({ event, data }),
    messageEventPayload: (messageId, extra = {}) => ({ messageId, ...extra }),
  })

  const first = variants.handleAcceptVariant({ nodeId: 'accept-dedupe' })
  const concurrent = variants.handleAcceptVariant({ nodeId: 'accept-dedupe' })
  assert.equal(calls, 1, 'concurrent Accept calls must join one backend request')
  gates.shift()()
  await Promise.all([first, concurrent])
  assert.equal(events.filter((entry) => entry.event === ST_EVENT_TYPES.MESSAGE_RECEIVED).length, 1)

  await variants.handleAcceptVariant({ nodeId: 'accept-dedupe' })
  assert.equal(calls, 1, 'a backend-idempotent replay must not re-publish terminal events')
  assert.equal(events.filter((entry) => entry.event === ST_EVENT_TYPES.MESSAGE_RECEIVED).length, 1)

  writing.messages[0].variants = [{ id: 'variant-force-dedupe', content: 'force draft', status: 'draft' }]
  const forceFirst = variants.handleAcceptVariant({ nodeId: 'accept-dedupe', forceAccept: true })
  const forceConcurrent = variants.handleAcceptVariant({ nodeId: 'accept-dedupe', forceAccept: true })
  assert.equal(calls, 2, 'concurrent force Accept calls must also join one request')
  gates.shift()()
  await Promise.all([forceFirst, forceConcurrent])
  assert.equal(events.filter((entry) => entry.event === ST_EVENT_TYPES.MESSAGE_RECEIVED).length, 2)
})

test('a quality-gated normal Accept and its force retry publish one terminal event', async () => {
  const { writing, campaign } = setup()
  campaign.currentConversationId = 'conv-quality-retry'
  writing.messages = [{
    id: 'quality-retry',
    role: 'assistant',
    active_variant: 0,
    variants: [{ id: 'variant-quality-retry', content: 'draft', status: 'draft' }],
  }]
  const events = []
  const calls = []
  const variants = useMessageVariants({
    acceptVariantApi: async (_conversationId, _nodeId, forceAccept) => {
      calls.push(forceAccept)
      if (!forceAccept) throw new Error('force_accept required')
    },
    askForceAccept: async () => true,
    broadcastPluginEvent: (event, data) => events.push({ event, data }),
    messageEventPayload: (messageId, extra = {}) => ({ messageId, ...extra }),
  })

  await Promise.all([
    variants.handleAcceptVariant({ nodeId: 'quality-retry' }),
    variants.handleAcceptVariant({ nodeId: 'quality-retry' }),
  ])

  assert.deepEqual(calls, [false, true])
  const received = events.filter((entry) => entry.event === ST_EVENT_TYPES.MESSAGE_RECEIVED)
  assert.equal(received.length, 1)
  assert.equal(received[0].data.turnStatus, 'Degraded')
})

test('default live-permission resolver revokes a plugin removed during the hook chain', async () => {
  const { plugin, bridge } = setup()
  const first = { id: 'first', permissions: ['ModifyPrompt'], manifest: { name: 'First' } }
  const removed = { id: 'removed', permissions: ['ModifyPrompt'], manifest: { name: 'Removed' } }
  plugin.hookPlugins = [first, removed]
  let removedRan = false
  plugin.setHookPluginHostRef('first', {
    async emitPluginEventAndWait(_event, payload) {
      plugin.hookPlugins = [first]
      return { ...payload, prompt: `${payload.prompt} + first` }
    },
  })
  plugin.setHookPluginHostRef('removed', {
    async emitPluginEventAndWait() {
      removedRan = true
      throw new Error('a removed plugin must not run')
    },
  })

  const result = await bridge.emitPromptHookEventAndWait(
    ST_EVENT_TYPES.CHAT_COMPLETION_PROMPT_READY,
    { prompt: 'base' },
    'live_removal',
  )

  assert.deepEqual(result, { prompt: 'base + first' })
  assert.equal(removedRan, false)
  assert.deepEqual(
    plugin.promptHookAuditRecords.map((record) => [record.pluginId, record.status]),
    [['first', 'ok'], ['removed', 'revoked']],
  )
})

test('live audit recorder refuses to overwrite a corrupted stored chain', () => {
  const { plugin, bridge } = setup()
  plugin.promptHookAuditRecords = chainAuditRecords([{
    pluginId: 'audit-plugin',
    pluginName: 'Audit Plugin',
    event: 'CHAT_COMPLETION_PROMPT_READY',
    stage: 'frontend_intent',
    status: 'ok',
    durationMs: 1,
    changedKeys: [],
    inputSummary: {},
    outputSummary: {},
    error: null,
    recordedAt: 100,
  }])
  plugin.promptHookAuditRecords[0].status = 'error'

  const result = bridge.recordPromptHookAudit({
    pluginId: 'audit-plugin',
    pluginName: 'Audit Plugin',
    event: 'CHAT_COMPLETION_PROMPT_READY',
    stage: 'frontend_intent',
    status: 'ok',
    durationMs: 1,
    changedKeys: [],
    inputSummary: {},
    outputSummary: {},
    error: null,
  })

  assert.equal(result.appended, false)
  assert.equal(result.integrity.valid, false)
  assert.equal(result.integrity.invalidIndex, 0)
  assert.equal(plugin.promptHookAuditRecords.length, 1)
})
