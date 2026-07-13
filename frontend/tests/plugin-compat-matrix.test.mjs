import assert from 'node:assert/strict'
import test from 'node:test'
import vm from 'node:vm'

import {
  createPluginHookBridge,
  generateBridgeScript,
  mapPipelineEventToPluginEvents,
  mapPluginEventRecordToPluginEvents,
  MSG_HOOK_REQUEST,
  MSG_HOOK_RESPONSE,
  ST_EVENT_TYPES,
} from '../src/plugin-bridge.js'
import {
  assertMatrixCoversStEventTypes,
  auditRecordLooksSafe,
  classifyDegradedHelperResult,
  classifySlashOutcome,
  entriesRequiringExplicitBehavior,
  findMatrixEntry,
  listMatrixBySurface,
  PLUGIN_COMPAT_MATRIX,
  SLASH_COMMAND_MATRIX,
  ST_EVENT_MATRIX,
  SUPPORT,
  supportedEventEntries,
  TAVERN_HELPER_MATRIX,
} from '../src/utils/pluginCompatMatrix.js'
import {
  appendPromptHookAuditRecord,
  emitPromptHookEventAndWaitForPlugins,
} from '../src/utils/promptHooks.js'
import {
  exportPromptHookAudit,
  parsePromptHookAuditExport,
} from '../src/utils/promptHookAudit.js'

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

  return { window, listeners, postedMessages }
}

function plain(value) {
  return JSON.parse(JSON.stringify(value))
}

test('matrix covers every ST_EVENT_TYPES constant exactly once', () => {
  const coverage = assertMatrixCoversStEventTypes(ST_EVENT_TYPES)
  assert.equal(coverage.ok, true, `missing matrix rows: ${coverage.missing.join(', ')}`)

  const names = ST_EVENT_MATRIX.map((entry) => entry.name)
  assert.equal(new Set(names).size, names.length)
  assert.ok(PLUGIN_COMPAT_MATRIX.length >= ST_EVENT_MATRIX.length)
  assert.ok(listMatrixBySurface('slash').length === SLASH_COMMAND_MATRIX.length)
})

test('every matrix entry has stable id, surface, status vocabulary', () => {
  const statuses = new Set(Object.values(SUPPORT))
  const ids = new Set()
  for (const entry of PLUGIN_COMPAT_MATRIX) {
    assert.ok(entry.id, 'entry needs id')
    assert.ok(!ids.has(entry.id), `duplicate id ${entry.id}`)
    ids.add(entry.id)
    assert.ok(entry.surface)
    assert.ok(statuses.has(entry.status), `unknown status ${entry.status} on ${entry.id}`)
    assert.ok(entry.name)
  }
})

test('supported ST events map to at least one plugin-visible event path', () => {
  const plugin = {
    id: 'matrix-reader',
    permissions: ['ReadMemory'],
    event_subscriptions: ['*'],
  }

  for (const entry of supportedEventEntries()) {
    if (entry.status === SUPPORT.ALIAS && entry.mapsFrom?.startsWith('pipeline.')) {
      const pipelineType = entry.mapsFrom.split('→')[0].replace('pipeline.', '')
      const mapped = mapPipelineEventToPluginEvents({
        event_type: pipelineType,
        data: { delta: 'x', text: 'y', message: 'z' },
      }, plugin)
      assert.ok(
        mapped.some((item) => item.event === entry.name || item.event === pipelineType || item.event.startsWith('pipeline.')),
        `${entry.name} should be reachable from ${pipelineType}`,
      )
      continue
    }

    if (entry.status === SUPPORT.DERIVED) {
      const source = entry.mapsFrom?.includes('MESSAGE_SENT')
        ? ST_EVENT_TYPES.MESSAGE_SENT
        : entry.mapsFrom?.includes('MESSAGE_RECEIVED')
          ? ST_EVENT_TYPES.MESSAGE_RECEIVED
          : ST_EVENT_TYPES.CHAT_LOADED
      const mapped = mapPluginEventRecordToPluginEvents({
        event: source,
        data: { role: 'assistant', content: 'body' },
      }, plugin)
      assert.ok(mapped.some((item) => item.event === entry.name || item.event === source), entry.name)
      continue
    }

    const mapped = mapPluginEventRecordToPluginEvents({
      event: entry.name,
      data: { content: 'body' },
    }, plugin)
    assert.ok(mapped.some((item) => item.event === entry.name), `${entry.name} should pass through when subscribed`)
  }
})

