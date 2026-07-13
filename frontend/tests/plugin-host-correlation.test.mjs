import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'

import {
  mapPluginEventRecordToPluginEvents,
  MSG_EVENT,
} from '../src/plugin-bridge.js'

function loadPluginHostHelpers() {
  const source = fs.readFileSync(new URL('../src/components/PluginHost.vue', import.meta.url), 'utf8')
  const match = source.match(/<script>([\s\S]*?)<\/script>/)
  assert.ok(match, 'PluginHost.vue should expose testable helpers')

  const moduleCode = match[1]
    .replaceAll('export const ', 'const ')
    .replaceAll('export function ', 'function ')

  const sandbox = { setTimeout, clearTimeout }
  vm.runInNewContext(`${moduleCode}
Object.assign(globalThis, {
  applyPluginSlotMount,
  getPluginSlotEntries,
  normalizePluginHostSlot,
  waitForPluginHostReady,
  PLUGIN_HOST_DEFAULT_SLOT,
  PLUGIN_HOST_HOOK_READY_TIMEOUT_MS,
})`, sandbox)

  return sandbox
}

/**
 * Deterministic stand-in for PluginHost event consumption without a real iframe.
 * Mirrors consumePluginEvents/postPluginEvent correlation rules.
 */
function createMockPluginHost(plugin) {
  let lastPluginEventId = 0
  const pendingPluginEvents = []
  const delivered = []
  let iframeReady = false
  const MAX_PENDING_PLUGIN_EVENTS = 100

  function queuePluginEvent(pluginEvent) {
    pendingPluginEvents.push(pluginEvent)
    if (pendingPluginEvents.length > MAX_PENDING_PLUGIN_EVENTS) {
      pendingPluginEvents.shift()
    }
  }

  function postPluginEvent(pluginEvent) {
    if (!iframeReady) {
      queuePluginEvent(pluginEvent)
      return
    }
    delivered.push({
      type: MSG_EVENT,
      event: pluginEvent.event,
      data: pluginEvent.data,
    })
  }

  function dispatchPluginEventRecord(record) {
    for (const pluginEvent of mapPluginEventRecordToPluginEvents(record, plugin)) {
      postPluginEvent(pluginEvent)
    }
  }

  function consumePluginEvents(events) {
    for (const record of events || []) {
      const eventId = Number(record?.id || 0)
      if (eventId > 0 && eventId <= lastPluginEventId) continue
      dispatchPluginEventRecord(record)
      if (eventId > lastPluginEventId) {
        lastPluginEventId = eventId
      }
    }
  }

  function markReady() {
    iframeReady = true
    while (pendingPluginEvents.length > 0) {
      const pluginEvent = pendingPluginEvents.shift()
      delivered.push({
        type: MSG_EVENT,
        event: pluginEvent.event,
        data: pluginEvent.data,
      })
    }
  }

  function unmount() {
    iframeReady = false
    pendingPluginEvents.length = 0
  }

  return {
    consumePluginEvents,
    markReady,
    unmount,
    get delivered() {
      return delivered.slice()
    },
    get pendingCount() {
      return pendingPluginEvents.length
    },
    get lastId() {
      return lastPluginEventId
    },
  }
}

const {
  applyPluginSlotMount,
  getPluginSlotEntries,
  waitForPluginHostReady,
  PLUGIN_HOST_HOOK_READY_TIMEOUT_MS,
} = loadPluginHostHelpers()

test('mock UI PluginHost correlates event ids and drops replays (labelled mock UI)', () => {
  const plugin = {
    id: 'mock-plugin',
    permissions: ['ReadMemory'],
    event_subscriptions: ['*'],
  }
  const host = createMockPluginHost(plugin)
  host.markReady()

  host.consumePluginEvents([
    { id: 1, event: 'MESSAGE_SENT', data: { messageId: 'm1', content: 'hi', role: 'user' } },
    { id: 1, event: 'MESSAGE_SENT', data: { messageId: 'm1', content: 'replay', role: 'user' } },
    { id: 2, event: 'MESSAGE_RECEIVED', data: { messageId: 'm2', content: 'yo', role: 'assistant' } },
  ])

  assert.equal(host.lastId, 2)
  const names = host.delivered.map((item) => item.event)
  assert.ok(names.includes('MESSAGE_SENT'))
  assert.ok(names.includes('USER_MESSAGE_RENDERED'))
  assert.ok(names.includes('MESSAGE_RECEIVED'))
  assert.equal(names.filter((name) => name === 'MESSAGE_SENT').length, 1)
})

test('mock UI PluginHost queues events until mount ready then flushes', () => {
  const plugin = {
    id: 'mock-plugin',
    permissions: ['ReadMemory'],
    event_subscriptions: ['MESSAGE_SENT', 'CHAT_CHANGED', 'USER_MESSAGE_RENDERED'],
  }
  const host = createMockPluginHost(plugin)

  host.consumePluginEvents([
    { id: 5, event: 'MESSAGE_SENT', data: { messageId: 'm1', content: 'queued', role: 'user' } },
  ])
  assert.equal(host.delivered.length, 0)
  assert.ok(host.pendingCount > 0)

  host.markReady()
  assert.ok(host.delivered.length > 0)
  assert.equal(host.pendingCount, 0)
  assert.equal(host.delivered[0].type, MSG_EVENT)
  assert.equal(host.delivered[0].data.messageId, 'm1')
})

test('mock UI PluginHost unmount drops pending delivery without throwing', () => {
  const plugin = {
    id: 'mock-plugin',
    permissions: [],
    event_subscriptions: ['MESSAGE_RECEIVED'],
  }
  const host = createMockPluginHost(plugin)
  host.consumePluginEvents([
    { id: 9, event: 'MESSAGE_RECEIVED', data: { messageId: 'm9', content: 'secret', role: 'assistant' } },
  ])
  assert.ok(host.pendingCount > 0)
  host.unmount()
  assert.equal(host.pendingCount, 0)
  assert.equal(host.delivered.length, 0)
})

test('status slots stay isolated across mount clear cycles', () => {
  let slots = {}
  slots = applyPluginSlotMount(slots, { slot: 'statusbar', html: '<span>A</span>' })
  slots = applyPluginSlotMount(slots, { slot: 'slash', html: '<button>/a</button>' })
  slots = applyPluginSlotMount(slots, { slot: 'statusbar', html: '' })

  assert.deepEqual(
    JSON.parse(JSON.stringify(getPluginSlotEntries(slots))),
    [{ slot: 'slash', html: '<button>/a</button>' }],
  )
})

test('hidden hook host readiness wait times out without deadlock', async () => {
  const result = await waitForPluginHostReady(
    () => false,
    () => () => {},
    Math.min(20, PLUGIN_HOST_HOOK_READY_TIMEOUT_MS),
  )
  assert.equal(result, false)
})
