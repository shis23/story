import test from 'node:test'
import assert from 'node:assert/strict'
import vm from 'node:vm'
import {
  createHostHandler,
  createPluginHookBridge,
  generateBridgeScript,
  canModifyPrompt,
  canReadMemory,
  mapPipelineEventToPluginEvents,
  mapPluginEventRecordToPluginEvents,
  MSG_HOOK_REQUEST,
  MSG_HOOK_RESPONSE,
  MSG_REQUEST,
  PROMPT_HOOK_PERMISSION,
  READ_MEMORY_PERMISSION,
  ST_EVENT_TYPES,
} from '../src/plugin-bridge.js'

function createBridgeSandbox(pluginId = 'plugin-a', hostOrigin = 'https://storyforge.local') {
  const listeners = {}
  const postedMessages = []
  const storage = new Map()
  const window = {
    addEventListener: (name, callback) => {
      listeners[name] = callback
    },
    localStorage: {
      getItem: (key) => storage.get(key) ?? null,
      setItem: (key, value) => storage.set(key, String(value)),
    },
    console,
  }
  const sandbox = {
    window,
    parent: {
      postMessage: (message, targetOrigin) => postedMessages.push({ message, targetOrigin }),
    },
    localStorage: window.localStorage,
    console,
  }
  sandbox.globalThis = sandbox

  const script = generateBridgeScript(pluginId, hostOrigin)
    .replace(/^<script>\n?/, '')
    .replace(/\n?<\/script>$/, '')
  vm.runInNewContext(script, sandbox)

  function postHostMessage(data) {
    listeners.message({
      data,
      source: sandbox.parent,
      origin: hostOrigin,
    })
  }

  return { window, listeners, postedMessages, postHostMessage }
}

function plain(value) {
  return JSON.parse(JSON.stringify(value))
}

function flushPromises() {
  return new Promise((resolve) => setImmediate(resolve))
}

test('bridge posts plugin messages to the configured host origin', () => {
  const { window, postedMessages } = createBridgeSandbox('plugin-a', 'https://host.example')

  assert.equal(postedMessages.at(-1).message.type, 'sf:ready')
  assert.equal(postedMessages.at(-1).targetOrigin, 'https://host.example')

  window.storyforge.ui.mountToSlot('sidebar', '<b>hello</b>')
  assert.equal(postedMessages.at(-1).message.type, 'sf:ui:mount')
  assert.equal(postedMessages.at(-1).targetOrigin, 'https://host.example')

  window.storyforge.character.list()
  assert.equal(postedMessages.at(-1).message.type, MSG_REQUEST)
  assert.equal(postedMessages.at(-1).targetOrigin, 'https://host.example')
})

test('bridge keeps local-only shims usable when host postMessage is unavailable', async () => {
  const listeners = {}
  const storage = new Map()
  const window = {
    addEventListener: (name, callback) => {
      listeners[name] = callback
    },
    localStorage: {
      getItem: (key) => storage.get(key) ?? null,
      setItem: (key, value) => storage.set(key, String(value)),
    },
    console,
  }
  const sandbox = {
    window,
    parent: {},
    localStorage: window.localStorage,
    console,
  }
  sandbox.globalThis = sandbox

  const script = generateBridgeScript('plugin-a', 'https://host.example')
    .replace(/^<script>\n?/, '')
    .replace(/\n?<\/script>$/, '')

  assert.doesNotThrow(() => vm.runInNewContext(script, sandbox))
  window.TavernHelper.storageSet('mode', { value: 'local' })
  assert.deepEqual(plain(window.TavernHelper.storageGet('mode')), { value: 'local' })

  window.SillyTavern.chat.push({ name: 'User', mes: 'hello' })
  assert.equal(window.getChatMessages('0').length, 1)
  assert.doesNotThrow(() => window.TavernHelper.setStatusBar('<b>offline</b>'))
  await assert.rejects(
    window.storyforge.character.list(),
    /postMessage unavailable/,
  )
})

test('host handler ignores untrusted sources and replies to request origin', async () => {
  const plugin = { id: 'plugin-a', permissions: ['ReadCharacters'] }
  const trustedSource = {
    posted: [],
    postMessage(message, targetOrigin) {
      this.posted.push({ message, targetOrigin })
    },
  }
  const otherSource = {
    posted: [],
    postMessage(message, targetOrigin) {
      this.posted.push({ message, targetOrigin })
    },
  }
  let invokeCount = 0
  const handler = createHostHandler(
    plugin,
    async () => {
      invokeCount += 1
      return ['Seraphina']
    },
    { isTrustedSource: (event) => event.source === trustedSource },
  )

  const request = {
    type: MSG_REQUEST,
    pluginId: 'plugin-a',
    id: 'req-1',
    method: 'character.list',
    params: {},
  }
  await handler({ data: request, source: otherSource, origin: 'https://evil.example' })
  assert.equal(invokeCount, 0)
  assert.equal(otherSource.posted.length, 0)

  await handler({ data: request, source: trustedSource, origin: 'https://plugin.example' })
  assert.equal(invokeCount, 1)
  assert.equal(trustedSource.posted.length, 1)
  assert.equal(trustedSource.posted[0].targetOrigin, 'https://plugin.example')
  assert.deepEqual(trustedSource.posted[0].message.result, ['Seraphina'])
})

test('host handler keeps local-only storage isolated when source cannot receive replies', async () => {
  const plugin = { id: 'plugin-a', permissions: [] }
  const source = {}
  let invokeCount = 0
  const handler = createHostHandler(plugin, async () => {
    invokeCount += 1
    return null
  })

  await assert.doesNotReject(handler({
    data: {
      type: MSG_REQUEST,
      pluginId: 'plugin-a',
      id: 'set-1',
      method: 'storage.set',
      params: { key: 'layout', value: { statusbar: true } },
    },
    source,
    origin: 'https://plugin.example',
  }))

  assert.equal(invokeCount, 0)
})