test('unsupported and noop ST events stay non-emitting under generic host mapping', () => {
  const plugin = {
    id: 'matrix-reader',
    permissions: ['ReadMemory'],
    event_subscriptions: ['*'],
  }
  const blocked = ST_EVENT_MATRIX.filter((entry) => (
    entry.status === SUPPORT.INTENTIONALLY_UNSUPPORTED || entry.status === SUPPORT.NOOP
  ))

  for (const entry of blocked) {
    // Constants exist, but host pipeline mapping must not invent aliases for them.
    const pipelineMapped = mapPipelineEventToPluginEvents({
      event_type: entry.name.toLowerCase(),
      data: { content: 'secret' },
    }, plugin)
    assert.equal(
      pipelineMapped.some((item) => item.event === entry.name),
      false,
      `${entry.name} must not appear as a pipeline alias`,
    )
  }

  const filtered = mapPipelineEventToPluginEvents({
    event_type: 'prompt_hook_request',
    data: { messages: [{ role: 'user', content: 'secret' }] },
  }, plugin)
  assert.deepEqual(filtered, [])
})

test('unknown slash commands fail visibly instead of silent undefined', () => {
  const { window } = createBridgeSandbox()
  let error = null
  let result
  try {
    result = window.triggerSlashCommand('definitely-not-registered-xyz')
  } catch (err) {
    error = err
  }

  const classified = classifySlashOutcome(result, error)
  assert.equal(
    classified.visibleFailure,
    true,
    `unknown slash must fail visibly, got ${JSON.stringify({ result, error: error?.message, classified })}`,
  )
  assert.equal(classified.ok, false)
  assert.match(classified.message, /unsupported|unknown|not (found|registered)/i)
})

test('slash pipe errors surface when a segment is unknown', async () => {
  const { window } = createBridgeSandbox()
  window.registerSlashCommand('echo', (rawArgs) => String(rawArgs || '').toUpperCase())

  let error = null
  try {
    await window.triggerSlash('/echo hi | missing-cmd next')
  } catch (err) {
    error = err
  }
  assert.ok(error, 'pipe with unknown segment should reject/throw')
  assert.match(String(error.message || error), /unsupported|unknown|not (found|registered)/i)
})

test('degraded TavernHelper helpers expose visible degradation markers', async () => {
  const { window } = createBridgeSandbox()
  const saveResult = await window.TavernHelper.saveChat()
  const popupResult = await window.TavernHelper.callGenericPopup('confirm?', window.SillyTavern.POPUP_TYPE.CONFIRM)
  const headers = window.TavernHelper.getRequestHeaders()

  assert.equal(classifyDegradedHelperResult('saveChat', saveResult).visible, true)
  assert.equal(saveResult.degraded, true)
  assert.ok(saveResult.reason)
  assert.match(saveResult.reason, /local_mirror|no_host_persist/i)

  // CONFIRM has no UI: ST-compatible null is the visible degraded outcome.
  assert.equal(popupResult, null)
  assert.equal(findMatrixEntry('th:callGenericPopup').status, SUPPORT.DEGRADED)

  // static headers are intentionally incomplete (no auth); matrix marks degraded
  assert.deepEqual(plain(headers), { 'Content-Type': 'application/json' })
  assert.equal(findMatrixEntry('th:getRequestHeaders').status, SUPPORT.DEGRADED)
  assert.equal(findMatrixEntry('th:saveChat').status, SUPPORT.DEGRADED)
  assert.ok(TAVERN_HELPER_MATRIX.some((entry) => entry.status === SUPPORT.INTENTIONALLY_UNSUPPORTED))
})

