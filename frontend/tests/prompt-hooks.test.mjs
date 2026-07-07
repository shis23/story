import assert from 'node:assert/strict'
import test from 'node:test'

import {
  emitPromptHookEventAndWaitForPlugins,
  resolveHookedIntent,
  resolveHookedMessages,
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