test('host handler supports plugin storage set/get without backend invoke', async () => {
  const plugin = { id: 'plugin-a', permissions: [] }
  const source = {
    posted: [],
    postMessage(message, targetOrigin) {
      this.posted.push({ message, targetOrigin })
    },
  }
  let invokeCount = 0
  const handler = createHostHandler(plugin, async () => {
    invokeCount += 1
    return null
  })

  await handler({
    data: {
      type: MSG_REQUEST,
      pluginId: 'plugin-a',
      id: 'set-1',
      method: 'storage.set',
      params: { key: 'layout', value: { statusbar: true } },
    },
    source,
    origin: 'https://plugin.example',
  })
  await handler({
    data: {
      type: MSG_REQUEST,
      pluginId: 'plugin-a',
      id: 'get-1',
      method: 'storage.get',
      params: { key: 'layout' },
    },
    source,
    origin: 'https://plugin.example',
  })

  assert.equal(invokeCount, 0)
  assert.equal(source.posted[0].message.result, true)
  assert.deepEqual(source.posted[1].message.result, { statusbar: true })
  assert.equal(source.posted[1].targetOrigin, 'https://plugin.example')
})

test('host handler routes plugin APIs through plugin-scoped backend commands', async () => {
  const plugin = { id: 'plugin-a', permissions: ['ReadCharacters', 'ReadVariables', 'WriteVariables'] }
  const source = {
    posted: [],
    postMessage(message, targetOrigin) {
      this.posted.push({ message, targetOrigin })
    },
  }
  const calls = []
  const handler = createHostHandler(plugin, async (command, params) => {
    calls.push({ command, params })
    return { ok: true }
  })

  await handler({
    data: {
      type: MSG_REQUEST,
      pluginId: 'plugin-a',
      id: 'list-1',
      method: 'character.list',
      params: {},
    },
    source,
    origin: 'https://plugin.example',
  })
  await handler({
    data: {
      type: MSG_REQUEST,
      pluginId: 'plugin-a',
      id: 'get-1',
      method: 'character.get',
      params: { id: 'char-1' },
    },
    source,
    origin: 'https://plugin.example',
  })
  await handler({
    data: {
      type: MSG_REQUEST,
      pluginId: 'plugin-a',
      id: 'vars-1',
      method: 'variables.get',
      params: { campaignId: 'camp-1', instanceId: 'inst-1' },
    },
    source,
    origin: 'https://plugin.example',
  })
  await handler({
    data: {
      type: MSG_REQUEST,
      pluginId: 'plugin-a',
      id: 'set-1',
      method: 'variables.set',
      params: { campaignId: 'camp-1', instanceId: 'inst-1', key: 'hp', value: 12 },
    },
    source,
    origin: 'https://plugin.example',
  })

  assert.deepEqual(calls, [
    { command: 'plugin_list_characters', params: { pluginId: 'plugin-a' } },
    { command: 'plugin_read_character', params: { pluginId: 'plugin-a', characterId: 'char-1' } },
    {
      command: 'plugin_get_variable',
      params: { pluginId: 'plugin-a', campaignId: 'camp-1', instanceId: 'inst-1' },
    },
    {
      command: 'plugin_set_variable',
      params: {
        pluginId: 'plugin-a',
        campaignId: 'camp-1',
        instanceId: 'inst-1',
        key: 'hp',
        value: 12,
      },
    },
  ])
})

test('host handler maps plugin LLM generation to start_writing intent', async () => {
  const plugin = { id: 'plugin-a', permissions: ['CallLlm'] }
  const source = {
    posted: [],
    postMessage(message, targetOrigin) {
      this.posted.push({ message, targetOrigin })
    },
  }
  const calls = []
  const handler = createHostHandler(plugin, async (command, params) => {
    calls.push({ command, params })
    return { text: 'generated' }
  })

  await handler({
    data: {
      type: MSG_REQUEST,
      pluginId: 'plugin-a',
      id: 'llm-1',
      method: 'llm.generate',
      params: { prompt: 'raw prompt' },
    },
    source,
    origin: 'https://plugin.example',
  })
  await handler({
    data: {
      type: MSG_REQUEST,
      pluginId: 'plugin-a',
      id: 'llm-2',
      method: 'llm.generate',
      params: { intent: 'object intent' },
    },
    source,
    origin: 'https://plugin.example',
  })

  assert.deepEqual(calls, [
    { command: 'start_writing', params: { intent: 'raw prompt' } },
    { command: 'start_writing', params: { intent: 'object intent' } },
  ])
  assert.equal(source.posted[0].message.result.text, 'generated')
})

test('host handler separates read and write variable permissions', async () => {
  const source = {
    posted: [],
    postMessage(message, targetOrigin) {
      this.posted.push({ message, targetOrigin })
    },
  }
  let invokeCount = 0
  const handler = createHostHandler({ id: 'plugin-a', permissions: ['ReadVariables'] }, async () => {
    invokeCount += 1
    return []
  })

  await handler({
    data: {
      type: MSG_REQUEST,
      pluginId: 'plugin-a',
      id: 'read-1',
      method: 'variables.get',
      params: { campaignId: 'camp-1', instanceId: 'inst-1' },
    },
    source,
    origin: 'https://plugin.example',
  })
  await handler({
    data: {
      type: MSG_REQUEST,
      pluginId: 'plugin-a',
      id: 'write-1',
      method: 'variables.set',
      params: { campaignId: 'camp-1', instanceId: 'inst-1', key: 'hp', value: 12 },
    },
    source,
    origin: 'https://plugin.example',
  })

  assert.equal(invokeCount, 1)
  assert.deepEqual(source.posted[0].message.result, [])
  assert.match(source.posted[1].message.error, /WriteVariables/)
})

