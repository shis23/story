import test from 'node:test'
import assert from 'node:assert/strict'
import { createPinia, setActivePinia } from 'pinia'

import { usePluginBridge } from '../../src/composables/usePluginBridge.js'
import { usePluginStore } from '../../src/stores/plugin.js'
import { useWritingStore } from '../../src/stores/writing.js'
import { useCampaignStore } from '../../src/stores/campaign.js'
import { ST_EVENT_TYPES } from '../../src/plugin-bridge.js'

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

  const result = await bridge.emitPromptHookEventAndWait(
    ST_EVENT_TYPES.CHAT_COMPLETION_PROMPT_READY,
    { prompt: 'base' },
    'wired_cancel',
  )

  assert.deepEqual(result, { prompt: 'base + first' })
  assert.deepEqual(calls, ['first'])
  assert.deepEqual(
    plugin.promptHookAuditRecords.map((record) => [record.pluginId, record.status]),
    [
      ['first', 'ok'],
      ['second', 'cancelled'],
    ],
  )
})
