import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'

function loadPluginHostSlotHelpers() {
  const source = fs.readFileSync(new URL('../src/components/PluginHost.vue', import.meta.url), 'utf8')
  const match = source.match(/<script>([\s\S]*?)<\/script>/)
  assert.ok(match, 'PluginHost.vue should expose testable slot helpers')

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

const {
  applyPluginSlotMount,
  getPluginSlotEntries,
  normalizePluginHostSlot,
  waitForPluginHostReady,
  PLUGIN_HOST_DEFAULT_SLOT,
  PLUGIN_HOST_HOOK_READY_TIMEOUT_MS,
} = loadPluginHostSlotHelpers()

function plain(value) {
  return JSON.parse(JSON.stringify(value))
}

test('keeps plugin host slot content isolated by slot name', () => {
  let slots = applyPluginSlotMount({}, { html: '<p>General</p>' })
  slots = applyPluginSlotMount(slots, { slot: 'slash', html: '<button>/heal</button>' })
  slots = applyPluginSlotMount(slots, { slot: 'statusbar', html: '<span>HP 5</span>' })

  assert.deepEqual(plain(slots), {
    [PLUGIN_HOST_DEFAULT_SLOT]: '<p>General</p>',
    slash: '<button>/heal</button>',
    statusbar: '<span>HP 5</span>',
  })

  slots = applyPluginSlotMount(slots, { slot: 'statusbar', html: '' })
  assert.deepEqual(plain(slots), {
    [PLUGIN_HOST_DEFAULT_SLOT]: '<p>General</p>',
    slash: '<button>/heal</button>',
  })
})

test('clearing one named plugin host slot does not clear sibling slots', () => {
  const slots = applyPluginSlotMount({
    slash: '<button>/heal</button>',
    sidebar: '<aside>Notes</aside>',
    statusbar: '<span>HP 5</span>',
  }, { slot: 'statusbar', html: '' })

  assert.deepEqual(plain(slots), {
    slash: '<button>/heal</button>',
    sidebar: '<aside>Notes</aside>',
  })
})

test('normalizes blank slots to default and renders non-empty entries only', () => {
  assert.equal(normalizePluginHostSlot('  '), PLUGIN_HOST_DEFAULT_SLOT)
  assert.equal(normalizePluginHostSlot(' sidebar '), 'sidebar')

  assert.deepEqual(plain(getPluginSlotEntries({
    [PLUGIN_HOST_DEFAULT_SLOT]: '<main>General</main>',
    slash: '',
    sidebar: '<aside>Notes</aside>',
  })), [
    { slot: PLUGIN_HOST_DEFAULT_SLOT, html: '<main>General</main>' },
    { slot: 'sidebar', html: '<aside>Notes</aside>' },
  ])
})

test('waits briefly for plugin hook host readiness before falling back', async () => {
  let ready = false
  let notifyReady = null
  const promise = waitForPluginHostReady(
    () => ready,
    (resolve) => {
      notifyReady = resolve
      return () => {
        notifyReady = null
      }
    },
    PLUGIN_HOST_HOOK_READY_TIMEOUT_MS,
  )

  ready = true
  notifyReady()

  assert.equal(await promise, true)
  assert.equal(notifyReady, null)
})

test('falls back when plugin hook host never becomes ready', async () => {
  const result = await waitForPluginHostReady(
    () => false,
    () => () => {},
    0,
  )

  assert.equal(result, false)
})

test('honors explicit cancellation while waiting for plugin hook host readiness', async () => {
  let ready = false
  let cancelWait = null
  const promise = waitForPluginHostReady(
    () => ready,
    (resolve) => {
      cancelWait = resolve
      return () => {
        cancelWait = null
      }
    },
    PLUGIN_HOST_HOOK_READY_TIMEOUT_MS,
  )

  ready = true
  cancelWait(false)

  assert.equal(await promise, false)
  assert.equal(cancelWait, null)
})