test('maps pipeline events to native plugin event names', () => {
  const rawEvent = { event_type: 'director_started', data: {} }
  const events = mapPipelineEventToPluginEvents(rawEvent)

  assert.deepEqual(events.map((event) => event.event), [
    'pipeline.director_started',
    'director_started',
  ])
  assert.equal(events[0].data.event_type, 'director_started')
  assert.equal(events[0].data.raw, rawEvent)
})

test('adds SillyTavern aliases for generation lifecycle events', () => {
  assert.deepEqual(
    mapPipelineEventToPluginEvents({ event_type: 'started', data: { session_id: 's1' } })
      .map((event) => event.event),
    ['pipeline.started', 'started', 'GENERATION_STARTED'],
  )

  assert.deepEqual(
    mapPipelineEventToPluginEvents({ event_type: 'draft_ready', data: { text: 'done' } })
      .map((event) => event.event),
    ['pipeline.draft_ready', 'draft_ready', 'GENERATION_ENDED'],
  )

  assert.deepEqual(
    mapPipelineEventToPluginEvents({
      event_type: 'committed',
      data: { session_id: 's1', variant_id: 'v1' },
    }).map((event) => event.event),
    ['pipeline.committed', 'committed', 'MESSAGE_RECEIVED'],
  )
})

test('adds STREAM_TOKEN alias with token payload for editor deltas', () => {
  const events = mapPipelineEventToPluginEvents({
    event_type: 'editor_progress',
    data: { delta: 'hello' },
  })

  const streamToken = events.find((event) => event.event === 'STREAM_TOKEN')
  assert.ok(streamToken)
  assert.equal(streamToken.data.token, 'hello')
  assert.equal(streamToken.data.text, 'hello')
  assert.equal(streamToken.data.delta, 'hello')
  assert.equal(streamToken.data.data.delta, 'hello')
})

test('ignores malformed pipeline events', () => {
  assert.deepEqual(mapPipelineEventToPluginEvents(null), [])
  assert.deepEqual(mapPipelineEventToPluginEvents({ data: {} }), [])
})

test('maps generic host plugin event records without losing payload', () => {
  const record = {
    id: 42,
    event: 'CHAT_CHANGED',
    data: { conversationId: 'c1', reason: 'loaded' },
  }

  assert.deepEqual(mapPluginEventRecordToPluginEvents(record), [{
    event: 'CHAT_CHANGED',
    data: { conversationId: 'c1', reason: 'loaded' },
  }])
})

test('maps nested generic host plugin event records', () => {
  const record = {
    id: 43,
    event: {
      name: 'MESSAGE_UPDATED',
      data: { messageId: 'm1' },
    },
  }

  assert.deepEqual(mapPluginEventRecordToPluginEvents(record), [{
    event: 'MESSAGE_UPDATED',
    data: { messageId: 'm1' },
  }])
})

test('keeps existing pipeline event record mapping', () => {
  const record = {
    id: 44,
    event: { event_type: 'committed', data: { variant_id: 'v1' } },
  }

  assert.deepEqual(
    mapPluginEventRecordToPluginEvents(record).map((event) => event.event),
    ['pipeline.committed', 'committed', 'MESSAGE_RECEIVED'],
  )
})

test('filters plugin events by declared subscriptions', () => {
  const record = {
    id: 45,
    event: 'MESSAGE_RECEIVED',
    data: { messageId: 'm1', content: 'secret' },
  }

  assert.deepEqual(mapPluginEventRecordToPluginEvents(record, {
    id: 'plugin-a',
    event_subscriptions: [],
    permissions: ['ReadMemory'],
  }), [])

  assert.deepEqual(mapPluginEventRecordToPluginEvents(record, {
    id: 'plugin-a',
    event_subscriptions: ['CHAT_CHANGED'],
    permissions: ['ReadMemory'],
  }), [])
})

test('redacts message content from subscribed plugins without ReadMemory', () => {
  const record = {
    id: 46,
    event: 'MESSAGE_RECEIVED',
    data: {
      messageId: 'm1',
      content: 'secret content',
      displayContent: '<b>secret</b>',
      nested: {
        text: 'secret nested',
        safe: 'metadata',
      },
    },
  }

  assert.deepEqual(mapPluginEventRecordToPluginEvents(record, {
    id: 'plugin-a',
    event_subscriptions: ['MESSAGE_RECEIVED'],
    permissions: [],
  }), [{
    event: 'MESSAGE_RECEIVED',
    data: {
      messageId: 'm1',
      nested: {
        safe: 'metadata',
      },
    },
  }])
})

test('preserves message content for subscribed plugins with ReadMemory', () => {
  const record = {
    id: 47,
    event: { event_type: 'draft_ready', data: { text: 'final draft' } },
  }

  const events = mapPluginEventRecordToPluginEvents(record, {
    id: 'plugin-a',
    event_subscriptions: ['GENERATION_ENDED'],
    permissions: ['ReadMemory'],
  })

  assert.deepEqual(events.map((event) => event.event), ['GENERATION_ENDED'])
  assert.equal(events[0].data.text, 'final draft')
  assert.equal(events[0].data.raw.event_type, 'draft_ready')
})

test('injects SillyTavern event type aliases into plugin iframe', () => {
  const { window } = createBridgeSandbox()

  assert.equal(ST_EVENT_TYPES.GENERATION_STARTED, 'GENERATION_STARTED')
  assert.equal(window.event_types.GENERATION_STARTED, 'GENERATION_STARTED')
  assert.equal(window.eventTypes.MESSAGE_RECEIVED, 'MESSAGE_RECEIVED')
  assert.equal(window.eventSource.on, window.storyforge.events.on)
})

test('dispatches host events through storyforge.events and ST eventSource', () => {
  const { window, postHostMessage } = createBridgeSandbox()
  const calls = []

  window.storyforge.events.on('GENERATION_STARTED', (payload) => calls.push(['storyforge', payload.session_id]))
  window.eventSource.on(window.event_types.GENERATION_STARTED, (payload) => calls.push(['st', payload.session_id]))

  postHostMessage({
    type: 'sf:api:event',
    event: 'GENERATION_STARTED',
    data: { session_id: 's1' },
  })

  assert.deepEqual(calls, [
    ['storyforge', 's1'],
    ['st', 's1'],
  ])
})

