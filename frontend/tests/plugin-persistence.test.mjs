import test from 'node:test'
import assert from 'node:assert/strict'

import {
  createSaveChatAdapter,
  createDefaultSaveChatAdapter,
  createPopupAdapter,
  createDefaultPopupAdapter,
  createRequestHeadersAdapter,
  createDefaultRequestHeadersAdapter,
  applyPersistenceAdapters,
  classifySaveChatResult,
  PERSISTENCE_DEGRADED_REASON,
} from '../src/utils/pluginPersistence.js'

function awaitable(result) {
  const pending = Promise.resolve(result.ok === undefined ? true : result.value === undefined ? true : result.value)
  pending.ok = result.ok
  pending.degraded = Boolean(result.degraded)
  pending.reason = result.reason || null
  return pending
}

test('default saveChat adapter is deterministic, in-memory, and degraded with ST-compatible promise', async () => {
  const adapter = createDefaultSaveChatAdapter()
  const result = await adapter.saveChat({ pluginId: 'plugin-a', chat: [{ role: 'user', content: 'hi' }] })

  assert.equal(result.ok, true)
  assert.equal(result.degraded, true)
  assert.equal(result.reason, PERSISTENCE_DEGRADED_REASON)
  assert.equal(result.persistedAt, null)

  const promise = classifySaveChatResult(result)
  // The ST boolean-compatibility contract: an awaited saveChat resolves truthy
  // AND the pending promise carries a visible degraded marker.
  assert.equal(await promise, true)
  assert.equal(promise.degraded, true)
  assert.equal(promise.reason, PERSISTENCE_DEGRADED_REASON)
})

test('custom saveChat adapter persists chat snapshots deterministically and is not degraded', async () => {
  const saved = []
  const adapter = createSaveChatAdapter({
    persist(snapshot) {
      saved.push(snapshot)
      return { ok: true, persistedAt: 'fixed-ts' }
    },
  })

  const result = await adapter.saveChat({ pluginId: 'plugin-a', chat: [{ role: 'user', content: 'hi' }] })

  assert.equal(result.ok, true)
  assert.equal(result.degraded, false)
  assert.equal(result.reason, null)
  assert.equal(result.persistedAt, 'fixed-ts')
  assert.deepEqual(saved, [{ pluginId: 'plugin-a', chat: [{ role: 'user', content: 'hi' }] }])
})

test('saveChat adapter respects cancellation signal and reports cancelled explicitly', async () => {
  const controller = new AbortController()
  let persistCalls = 0
  const adapter = createSaveChatAdapter({
    async persist() {
      persistCalls += 1
      return { ok: true }
    },
  })

  controller.abort()
  const result = await adapter.saveChat(
    { pluginId: 'plugin-a', chat: [] },
    { signal: controller.signal },
  )

  assert.equal(result.ok, false)
  assert.equal(result.cancelled, true)
  assert.equal(result.reason, 'cancelled')
  assert.equal(persistCalls, 0)
})

test('saveChat adapter surfaces persist failures as degraded with reason but stays fail-open for ST', async () => {
  const adapter = createSaveChatAdapter({
    async persist() {
      throw new Error('backend down with secret SF_SECRET_x')
    },
  })

  const result = await adapter.saveChat({ pluginId: 'plugin-a', chat: [] })

  assert.equal(result.ok, false)
  assert.equal(result.degraded, true)
  assert.equal(result.cancelled, false)
  // Error message must not be inlined raw into the result shape.
  assert.equal(result.reason, 'persist_failed')
  assert.equal(JSON.stringify(result).includes('SF_SECRET_'), false)
})

test('popup adapter returns explicit defaults without exposing UI, and supports cancel/unsupported', async () => {
  const adapter = createDefaultPopupAdapter()

  assert.equal(await adapter.popup({ type: 3, defaultValue: 'draft' }), 'draft')
  assert.equal(await adapter.popup({ type: 2 }), null)
  assert.equal(await adapter.popup({}), '')

  const classified = adapter.popup({ type: 'confirm' })
  const resolved = await classified
  assert.equal(resolved, null)
})

test('custom popup adapter honors permission gating and never returns secrets', async () => {
  const adapter = createPopupAdapter({
    isAuthorized: (plugin) => plugin?.permissions?.includes('Notifications'),
    async render({ type, defaultValue }) {
      assert.equal(type, 3)
      return String(defaultValue)
    },
  })

  const allowed = await adapter.popup({ type: 3, defaultValue: 'ok' }, { permissions: ['Notifications'] })
  assert.equal(allowed, 'ok')

  const denied = await adapter.popup({ type: 3, defaultValue: 'ok' }, { permissions: [] })
  assert.equal(denied, undefined)
  assert.equal(denied?.degraded, undefined) // no degraded marker; explicit unsupported contract
})

test('request headers adapter never exposes authorization, api keys, or bearer tokens', () => {
  const adapter = createDefaultRequestHeadersAdapter()
  const headers = adapter.getHeaders()

  assert.deepEqual(headers, { 'Content-Type': 'application/json' })
  assert.equal('Authorization' in headers, false)
  assert.equal('authorization' in headers, false)
  assert.match(JSON.stringify(headers), /^[^{]*\{[^}]*Content-Type[^}]*\}?$/)
})

test('custom request headers adapter is redacted before it can leak credentials', () => {
  const adapter = createRequestHeadersAdapter({
    getHeaders: () => ({
      'Content-Type': 'application/json',
      Authorization: 'Bearer secret-token',
      'x-api-key': 'SF_SECRET_abc',
    }),
  })

  const headers = adapter.getHeaders()

  assert.equal(headers.Authorization, undefined)
  assert.equal(headers['x-api-key'], undefined)
  assert.equal(headers['Content-Type'], 'application/json')
})

test('applyPersistenceAdapters merges partial adapters and falls back to defaults', async () => {
  const saved = []
  const merged = applyPersistenceAdapters({
    saveChat: createSaveChatAdapter({
      persist(snapshot) {
        saved.push(snapshot)
        return { ok: true }
      },
    }),
  })

  assert.equal(typeof merged.popup.popup, 'function')
  assert.equal(typeof merged.requestHeaders.getHeaders, 'function')

  const saveResult = await merged.saveChat.saveChat({ pluginId: 'p', chat: [] })
  assert.equal(saveResult.ok, true)
  assert.deepEqual(saved, [{ pluginId: 'p', chat: [] }])

  const popupResult = await merged.popup.popup({ type: 2 })
  assert.equal(popupResult, null)

  const headers = merged.requestHeaders.getHeaders()
  assert.deepEqual(headers, { 'Content-Type': 'application/json' })
})

test('applyPersistenceAdapters with no overrides yields fully degraded defaults', async () => {
  const merged = applyPersistenceAdapters({})

  const saveResult = await merged.saveChat.saveChat({ pluginId: 'p', chat: [] })
  assert.equal(saveResult.degraded, true)
  assert.equal(saveResult.reason, PERSISTENCE_DEGRADED_REASON)

  const promise = classifySaveChatResult(saveResult)
  assert.equal(promise.degraded, true)
})

test('awaitable helper keeps the ST-compatible saveChat contract', async () => {
  const result = { ok: true, degraded: false, reason: null }
  const pending = awaitable(result)
  assert.equal(await pending, true)
  assert.equal(pending.degraded, false)
})