test('prompt hook timeout is audited as timeout and remains fail-open', async () => {
  const audits = []
  const plugins = [
    { id: 'slow', permissions: ['ModifyPrompt'], manifest: { name: 'Slow Hook' } },
    { id: 'fast', permissions: ['ModifyPrompt'], manifest: { name: 'Fast Hook' } },
  ]
  const hostRefs = new Map([
    ['slow', {
      async emitPluginEventAndWait() {
        await new Promise((resolve) => setTimeout(resolve, 40))
        return { prompt: 'should-not-apply-after-timeout' }
      },
    }],
    ['fast', {
      async emitPluginEventAndWait(event, payload) {
        return { ...payload, prompt: `${payload.prompt} + fast` }
      },
    }],
  ])

  const result = await emitPromptHookEventAndWaitForPlugins(
    plugins,
    hostRefs,
    'CHAT_COMPLETION_PROMPT_READY',
    { prompt: 'base' },
    {
      stage: 'matrix_timeout',
      timeoutMs: 5,
      onAudit: (record) => audits.push(record),
    },
  )

  assert.deepEqual(result, { prompt: 'base + fast' })
  assert.equal(audits[0].pluginId, 'slow')
  assert.equal(audits[0].status, 'timeout')
  assert.equal(audits[1].pluginId, 'fast')
  assert.equal(audits[1].status, 'ok')
  assert.equal(JSON.stringify(audits).includes('should-not-apply-after-timeout'), false)
})

test('prompt hook cancellation stops later plugins without deadlocking', async () => {
  const audits = []
  const calls = []
  const controller = { cancelled: false }
  const plugins = [
    { id: 'first', permissions: ['ModifyPrompt'] },
    { id: 'second', permissions: ['ModifyPrompt'] },
    { id: 'third', permissions: ['ModifyPrompt'] },
  ]
  const hostRefs = new Map([
    ['first', {
      async emitPluginEventAndWait(event, payload) {
        calls.push('first')
        controller.cancelled = true
        return { ...payload, prompt: `${payload.prompt} + first` }
      },
    }],
    ['second', {
      async emitPluginEventAndWait(event, payload) {
        calls.push('second')
        return { ...payload, prompt: `${payload.prompt} + second` }
      },
    }],
    ['third', {
      async emitPluginEventAndWait(event, payload) {
        calls.push('third')
        return { ...payload, prompt: `${payload.prompt} + third` }
      },
    }],
  ])

  const result = await emitPromptHookEventAndWaitForPlugins(
    plugins,
    hostRefs,
    'CHAT_COMPLETION_PROMPT_READY',
    { prompt: 'base' },
    {
      stage: 'matrix_cancel',
      isCancelled: () => controller.cancelled,
      onAudit: (record) => audits.push(record),
    },
  )

  assert.deepEqual(result, { prompt: 'base + first' })
  assert.deepEqual(calls, ['first'])
  assert.deepEqual(audits.map((record) => [record.pluginId, record.status]), [
    ['first', 'ok'],
    ['second', 'cancelled'],
    ['third', 'cancelled'],
  ])
})

test('late duplicate hook response ids are ignored after settle', async () => {
  const target = {
    posted: [],
    postMessage(message) {
      this.posted.push(message)
    },
  }
  const errors = []
  const hookBridge = createPluginHookBridge(
    { id: 'plugin-a' },
    {
      getTarget: () => target,
      isTrustedSource: (event) => event.source === target,
      timeoutMs: 30,
      onError: (error) => errors.push(String(error?.message || error)),
    },
  )

  const payload = { prompt: 'base' }
  const promise = hookBridge.emitAndWait('CHAT_COMPLETION_PROMPT_READY', payload)
  const request = target.posted.at(-1)
  assert.equal(request.type, MSG_HOOK_REQUEST)

  // First settle via timeout path.
  const timedOut = await promise
  assert.equal(timedOut, payload)
  assert.ok(errors.some((message) => /timed out/i.test(message)))

  // Late duplicate response must not throw or re-resolve.
  assert.equal(hookBridge.handleMessage({
    source: target,
    data: {
      type: MSG_HOOK_RESPONSE,
      pluginId: 'plugin-a',
      id: request.id,
      result: { prompt: 'late-duplicate' },
    },
  }), false)
})