test('supports ST once, makeFirst, makeLast, removeListener, and local emit', async () => {
  const { window } = createBridgeSandbox()
  const calls = []
  const first = () => calls.push('first')
  const middle = () => calls.push('middle')
  const removed = () => calls.push('removed')
  const last = () => calls.push('last')
  const once = () => calls.push('once')

  window.eventSource.on('CHAT_CHANGED', middle)
  window.eventSource.on('CHAT_CHANGED', removed)
  window.eventSource.makeFirst('CHAT_CHANGED', first)
  window.eventSource.makeLast('CHAT_CHANGED', last)
  window.eventSource.once('CHAT_CHANGED', once)
  window.eventSource.removeListener('CHAT_CHANGED', removed)

  await window.eventSource.emit('CHAT_CHANGED', { id: 1 })
  await window.eventSource.emit('CHAT_CHANGED', { id: 2 })

  assert.deepEqual(calls, [
    'first',
    'middle',
    'last',
    'once',
    'first',
    'middle',
    'last',
  ])
})

test('awaits async listeners during ST emit in listener order', async () => {
  const { window } = createBridgeSandbox()
  const calls = []

  window.eventSource.on('CHAT_COMPLETION_PROMPT_READY', async () => {
    calls.push('first-start')
    await Promise.resolve()
    calls.push('first-end')
  })
  window.eventSource.on('CHAT_COMPLETION_PROMPT_READY', () => {
    calls.push('second')
  })

  await window.eventSource.emit('CHAT_COMPLETION_PROMPT_READY', { prompt: 'draft' })

  assert.deepEqual(calls, ['first-start', 'first-end', 'second'])
})

test('supports ST emitAndWait with listener mutation', async () => {
  const { window } = createBridgeSandbox()
  const payload = { prompt: 'base' }
  const calls = []

  window.eventSource.on('GENERATE_BEFORE_COMBINE_PROMPTS', async (eventPayload) => {
    calls.push('first-start')
    await Promise.resolve()
    eventPayload.prompt += ' + first'
    calls.push('first-end')
  })
  window.eventSource.on('GENERATE_BEFORE_COMBINE_PROMPTS', (eventPayload) => {
    calls.push('second')
    eventPayload.prompt += ' + second'
  })

  const result = await window.eventSource.emitAndWait('GENERATE_BEFORE_COMBINE_PROMPTS', payload)

  assert.equal(payload.prompt, 'base + first + second')
  assert.deepEqual(calls, ['first-start', 'first-end', 'second'])
  assert.equal(result, payload)
})

test('supports ST emitAndWait with returned payload chaining', async () => {
  const { window } = createBridgeSandbox()
  const payload = { prompt: 'base' }

  window.eventSource.on('CHAT_COMPLETION_PROMPT_READY', async (eventPayload) => {
    await Promise.resolve()
    return { ...eventPayload, prompt: `${eventPayload.prompt} + returned` }
  })
  window.eventSource.on('CHAT_COMPLETION_PROMPT_READY', (eventPayload) => {
    return { ...eventPayload, prompt: `${eventPayload.prompt} + second` }
  })

  const result = await window.eventSource.emitAndWait('CHAT_COMPLETION_PROMPT_READY', payload)

  assert.equal(payload.prompt, 'base')
  assert.deepEqual(plain(result), { prompt: 'base + returned + second' })
})

test('responds to host hook requests after async ST listener mutation', async () => {
  const { window, postedMessages, postHostMessage } = createBridgeSandbox('plugin-a', 'https://host.example')
  const payload = {
    intent: 'base',
    messages: [{ role: 'user', content: 'hello' }],
  }

  window.eventSource.on('CHAT_COMPLETION_PROMPT_READY', async (eventPayload) => {
    await Promise.resolve()
    eventPayload.messages.push({ role: 'system', content: 'hooked' })
  })
  window.eventSource.on('CHAT_COMPLETION_PROMPT_READY', (eventPayload) => {
    eventPayload.intent += ' + plugin'
  })

  postHostMessage({
    type: MSG_HOOK_REQUEST,
    pluginId: 'plugin-a',
    id: 'hook-1',
    event: 'CHAT_COMPLETION_PROMPT_READY',
    data: payload,
  })
  await flushPromises()

  const response = postedMessages.at(-1)
  assert.equal(response.targetOrigin, 'https://host.example')
  assert.equal(response.message.type, MSG_HOOK_RESPONSE)
  assert.equal(response.message.pluginId, 'plugin-a')
  assert.equal(response.message.id, 'hook-1')
  assert.deepEqual(plain(response.message.result), {
    intent: 'base + plugin',
    messages: [
      { role: 'user', content: 'hello' },
      { role: 'system', content: 'hooked' },
    ],
  })
})

test('responds to host hook requests with returned payload chaining', async () => {
  const { window, postedMessages, postHostMessage } = createBridgeSandbox('plugin-a', 'https://host.example')
  const payload = {
    messages: [{ role: 'user', content: 'hello' }],
  }

  window.eventSource.on('CHAT_COMPLETION_PROMPT_READY', async (eventPayload) => {
    await Promise.resolve()
    return {
      ...eventPayload,
      messages: [
        ...eventPayload.messages,
        { role: 'system', content: 'first returned' },
      ],
    }
  })
  window.eventSource.on('CHAT_COMPLETION_PROMPT_READY', (eventPayload) => ({
    ...eventPayload,
    messages: [
      ...eventPayload.messages,
      { role: 'system', content: 'second returned' },
    ],
  }))

  postHostMessage({
    type: MSG_HOOK_REQUEST,
    pluginId: 'plugin-a',
    id: 'hook-return',
    event: 'CHAT_COMPLETION_PROMPT_READY',
    data: payload,
  })
  await flushPromises()

  const response = postedMessages.at(-1)
  assert.equal(response.message.type, MSG_HOOK_RESPONSE)
  assert.deepEqual(plain(response.message.result), {
    messages: [
      { role: 'user', content: 'hello' },
      { role: 'system', content: 'first returned' },
      { role: 'system', content: 'second returned' },
    ],
  })
  assert.deepEqual(payload.messages, [{ role: 'user', content: 'hello' }])
})

