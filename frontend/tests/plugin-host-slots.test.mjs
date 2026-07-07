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

  const sandbox = {}
  vm.runInNewContext(`${moduleCode}
Object.assign(globalThis, {
  applyPluginSlotMount,
  getPluginSlotEntries,
  normalizePluginHostSlot,
  PLUGIN_HOST_DEFAULT_SLOT,
})`, sandbox)

  return sandbox
}

const {
  applyPluginSlotMount,
  getPluginSlotEntries,
  normalizePluginHostSlot,
  PLUGIN_HOST_DEFAULT_SLOT,
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
