import test from 'node:test'
import assert from 'node:assert/strict'
import vm from 'node:vm'
import {
  generateBridgeScript,
  mapPipelineEventToPluginEvents,
  mapPluginEventRecordToPluginEvents,
  ST_EVENT_TYPES,
} from '../src/plugin-bridge.js'

function createBridgeSandbox(pluginId = 'plugin-a') {
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
      postMessage: (message) => postedMessages.push(message),
    },
    localStorage: window.localStorage,
    console,
  }
  sandbox.globalThis = sandbox

  const script = generateBridgeScript(pluginId)
    .replace(/^<script>\n?/, '')
    .replace(/\n?<\/script>$/, '')
  vm.runInNewContext(script, sandbox)

  return { window, listeners, postedMessages }
}

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
  const { window, listeners } = createBridgeSandbox()
  const calls = []

  window.storyforge.events.on('GENERATION_STARTED', (payload) => calls.push(['storyforge', payload.session_id]))
  window.eventSource.on(window.event_types.GENERATION_STARTED, (payload) => calls.push(['st', payload.session_id]))

  listeners.message({
    data: {
      type: 'sf:api:event',
      event: 'GENERATION_STARTED',
      data: { session_id: 's1' },
    },
  })

  assert.deepEqual(calls, [
    ['storyforge', 's1'],
    ['st', 's1'],
  ])
})

test('supports ST once, makeFirst, makeLast, removeListener, and local emit', () => {
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

  window.eventSource.emit('CHAT_CHANGED', { id: 1 })
  window.eventSource.emit('CHAT_CHANGED', { id: 2 })

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