test('does not broadcast backend prompt hook requests through generic plugin events', () => {
  const events = mapPluginEventRecordToPluginEvents({
    event_type: 'prompt_hook_request',
    data: {
      request_id: 'hook-1',
      messages: [{ role: 'system', content: 'secret prompt' }],
    },
  })

  assert.deepEqual(events, [])
})

test('checks explicit ModifyPrompt permission for prompt hooks', () => {
  assert.equal(PROMPT_HOOK_PERMISSION, 'ModifyPrompt')
  assert.equal(READ_MEMORY_PERMISSION, 'ReadMemory')
  assert.equal(canModifyPrompt({ permissions: ['ReadMemory'] }), false)
  assert.equal(canModifyPrompt({ permissions: ['ReadMemory', 'ModifyPrompt'] }), true)
  assert.equal(canReadMemory({ permissions: [] }), false)
  assert.equal(canReadMemory({ permissions: ['ReadMemory'] }), true)
})

test('ignores hook requests that do not come from the host parent', async () => {
  const { window, listeners, postedMessages } = createBridgeSandbox('plugin-a', 'https://host.example')

  window.eventSource.on('CHAT_COMPLETION_PROMPT_READY', (eventPayload) => {
    eventPayload.prompt = 'mutated'
  })

  listeners.message({
    source: { postMessage() {} },
    origin: 'https://host.example',
    data: {
      type: MSG_HOOK_REQUEST,
      pluginId: 'plugin-a',
      id: 'hook-evil',
      event: 'CHAT_COMPLETION_PROMPT_READY',
      data: { prompt: 'base' },
    },
  })
  await flushPromises()

  assert.equal(postedMessages.at(-1).message.type, 'sf:ready')
})

test('reports hook listener errors back to the host', async () => {
  const { window, postedMessages, postHostMessage } = createBridgeSandbox('plugin-a', 'https://host.example')

  window.eventSource.on('GENERATE_BEFORE_COMBINE_PROMPTS', () => {
    throw new Error('hook failed')
  })

  postHostMessage({
    type: MSG_HOOK_REQUEST,
    pluginId: 'plugin-a',
    id: 'hook-err',
    event: 'GENERATE_BEFORE_COMBINE_PROMPTS',
    data: { prompt: 'base' },
  })
  await flushPromises()

  const response = postedMessages.at(-1).message
  assert.equal(response.type, MSG_HOOK_RESPONSE)
  assert.equal(response.id, 'hook-err')
  assert.match(response.error, /hook failed/)
})

test('host hook bridge resolves only trusted matching hook responses', async () => {
  const target = {
    posted: [],
    postMessage(message, targetOrigin) {
      this.posted.push({ message, targetOrigin })
    },
  }
  const hookBridge = createPluginHookBridge(
    { id: 'plugin-a' },
    {
      getTarget: () => target,
      isTrustedSource: (event) => event.source === target,
      targetOrigin: '*',
      timeoutMs: 1000,
    },
  )

  const payload = { prompt: 'base' }
  const promise = hookBridge.emitAndWait('CHAT_COMPLETION_PROMPT_READY', payload)
  const request = target.posted.at(-1).message
  assert.equal(target.posted.at(-1).targetOrigin, '*')
  assert.equal(request.type, MSG_HOOK_REQUEST)
  assert.equal(request.pluginId, 'plugin-a')

  assert.equal(hookBridge.handleMessage({
    source: { postMessage() {} },
    data: {
      type: MSG_HOOK_RESPONSE,
      pluginId: 'plugin-a',
      id: request.id,
      result: { prompt: 'evil' },
    },
  }), false)

  assert.equal(hookBridge.handleMessage({
    source: target,
    data: {
      type: MSG_HOOK_RESPONSE,
      pluginId: 'plugin-a',
      id: request.id,
      result: { prompt: 'trusted' },
    },
  }), true)

  assert.deepEqual(await promise, { prompt: 'trusted' })
})

test('host hook bridge falls back to original payload on plugin hook errors', async () => {
  const errors = []
  const target = {
    posted: [],
    postMessage(message, targetOrigin) {
      this.posted.push({ message, targetOrigin })
    },
  }
  const hookBridge = createPluginHookBridge(
    { id: 'plugin-a' },
    {
      getTarget: () => target,
      isTrustedSource: (event) => event.source === target,
      timeoutMs: 1000,
      onError: (error) => errors.push(error.message),
    },
  )

  const payload = { prompt: 'base' }
  const promise = hookBridge.emitAndWait('CHAT_COMPLETION_PROMPT_READY', payload)
  const request = target.posted.at(-1).message

  hookBridge.handleMessage({
    source: target,
    data: {
      type: MSG_HOOK_RESPONSE,
      pluginId: 'plugin-a',
      id: request.id,
      error: 'hook failed',
    },
  })

  assert.equal(await promise, payload)
  assert.deepEqual(errors, ['hook failed'])
})

test('provides ST slash command registration and trigger fallbacks', () => {
  const { window } = createBridgeSandbox()
  const calls = []

  const registered = window.registerSlashCommand(
    'heal',
    (args, context) => calls.push(['heal', args, context?.source]),
    ['hp'],
  )
  const objectCommand = window.SlashCommand.fromProps({
    name: 'inspect',
    aliases: ['look'],
    callback: (args) => calls.push(['inspect', args]),
    helpString: 'inspect target',
  })

  window.SlashCommandParser.addCommandObject(objectCommand)
  window.triggerSlash('hp', '10', { source: 'test' })
  window.triggerSlashCommand('look', 'door')

  assert.equal(registered.name, 'heal')
  assert.deepEqual(Array.from(window.SlashCommandParser.commands, (command) => command.name), ['heal', 'inspect'])
  assert.deepEqual(Array.from(window.storyforge.slashCommands.list(), (command) => command.helpString || ''), ['', 'inspect target'])
  assert.deepEqual(calls, [
    ['heal', '10', 'test'],
    ['inspect', 'door'],
  ])
})