test('audit export proves identity/timing/outcome without secrets or prompt bodies', async () => {
  const audits = []
  const plugins = [
    { id: 'hook-a', permissions: ['ModifyPrompt'], manifest: { name: 'Hook A' } },
  ]
  const hostRefs = new Map([
    ['hook-a', {
      async emitPluginEventAndWait(event, payload) {
        return {
          ...payload,
          prompt: `${payload.prompt} api_key=SF_SECRET_should_not_land_in_audit`,
          messages: [{ role: 'user', content: 'private memory line' }],
        }
      },
    }],
  ])

  await emitPromptHookEventAndWaitForPlugins(
    plugins,
    hostRefs,
    ST_EVENT_TYPES.CHAT_COMPLETION_PROMPT_READY,
    {
      prompt: 'private prompt body',
      messages: [{ role: 'user', content: 'private message body' }],
      api_key: 'SF_SECRET_abc',
    },
    {
      stage: 'matrix_audit',
      onAudit: (record) => audits.push(record),
    },
  )

  const ring = appendPromptHookAuditRecord([], audits[0], 100)
  const exported = exportPromptHookAudit(ring)
  const parsed = parsePromptHookAuditExport(exported)

  assert.equal(parsed.records[0].pluginId, 'hook-a')
  assert.equal(parsed.records[0].pluginName, 'Hook A')
  assert.equal(parsed.records[0].event, ST_EVENT_TYPES.CHAT_COMPLETION_PROMPT_READY)
  assert.equal(parsed.records[0].stage, 'matrix_audit')
  assert.equal(typeof parsed.records[0].durationMs, 'number')
  assert.equal(parsed.records[0].status, 'ok')
  assert.ok(Array.isArray(parsed.records[0].changedKeys))

  const safety = auditRecordLooksSafe(parsed.records[0])
  assert.equal(safety.ok, true, `unsafe audit tokens: ${safety.hits.join(', ')}`)
  assert.equal(exported.includes('private prompt body'), false)
  assert.equal(exported.includes('private message body'), false)
  assert.equal(exported.includes('SF_SECRET_'), false)
  assert.equal(exported.includes('private memory line'), false)
})

test('event subscriptions redact body fields without ReadMemory', () => {
  const withMemory = {
    id: 'mem',
    permissions: ['ReadMemory'],
    event_subscriptions: ['MESSAGE_RECEIVED'],
  }
  const withoutMemory = {
    id: 'no-mem',
    permissions: [],
    event_subscriptions: ['MESSAGE_RECEIVED'],
  }
  const record = {
    event: ST_EVENT_TYPES.MESSAGE_RECEIVED,
    data: {
      messageId: 'm1',
      content: 'private body',
      displayContent: 'private display',
      role: 'assistant',
    },
  }

  const allowed = mapPluginEventRecordToPluginEvents(record, withMemory)
  const redacted = mapPluginEventRecordToPluginEvents(record, withoutMemory)

  assert.equal(allowed[0].data.content, 'private body')
  assert.equal(redacted[0].data.content, undefined)
  assert.equal(redacted[0].data.displayContent, undefined)
  assert.equal(redacted[0].data.role, 'assistant')
  assert.equal(findMatrixEntry('perm:read_memory_redaction').redactsBodyWithoutReadMemory, true)
})

test('matrix lists explicit behavior rows for every degraded/unsupported/noop surface', () => {
  const explicit = entriesRequiringExplicitBehavior()
  assert.ok(explicit.length >= 10)
  for (const entry of explicit) {
    assert.ok(entry.reason || entry.status === SUPPORT.DEGRADED, `${entry.id} needs reason`)
  }
})
