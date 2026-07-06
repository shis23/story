import test from 'node:test'
import assert from 'node:assert/strict'
import { mapPipelineEventToPluginEvents } from '../src/plugin-bridge.js'

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