test('parses slash invocation strings into raw, named, and unnamed args', () => {
  const { window } = createBridgeSandbox()
  const calls = []

  window.registerSlashCommand('heal', (rawArgs, context) => calls.push({
    rawArgs,
    namedArgs: context.namedArgs,
    unnamedArgs: context.unnamedArgs,
    source: context.source,
    input: context.input,
  }), ['hp'])

  window.triggerSlash('/hp target=alice --amount 10 --critical "red zone" loose')

  assert.deepEqual(plain(calls), [{
    rawArgs: 'target=alice --amount 10 --critical "red zone" loose',
    namedArgs: { target: 'alice', amount: '10', critical: 'red zone' },
    unnamedArgs: ['loose'],
    source: 'slash',
    input: '/hp target=alice --amount 10 --critical "red zone" loose',
  }])
})

test('pipes slash command results through pipeline segments', () => {
  const { window } = createBridgeSandbox()
  const calls = []

  window.registerSlashCommand('echo', (rawArgs, context) => {
    calls.push(['echo', rawArgs, context.source, context.pipe ?? null])
    return rawArgs.toUpperCase()
  })
  window.registerSlashCommand('wrap', (rawArgs, context) => {
    calls.push(['wrap', rawArgs, context.source, context.pipe])
    return `[${context.pipe}] ${rawArgs}`
  })

  const result = window.triggerSlash('/echo hi | wrap suffix')

  assert.equal(result, '[HI] suffix')
  assert.deepEqual(calls, [
    ['echo', 'hi', 'slash', null],
    ['wrap', 'suffix', 'slash', 'HI'],
  ])
})

test('does not split slash pipes inside quoted arguments', () => {
  const { window } = createBridgeSandbox()
  const calls = []

  window.registerSlashCommand('echo', (rawArgs) => {
    calls.push(rawArgs)
    return rawArgs
  })

  const result = window.triggerSlash('/echo "left | right"')

  assert.equal(result, '"left | right"')
  assert.deepEqual(calls, ['"left | right"'])
})

test('awaits async slash command results before piping to the next segment', async () => {
  const { window } = createBridgeSandbox()
  const calls = []

  window.registerSlashCommand('fetch', async (rawArgs) => {
    calls.push(['fetch', rawArgs])
    await flushPromises()
    return 'async-value'
  })
  window.registerSlashCommand('use', (rawArgs, context) => {
    calls.push(['use', rawArgs, context.pipe])
    return `${context.pipe}:${rawArgs}`
  })

  const result = await window.triggerSlash('/fetch source | use target')

  assert.equal(result, 'async-value:target')
  assert.deepEqual(calls, [
    ['fetch', 'source'],
    ['use', 'target', 'async-value'],
  ])
})

test('keeps zero-argument slash command triggers unchanged', () => {
  const { window } = createBridgeSandbox()
  const calls = []

  window.registerSlashCommand('noop', function() {
    calls.push(Array.prototype.slice.call(arguments))
  })

  window.triggerSlash('noop')

  assert.deepEqual(calls, [[]])
})

test('provides genraw slash fallback through LLM generation', () => {
  const { window, postedMessages } = createBridgeSandbox('plugin-a', 'https://host.example')

  assert.equal(window.SlashCommandParser.commands.length, 0)
  assert.equal(window.storyforge.slashCommands.list().length, 0)

  window.triggerSlash('/genraw write a short beat')
  assert.equal(postedMessages.at(-1).message.type, MSG_REQUEST)
  assert.equal(postedMessages.at(-1).message.method, 'llm.generate')
  assert.deepEqual(plain(postedMessages.at(-1).message.params), { prompt: 'write a short beat' })

  window.storyforge.llm.generate({ intent: 'object style intent' })
  assert.equal(postedMessages.at(-1).message.method, 'llm.generate')
  assert.deepEqual(plain(postedMessages.at(-1).message.params), { intent: 'object style intent' })
})

test('provides TavernHelper aliases for common ST plugin APIs', async () => {
  const { window, postedMessages } = createBridgeSandbox('plugin-a', 'https://host.example')
  const calls = []

  assert.equal(window.tavernHelper, window.TavernHelper)
  assert.equal(typeof window.TavernHelper.eventOn, 'function')
  assert.equal(typeof window.TavernHelper.registerSlashCommand, 'function')

  window.TavernHelper.eventOn('CHAT_CHANGED', (payload) => calls.push(['event', payload.reason]))
  await window.TavernHelper.eventEmit('CHAT_CHANGED', { reason: 'helper' })

  window.TavernHelper.registerSlashCommand('mark', (rawArgs, context) => calls.push([
    'slash',
    rawArgs,
    context.namedArgs.target,
  ]))
  window.TavernHelper.triggerSlash('/mark target=door note')

  window.TavernHelper.setStatusBar('<b>ready</b>')
  assert.equal(postedMessages.at(-1).message.type, 'sf:ui:mount')
  assert.equal(postedMessages.at(-1).message.slot, 'statusbar')
  assert.equal(postedMessages.at(-1).message.html, '<b>ready</b>')

  window.TavernHelper.storageSet('mode', { value: 'compat' })
  assert.deepEqual(plain(window.TavernHelper.storageGet('mode')), { value: 'compat' })

  const variablePromise = window.TavernHelper.getVariables('campaign-1', 'inst-1')
  assert.equal(postedMessages.at(-1).message.type, MSG_REQUEST)
  assert.equal(postedMessages.at(-1).message.method, 'variables.get')
  assert.deepEqual(plain(postedMessages.at(-1).message.params), {
    campaignId: 'campaign-1',
    instanceId: 'inst-1',
  })

  assert.deepEqual(calls, [
    ['event', 'helper'],
    ['slash', 'target=door note', 'door'],
  ])
  assert.equal(typeof variablePromise.then, 'function')
})

