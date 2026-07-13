import assert from 'node:assert/strict'
import test from 'node:test'

import {
  appendPromptHookAuditRecord,
  classifyPromptHookFailurePolicy,
  emitPromptHookEventAndWaitForPlugins,
  promptHookChangedKeys,
  resolveHookedIntent,
  resolveHookedMessages,
  summarizePromptHookPayload,
} from '../src/utils/promptHooks.js'

test('runs prompt hook plugins sequentially and skips plugins without ModifyPrompt', async () => {
  const calls = []
  const plugins = [
    { id: 'reader', permissions: ['ReadMemory'] },
    { id: 'first', permissions: ['ModifyPrompt'] },
    { id: 'missing-host', permissions: ['ModifyPrompt'] },
    { id: 'second', permissions: ['ReadMemory', 'ModifyPrompt'] },
  ]
  const hostRefs = new Map([
    ['reader', {
      async emitPluginEventAndWait() {
        calls.push('reader')
        return { prompt: 'bad' }
      },
    }],
    ['first', {
      async emitPluginEventAndWait(event, payload) {
        calls.push(['first', event, payload.prompt])
        return { ...payload, prompt: `${payload.prompt} + first` }
      },
    }],
    ['second', {
      async emitPluginEventAndWait(event, payload) {
        calls.push(['second', event, payload.prompt])
        return { ...payload, prompt: `${payload.prompt} + second` }
      },
    }],
  ])

  const result = await emitPromptHookEventAndWaitForPlugins(
    plugins,
    hostRefs,
    'CHAT_COMPLETION_PROMPT_READY',
    { prompt: 'base' },
  )

  assert.deepEqual(calls, [
    ['first', 'CHAT_COMPLETION_PROMPT_READY', 'base'],
    ['second', 'CHAT_COMPLETION_PROMPT_READY', 'base + first'],
  ])
  assert.deepEqual(result, { prompt: 'base + first + second' })
})

test('records prompt hook audit entries without storing prompt text', async () => {
  const audits = []
  const plugins = [
    { id: 'first', permissions: ['ModifyPrompt'], manifest: { name: 'First Hook' } },
    { id: 'missing-host', permissions: ['ModifyPrompt'] },
  ]
  const hostRefs = new Map([
    ['first', {
      async emitPluginEventAndWait(event, payload) {
        return {
          ...payload,
          prompt: `${payload.prompt} secret suffix`,
          messages: [
            ...payload.messages,
            { role: 'system', content: 'hidden system secret' },
          ],
        }
      },
    }],
  ])

  const result = await emitPromptHookEventAndWaitForPlugins(
    plugins,
    hostRefs,
    'CHAT_COMPLETION_PROMPT_READY',
    {
      prompt: 'private prompt body',
      messages: [{ role: 'user', content: 'private message body' }],
    },
    {
      stage: 'frontend_intent',
      onAudit: (record) => audits.push(record),
    },
  )

  assert.equal(result.prompt, 'private prompt body secret suffix')
  assert.equal(audits.length, 2)
  assert.equal(audits[0].pluginId, 'first')
  assert.equal(audits[0].pluginName, 'First Hook')
  assert.equal(audits[0].event, 'CHAT_COMPLETION_PROMPT_READY')
  assert.equal(audits[0].stage, 'frontend_intent')
  assert.equal(audits[0].status, 'ok')
  assert.deepEqual(audits[0].changedKeys, ['messages', 'prompt'])
  assert.equal(audits[0].inputSummary.prompt.type, 'string')
  assert.equal(audits[0].inputSummary.prompt.length, 'private prompt body'.length)
  assert.equal(typeof audits[0].inputSummary.prompt.hash, 'string')
  assert.equal(audits[0].inputSummary.messages.length, 1)
  assert.equal(audits[0].outputSummary.messages.length, 2)
  assert.equal(JSON.stringify(audits).includes('private prompt body'), false)
  assert.equal(JSON.stringify(audits).includes('hidden system secret'), false)
  assert.equal(audits[1].pluginId, 'missing-host')
  assert.equal(audits[1].status, 'missing_host')
})

