import test from 'node:test'
import assert from 'node:assert/strict'
import {
  applyCampaignWorldbookEnabledUpdates,
  enqueueCampaignWorldbookEnabledUpdates,
  enqueueCampaignWorldbookMutation,
  mapCampaignWorldbookForTavernHelper,
  resolveCampaignWorldbookEnabledUpdates,
} from '../src/utils/cardShellWorldbook.js'

test('maps Campaign world-info names and enabled flags for the TavernHelper shell API', () => {
  const entries = mapCampaignWorldbookForTavernHelper({
    entries: [
      { index: 2, name: '命定系统-阿比盖尔核心', disabled: false, keys: [] },
      { index: 3, name: '', disabled: true, keys: ['fallback key'] },
    ],
  })

  assert.deepEqual(entries, [
    { index: 2, name: '命定系统-阿比盖尔核心', enabled: true },
    { index: 3, name: 'fallback key', enabled: false },
  ])
})

test('writes back only changed enabled flags and applies duplicate names consistently', () => {
  const current = [
    { index: 2, name: '命定系统-甲', disabled: false, keys: [] },
    { index: 3, name: '命定系统-甲', disabled: false, keys: [] },
    { index: 4, name: '命定系统-乙', disabled: true, keys: [] },
  ]

  assert.deepEqual(
    resolveCampaignWorldbookEnabledUpdates(current, [
      { name: '命定系统-甲', enabled: false },
      { name: '命定系统-乙', enabled: false },
    ]),
    [
      { entryIndex: 2, enabled: false },
      { entryIndex: 3, enabled: false },
    ],
  )
})

test('serializes worldbook writes so multi-entry DLC updates cannot race campaign storage', async () => {
  const calls = []
  let inFlight = 0
  let maxInFlight = 0

  await applyCampaignWorldbookEnabledUpdates(
    [
      { entryIndex: 3, enabled: false },
      { entryIndex: 8, enabled: true },
    ],
    async (update) => {
      calls.push(update)
      inFlight += 1
      maxInFlight = Math.max(maxInFlight, inFlight)
      await Promise.resolve()
      inFlight -= 1
    },
  )

  assert.deepEqual(calls, [
    { entryIndex: 3, enabled: false },
    { entryIndex: 8, enabled: true },
  ])
  assert.equal(maxInFlight, 1)
})

test('serializes concurrent write batches from multiple shell hosts in one Campaign', async () => {
  const calls = []
  let inFlight = 0
  let maxInFlight = 0
  let releaseFirst
  const firstStarted = new Promise((resolve) => {
    releaseFirst = resolve
  })
  let firstPersisted
  const firstPersistedPromise = new Promise((resolve) => {
    firstPersisted = resolve
  })

  const persist = async (update) => {
    calls.push(update)
    inFlight += 1
    maxInFlight = Math.max(maxInFlight, inFlight)
    if (calls.length === 1) {
      firstPersisted()
      await firstStarted
    }
    inFlight -= 1
  }

  const first = enqueueCampaignWorldbookEnabledUpdates(
    'campaign-multi-host',
    [{ entryIndex: 2, enabled: false }],
    persist,
  )
  const second = enqueueCampaignWorldbookEnabledUpdates(
    'campaign-multi-host',
    [{ entryIndex: 8, enabled: false }],
    persist,
  )

  await firstPersistedPromise
  assert.equal(maxInFlight, 1)
  releaseFirst()
  await Promise.all([first, second])

  assert.deepEqual(calls, [
    { entryIndex: 2, enabled: false },
    { entryIndex: 8, enabled: false },
  ])
  assert.equal(maxInFlight, 1)
})

test('queues worldbook snapshot resolution so a later shell intent is not stale', async () => {
  const entries = [{ index: 2, name: 'shared core', disabled: false }]
  const writes = []

  const requestEnabled = (enabled) => enqueueCampaignWorldbookMutation(
    'campaign-stale-snapshot',
    async () => {
      const updates = resolveCampaignWorldbookEnabledUpdates(entries, [
        { name: 'shared core', enabled },
      ])
      await applyCampaignWorldbookEnabledUpdates(updates, async (update) => {
        entries[0].disabled = !update.enabled
        writes.push(update)
      })
    },
  )

  await Promise.all([requestEnabled(false), requestEnabled(true)])

  assert.deepEqual(writes, [
    { entryIndex: 2, enabled: false },
    { entryIndex: 2, enabled: true },
  ])
  assert.equal(entries[0].disabled, false)
})