test('provides SillyTavern globals and chat message helpers for ST compatibility', async () => {
  const { window, postedMessages } = createBridgeSandbox()
  const readyMessageCount = postedMessages.length

  assert.equal(typeof window.SillyTavern, 'object')
  assert.equal(window.SillyTavern.chat, window.SillyTavern.getContext().chat)
  assert.equal(window.getChatMessages, window.TavernHelper.getChatMessages)
  assert.equal(window.setChatMessages, window.TavernHelper.setChatMessages)
  assert.equal(window.getLastMessageId, window.TavernHelper.getLastMessageId)
  assert.equal(window.setChatMessage, window.TavernHelper.setChatMessage)
  assert.equal(typeof window.SillyTavern.saveChat, 'function')
  assert.equal(typeof window.SillyTavern.callGenericPopup, 'function')
  assert.equal(typeof window.SillyTavern.getRequestHeaders, 'function')
  assert.equal(typeof window.SillyTavern.ToolManager.registerTool, 'function')
  assert.equal(typeof window.registerMacro, 'function')
  assert.equal(typeof window.unregisterMacro, 'function')
  assert.equal(window.SillyTavern.getContext().eventSource, window.eventSource)
  assert.equal(window.SillyTavern.getContext().event_types, window.event_types)
  assert.equal(window.SillyTavern.getContext().eventTypes, window.eventTypes)
  assert.equal(window.SillyTavern.getContext().TavernHelper, window.TavernHelper)
  assert.equal(window.SillyTavern.getContext().saveSettingsDebounced, window.saveSettingsDebounced)
  assert.deepEqual(plain(window.SillyTavern.getContext().chat_metadata), {})
  assert.equal(typeof window.SillyTavern.getContext().getCurrentChatId, 'function')

  window.SillyTavern.chat.push({ name: 'User', mes: 'hello' })
  window.SillyTavern.chat.push({ name: 'Assistant', message: 'old reply' })

  assert.equal(window.getLastMessageId(), 1)
  assert.equal(window.getChatMessages('0').length, 1)
  assert.equal(window.getChatMessages('01').length, 2)
  assert.equal(window.getChatMessages('not-a-number').length, 2)
  assert.deepEqual(plain(window.getChatMessages(1)), [
    { name: 'User', mes: 'hello', message_id: 0, message: 'hello' },
    { name: 'Assistant', message: 'old reply', message_id: 1, mes: 'old reply' },
  ])

  assert.equal(await window.setChatMessages([{ message_id: 1, message: 'new reply' }]), true)
  assert.equal(window.SillyTavern.chat[1].mes, 'new reply')
  assert.equal(await window.setChatMessage({ variables: { hp: 5 } }, 1), true)
  assert.deepEqual(plain(window.SillyTavern.chat[1].variables), { hp: 5 })
  assert.equal(await window.SillyTavern.saveChat(), true)
  assert.equal(await window.SillyTavern.callGenericPopup('prompt', window.SillyTavern.POPUP_TYPE.INPUT, '10'), '10')
  assert.deepEqual(plain(window.SillyTavern.getRequestHeaders()), { 'Content-Type': 'application/json' })

  const macro = () => 'macro-value'
  window.registerMacro('mvu', macro)
  assert.equal(window.SillyTavern.getContext().chat.length, 2)

  const tool = { name: 'mvu-tool', call: () => 'ok' }
  assert.equal(window.SillyTavern.ToolManager.registerTool(tool), tool)
  assert.equal(window.SillyTavern.ToolManager.getTool('mvu-tool'), tool)
  window.SillyTavern.ToolManager.unregisterTool('mvu-tool')
  assert.equal(window.SillyTavern.ToolManager.getTool('mvu-tool'), undefined)
  assert.equal(postedMessages.length, readyMessageCount)
})

test('chains TavernHelper eventEmitAndWait returned payloads in listener order', async () => {
  const { window } = createBridgeSandbox()
  const calls = []
  const payload = { prompt: 'base', steps: [] }

  window.TavernHelper.eventOn('CHAT_COMPLETION_PROMPT_READY', async (eventPayload) => {
    calls.push(['first', eventPayload.prompt])
    await Promise.resolve()
    return {
      ...eventPayload,
      prompt: `${eventPayload.prompt} + first`,
      steps: [...eventPayload.steps, 'first'],
    }
  })
  window.TavernHelper.eventOn('CHAT_COMPLETION_PROMPT_READY', (eventPayload) => {
    calls.push(['second', eventPayload.prompt])
    return {
      ...eventPayload,
      prompt: `${eventPayload.prompt} + second`,
      steps: [...eventPayload.steps, 'second'],
    }
  })
  window.TavernHelper.eventOn('CHAT_COMPLETION_PROMPT_READY', (eventPayload) => {
    calls.push(['third', eventPayload.prompt])
    eventPayload.steps.push('third')
  })

  const result = await window.TavernHelper.eventEmitAndWait('CHAT_COMPLETION_PROMPT_READY', payload)

  assert.deepEqual(calls, [
    ['first', 'base'],
    ['second', 'base + first'],
    ['third', 'base + first + second'],
  ])
  assert.deepEqual(plain(result), {
    prompt: 'base + first + second',
    steps: ['first', 'second', 'third'],
  })
  assert.deepEqual(payload, { prompt: 'base', steps: [] })
})