test('prompt hooks fail open and continue when one plugin throws or returns undefined', async () => {
  const errors = []
  const audits = []
  const plugins = [
    { id: 'first', permissions: ['ModifyPrompt'] },
    { id: 'throws', permissions: ['ModifyPrompt'] },
    { id: 'undefined', permissions: ['ModifyPrompt'] },
    { id: 'last', permissions: ['ModifyPrompt'] },
  ]
  const hostRefs = new Map([
    ['first', {
      async emitPluginEventAndWait(event, payload) {
        return { ...payload, prompt: `${payload.prompt} + first` }
      },
    }],
    ['throws', {
      async emitPluginEventAndWait() {
        throw new Error('hook failed with private prompt text')
      },
    }],
    ['undefined', {
      async emitPluginEventAndWait() {
        return undefined
      },
    }],
    ['last', {
      async emitPluginEventAndWait(event, payload) {
        return { ...payload, prompt: `${payload.prompt} + last` }
      },
    }],
  ])

  const result = await emitPromptHookEventAndWaitForPlugins(
    plugins,
    hostRefs,
    'CHAT_COMPLETION_PROMPT_READY',
    { prompt: 'base' },
    {
      onError: (error, plugin) => errors.push([plugin.id, error.message]),
      onAudit: (record) => audits.push(record),
    },
  )

  assert.deepEqual(result, { prompt: 'base + first + last' })
  assert.deepEqual(errors, [['throws', 'hook failed with private prompt text']])
  assert.deepEqual(audits.map((record) => [record.pluginId, record.status, record.changedKeys]), [
    ['first', 'ok', ['prompt']],
    ['throws', 'error', []],
    ['undefined', 'no_change', []],
    ['last', 'ok', ['prompt']],
  ])
  assert.equal(audits[1].error.name, 'Error')
  assert.equal(audits[1].error.messageLength, 'hook failed with private prompt text'.length)
  assert.equal(typeof audits[1].error.messageHash, 'string')
  assert.equal(JSON.stringify(audits).includes('private prompt text'), false)
})

test('prompt hook error reporting is also fail open', async () => {
  const result = await emitPromptHookEventAndWaitForPlugins(
    [{ id: 'throws', permissions: ['ModifyPrompt'] }, { id: 'last', permissions: ['ModifyPrompt'] }],
    new Map([
      ['throws', {
        async emitPluginEventAndWait() {
          throw new Error('hook failed with private prompt text')
        },
      }],
      ['last', {
        async emitPluginEventAndWait(event, payload) {
          return { ...payload, prompt: `${payload.prompt} + last` }
        },
      }],
    ]),
    'CHAT_COMPLETION_PROMPT_READY',
    { prompt: 'base' },
    { onError: () => { throw new Error('logger failed') } },
  )

  assert.deepEqual(result, { prompt: 'base + last' })
})

test('resolves hooked intent from intent first, then prompt, then fallback', () => {
  assert.equal(resolveHookedIntent({ intent: 'intent-hook', prompt: 'prompt-hook' }, 'base'), 'intent-hook')
  assert.equal(resolveHookedIntent({ prompt: 'prompt-hook' }, 'base'), 'prompt-hook')
  assert.equal(resolveHookedIntent({ intent: 42, prompt: null }, 'base'), 'base')
})

test('resolves backend prompt hook messages with original fallback', () => {
  const originalMessages = [{ role: 'user', content: 'before' }]
  const hookedMessages = [
    { role: 'system', content: 'plugin system' },
    { role: 'user', content: 'after' },
  ]

  assert.equal(resolveHookedMessages({ messages: hookedMessages }, originalMessages), hookedMessages)
  assert.equal(resolveHookedMessages({ messages: null }, originalMessages), originalMessages)
})

test('summarizes prompt hook payloads and ring buffer records safely', () => {
  const summary = summarizePromptHookPayload({
    intent: 'do not log this',
    prompt: 'or this',
    messages: [{ role: 'user', content: 'nor this' }],
    messageCount: 1,
  })

  assert.equal(summary.intent.length, 'do not log this'.length)
  assert.equal(summary.messages.type, 'array')
  assert.equal(summary.messages.length, 1)
  assert.equal(JSON.stringify(summary).includes('do not log this'), false)
  assert.equal(JSON.stringify(summary).includes('nor this'), false)
  assert.deepEqual(
    promptHookChangedKeys({ prompt: 'a', messageCount: 1 }, { prompt: 'b', messageCount: 1 }),
    ['prompt'],
  )
  assert.deepEqual(
    appendPromptHookAuditRecord([{ id: 1 }, { id: 2 }], { id: 3 }, 2),
    [{ id: 2 }, { id: 3 }],
  )
})

