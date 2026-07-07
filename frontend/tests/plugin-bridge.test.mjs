import test from 'node:test'
import assert from 'node:assert/strict'
import vm from 'node:vm'
import {
  createHostHandler,
  createPluginHookBridge,
  generateBridgeScript,
  mapPipelineEventToPluginEvents,
  mapPluginEventRecordToPluginEvents,
  MSG_HOOK_REQUEST,
  MSG_HOOK_RESPONSE,
  MSG_REQUEST,
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
  assert.equal(result, undefined)
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

test('keeps zero-argument slash command triggers unchanged', () => {
  const { window } = createBridgeSandbox()
  const calls = []

  window.registerSlashCommand('noop', function() {
    calls.push(Array.prototype.slice.call(arguments))
  })

  window.triggerSlash('noop')

  assert.deepEqual(calls, [[]])
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