test('supports TavernHelper prompt hook aliases for host hook requests', async () => {
  const { window, postedMessages, postHostMessage } = createBridgeSandbox('plugin-a', 'https://host.example')

  assert.equal(typeof window.TavernHelper.onGenerateBeforeCombinePrompts, 'function')
  assert.equal(typeof window.TavernHelper.onChatCompletionPromptReady, 'function')
  assert.equal(typeof window.TavernHelper.promptHooks.onChatCompletionPromptReady, 'function')

  window.TavernHelper.onGenerateBeforeCombinePrompts((eventPayload) => ({
    ...eventPayload,
    prompt: `${eventPayload.prompt} + before`,
  }))
  window.TavernHelper.promptHooks.onChatCompletionPromptReady(async (eventPayload) => {
    await Promise.resolve()
    return {
      ...eventPayload,
      messages: [
        ...eventPayload.messages,
        { role: 'system', content: 'helper alias' },
      ],
    }
  })

  assert.deepEqual(
    plain(await window.TavernHelper.eventEmitAndWait('GENERATE_BEFORE_COMBINE_PROMPTS', { prompt: 'base' })),
    { prompt: 'base + before' },
  )

  postHostMessage({
    type: MSG_HOOK_REQUEST,
    pluginId: 'plugin-a',
    id: 'helper-hook-alias',
    event: 'CHAT_COMPLETION_PROMPT_READY',
    data: { messages: [{ role: 'user', content: 'hello' }] },
  })
  await flushPromises()

  const response = postedMessages.at(-1)
  assert.equal(response.targetOrigin, 'https://host.example')
  assert.equal(response.message.type, MSG_HOOK_RESPONSE)
  assert.equal(response.message.id, 'helper-hook-alias')
  assert.deepEqual(plain(response.message.result), {
    messages: [
      { role: 'user', content: 'hello' },
      { role: 'system', content: 'helper alias' },
    ],
  })
})

test('supports selector-based TavernHelper variable helpers without backend calls', () => {
  const { window, postedMessages } = createBridgeSandbox('plugin-a', 'https://host.example')
  const selector = { type: 'message', message_id: 'latest' }

  assert.equal(window.getVariables, window.TavernHelper.getVariables)
  assert.equal(window.insertOrAssignVariables, window.TavernHelper.insertOrAssignVariables)

  assert.deepEqual(plain(window.getVariables(selector)), {})
  window.insertOrAssignVariables(selector, { hp: 10 })
  window.updateVariablesWith({ message_id: 'latest', type: 'message' }, (vars) => ({
    hp: vars.hp + 5,
    mp: 3,
  }))
  assert.deepEqual(plain(window.TavernHelper.getVariables(selector)), { hp: 15, mp: 3 })

  window.replaceVariables(selector, { ready: true })
  assert.deepEqual(plain(window.TavernHelper.getVariables(selector)), { ready: true })

  window.setVariable(selector, 'phase', 'intro')
  assert.equal(window.getVariable(selector, 'phase'), 'intro')
  assert.equal(postedMessages.at(-1).message.type, 'sf:ready')
})

test('supports TavernHelper variable selector helpers with ST argument order', () => {
  const { window, postedMessages } = createBridgeSandbox('plugin-a', 'https://host.example')
  const selector = { type: 'message', message_id: 7 }

  window.TavernHelper.setVariables({ type: 'state', hp: 10 }, selector)
  assert.deepEqual(plain(window.TavernHelper.getVariables(selector)), { type: 'state', hp: 10 })

  window.insertOrAssignVariables({ type: 'patch', mp: 4 }, selector)
  assert.deepEqual(plain(window.getVariables(selector)), { type: 'patch', hp: 10, mp: 4 })

  window.updateVariablesWith((vars) => ({
    hp: vars.hp + 5,
    mp: vars.mp,
    type: vars.type,
    ready: true,
  }), selector)
  assert.deepEqual(plain(window.TavernHelper.getVariables(selector)), { type: 'patch', hp: 15, mp: 4, ready: true })

  window.replaceVariables({ type: 'done', done: true }, selector)
  assert.deepEqual(plain(window.TavernHelper.getVariables(selector)), { type: 'done', done: true })
  window.TavernHelper.setVariables(selector, { type: 'local', hp: 2 })
  assert.deepEqual(plain(window.TavernHelper.getVariables(selector)), { type: 'local', hp: 2 })
  window.insertOrAssignVariables(selector, { type: 'global', mp: 1 })
  assert.deepEqual(plain(window.getVariables(selector)), { type: 'global', hp: 2, mp: 1 })
  window.replaceVariables(selector, { type: 'message', done: true })
  assert.deepEqual(plain(window.TavernHelper.getVariables(selector)), { type: 'message', done: true })
  window.TavernHelper.setVariables(selector, { type: 'local' })
  assert.deepEqual(plain(window.TavernHelper.getVariables(selector)), { type: 'local' })
  assert.equal(postedMessages.at(-1).message.type, 'sf:ready')
})

test('normalizes slash placement and status bar mount fallbacks', () => {
  const { window, postedMessages } = createBridgeSandbox('plugin-a', 'https://host.example')

  window.storyforge.ui.mountToSlot('slash_command', '<button>/heal</button>')
  assert.equal(postedMessages.at(-1).message.type, 'sf:ui:mount')
  assert.equal(postedMessages.at(-1).message.slot, 'slash')
  assert.equal(postedMessages.at(-1).targetOrigin, 'https://host.example')

  window.storyforge.ui.mountToSlot('status_bar', '<div class="sf-status-bar">HP 5</div>')
  assert.equal(postedMessages.at(-1).message.slot, 'statusbar')
  assert.equal(postedMessages.at(-1).message.html, '<div class="sf-status-bar">HP 5</div>')

  window.storyforge.statusBar.clear()
  assert.equal(postedMessages.at(-1).message.slot, 'statusbar')
  assert.equal(postedMessages.at(-1).message.html, '')

  assert.equal(Object.keys(window.extension_settings.storyforge.statusBar).length, 0)
  assert.equal(
    window.extension_settings.storyforge.status_bar,
    window.extension_settings.storyforge.statusBar,
  )
})