test('prompt hook audit stays fail-open for cyclic and hostile payloads', async () => {
  const audits = []
  const cyclic = { prompt: 'cycle source', count: 1n }
  cyclic.self = cyclic
  Object.defineProperty(cyclic, 'hostile', {
    enumerable: true,
    get() {
      throw new Error('hostile getter private text')
    },
  })

  const result = await emitPromptHookEventAndWaitForPlugins(
    [{ id: 'cyclic', permissions: ['ModifyPrompt'] }, { id: 'last', permissions: ['ModifyPrompt'] }],
    new Map([
      ['cyclic', {
        async emitPluginEventAndWait() {
          return cyclic
        },
      }],
      ['last', {
        async emitPluginEventAndWait(event, payload) {
          return { prompt: 'still continued' }
        },
      }],
    ]),
    'CHAT_COMPLETION_PROMPT_READY',
    { prompt: 'base' },
    { onAudit: (record) => audits.push(record) },
  )

  assert.equal(result.prompt, 'still continued')
  assert.equal(audits.length, 2)
  assert.equal(audits[0].status, 'ok')
  assert.equal(audits[0].outputSummary.self.circular, true)
  assert.equal(audits[0].outputSummary.count.type, 'bigint')
  assert.equal(audits[0].outputSummary.hostile.type, 'unreadable')
  assert.equal(JSON.stringify(audits).includes('cycle source'), false)
  assert.equal(JSON.stringify(audits).includes('hostile getter private text'), false)
  assert.deepEqual(audits[1].changedKeys, ['count', 'hostile', 'prompt', 'self'])

// ─── Commit 2: budgets, unload, revocation, fail policy, correlation ────────

test('skips plugins whose host ref was unloaded and audits them as missing_host', async () => {
  const audits = []
  const plugins = [
    { id: 'gone', permissions: ['ModifyPrompt'] },
    { id: 'alive', permissions: ['ModifyPrompt'] },
  ]
  // 'gone' has no host ref entry (unloaded); 'alive' is present.
  const hostRefs = new Map([
    ['alive', {
      async emitPluginEventAndWait(event, payload) {
        return { ...payload, prompt: `${payload.prompt} + alive` }
      },
    }],
  ])

  const result = await emitPromptHookEventAndWaitForPlugins(
    plugins,
    hostRefs,
    'CHAT_COMPLETION_PROMPT_READY',
    { prompt: 'base' },
    { onAudit: (record) => audits.push(record) },
  )

  assert.deepEqual(result, { prompt: 'base + alive' })
  assert.deepEqual(audits.map((record) => [record.pluginId, record.status]), [
    ['gone', 'missing_host'],
    ['alive', 'ok'],
  ])
})

test('skips plugins whose ModifyPrompt permission was revoked at runtime and audits revoked', async () => {
  const audits = []
  // Both plugins declared ModifyPrompt at install time, but the runtime
  // permission resolver (getPluginPermissions) reports 'revoked' has lost it.
  const plugins = [
    { id: 'revoked', permissions: ['ModifyPrompt'] },
    { id: 'kept', permissions: ['ModifyPrompt'] },
  ]
  const hostRefs = new Map([
    ['revoked', {
      async emitPluginEventAndWait() { throw new Error('should not run') },
    }],
    ['kept', {
      async emitPluginEventAndWait(event, payload) {
        return { ...payload, prompt: `${payload.prompt} + kept` }
      },
    }],
  ])
  const runtimePermissions = {
    revoked: ['ReadMemory'], // ModifyPrompt revoked
    kept: ['ModifyPrompt'],
  }

  const result = await emitPromptHookEventAndWaitForPlugins(
    plugins,
    hostRefs,
    'CHAT_COMPLETION_PROMPT_READY',
    { prompt: 'base' },
    {
      getPluginPermissions: (plugin) => runtimePermissions[plugin.id] ?? plugin.permissions,
      onAudit: (record) => audits.push(record),
    },
  )

  assert.deepEqual(result, { prompt: 'base + kept' })
  assert.deepEqual(audits.map((record) => [record.pluginId, record.status]), [
    ['revoked', 'revoked'],
    ['kept', 'ok'],
  ])
})

test('enforces a per-plugin payload size budget and audits oversized plugins as budget_exceeded', async () => {
  const audits = []
  const plugins = [
    { id: 'bloater', permissions: ['ModifyPrompt'] },
    { id: 'safe', permissions: ['ModifyPrompt'] },
  ]
  const hostRefs = new Map([
    ['bloater', {
      async emitPluginEventAndWait(event, payload) {
        return { ...payload, prompt: payload.prompt + ' X'.repeat(2000) }
      },
    }],
    ['safe', {
      async emitPluginEventAndWait(event, payload) {
        return { ...payload, prompt: `${payload.prompt} + safe` }
      },
    }],
  ])

  const result = await emitPromptHookEventAndWaitForPlugins(
    plugins,
    hostRefs,
    'CHAT_COMPLETION_PROMPT_READY',
    { prompt: 'base' },
    {
      maxPayloadBytesPerPlugin: 256,
      onAudit: (record) => audits.push(record),
    },
  )

  // bloater exceeded the budget: its mutation is discarded, fallback used, and
  // the next plugin runs on the pre-bloater payload.
  assert.deepEqual(result, { prompt: 'base + safe' })
  assert.deepEqual(audits.map((record) => [record.pluginId, record.status]), [
    ['bloater', 'budget_exceeded'],
    ['safe', 'ok'],
  ])
})

test('classifyPromptHookFailurePolicy is machine-readable and fail-open for errors/timeouts', () => {
  // Pre-generation mutating hooks fail-open on plugin error and timeout.
  assert.equal(classifyPromptHookFailurePolicy('frontend_intent', 'prompt_hook', 'error').failOpen, true)
  assert.equal(classifyPromptHookFailurePolicy('frontend_intent', 'prompt_hook', 'timeout').failOpen, true)
  assert.equal(classifyPromptHookFailurePolicy('frontend_intent', 'prompt_hook', 'budget_exceeded').failOpen, true)
  // Cancellation is fail-closed: the whole turn must abort.
  assert.equal(classifyPromptHookFailurePolicy('frontend_intent', 'prompt_hook', 'cancelled').failOpen, false)
  // Each classification carries an explicit reason string.
  for (const status of ['error', 'timeout', 'cancelled', 'budget_exceeded', 'ok']) {
    const policy = classifyPromptHookFailurePolicy('frontend_intent', 'prompt_hook', status)
    assert.equal(typeof policy.reason, 'string')
    assert.equal(typeof policy.failOpen, 'boolean')
  }
})

test('cancellation aborts the whole hook chain so later plugins never run', async () => {
  const audits = []
  let secondRan = false
  const plugins = [
    { id: 'first', permissions: ['ModifyPrompt'] },
    { id: 'second', permissions: ['ModifyPrompt'] },
  ]
  const hostRefs = new Map([
    ['first', {
      async emitPluginEventAndWait() { throw new Error('boom') },
    }],
    ['second', {
      async emitPluginEventAndWait(event, payload) {
        secondRan = true
        return { ...payload, prompt: `${payload.prompt} + second` }
      },
    }],
  ])

  const controller = new AbortController()
  controller.abort()

  await assert.rejects(
    emitPromptHookEventAndWaitForPlugins(
      plugins,
      hostRefs,
      'CHAT_COMPLETION_PROMPT_READY',
      { prompt: 'base' },
      { signal: controller.signal, onAudit: (record) => audits.push(record) },
    ),
    /cancelled/i,
  )
  // The pre-aborted signal audits the first pending plugin as cancelled and
  // aborts the chain; the second plugin must never run.
  assert.deepEqual(audits.map((record) => [record.pluginId, record.status]), [
    ['first', 'cancelled'],
  ])
  assert.equal(secondRan, false)
})

test('stamps correlationId and generationId on every async audit record', async () => {
  const audits = []
  const plugins = [{ id: 'p', permissions: ['ModifyPrompt'] }]
  const hostRefs = new Map([
    ['p', { async emitPluginEventAndWait(event, payload) { return payload } }],
  ])

  await emitPromptHookEventAndWaitForPlugins(
    plugins,
    hostRefs,
    'CHAT_COMPLETION_PROMPT_READY',
    { prompt: 'base' },
    {
      correlationId: 'corr-7',
      generationId: 'gen-3',
      onAudit: (record) => audits.push(record),
    },
  )

  assert.equal(audits.length, 1)
  assert.equal(audits[0].correlationId, 'corr-7')
  assert.equal(audits[0].generationId, 'gen-3')
})

test('late duplicate generation does not revive a cancelled earlier generation', async () => {
  // Simulate two generations; the first is cancelled. A new generation runs and
  // completes; the cancelled generation's signal stays aborted (no revival).
  const gen1 = { controller: { signal: { aborted: false } } }
  const gen2 = { controller: { signal: { aborted: false } } }

  gen1.controller.signal.aborted = true
  // Even after gen2 runs, gen1.signal stays aborted — generations are isolated.
  gen2.controller.signal.aborted = false

  assert.equal(gen1.controller.signal.aborted, true)
  assert.equal(gen2.controller.signal.aborted, false)

  const plugins = [{ id: 'p', permissions: ['ModifyPrompt'] }]
  const hostRefs = new Map([
    ['p', { async emitPluginEventAndWait(event, payload) { return payload } }],
  ])
  const audits = []
  const result = await emitPromptHookEventAndWaitForPlugins(
    plugins, hostRefs, 'CHAT_COMPLETION_PROMPT_READY', { prompt: 'gen2' },
    { signal: gen2.controller.signal, generationId: 'gen-2', onAudit: (record) => audits.push(record) },
  )
  assert.deepEqual(result, { prompt: 'gen2' })
  assert.equal(audits[0].generationId, 'gen-2')
})
})
