import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { runInNewContext } from 'node:vm'
import { PALETTE_OPTIONS, readThemePreferences, writeThemePreferences } from '../src/themePreferences.js'

function memoryStorage(values = {}) {
  const entries = new Map(Object.entries(values))
  return {
    getItem: key => entries.get(key) ?? null,
    setItem: (key, value) => entries.set(key, value),
  }
}

test('defaults to light teal and retains the existing dark preference', () => {
  assert.deepEqual(readThemePreferences(memoryStorage()), { theme: 'light', palette: 'teal' })
  assert.deepEqual(readThemePreferences(memoryStorage({ 'storyforge-theme': 'dark' })), {
    theme: 'dark', palette: 'teal',
  })
})

test('all four palettes round-trip independently of light and dark mode', () => {
  assert.deepEqual(PALETTE_OPTIONS.map(option => option.id), ['teal', 'blue', 'rose', 'classic'])
  for (const { id: palette } of PALETTE_OPTIONS) {
    for (const theme of ['light', 'dark']) {
      const storage = memoryStorage()
      writeThemePreferences({ theme, palette }, storage)
      assert.deepEqual(readThemePreferences(storage), { theme, palette })
    }
  }
})

test('restores the classic palette without changing the existing defaults', () => {
  const storage = memoryStorage({ 'storyforge-theme': 'dark', 'storyforge-palette': 'classic' })
  assert.deepEqual(readThemePreferences(storage), { theme: 'dark', palette: 'classic' })
  assert.deepEqual(readThemePreferences(memoryStorage()), { theme: 'light', palette: 'teal' })
})

test('invalid saved values fall back without accepting arbitrary attributes', () => {
  assert.deepEqual(readThemePreferences(memoryStorage({
    'storyforge-theme': 'unknown', 'storyforge-palette': '__proto__',
  })), { theme: 'light', palette: 'teal' })
})

test('unavailable storage does not prevent the application from loading or switching', () => {
  const denied = {
    getItem() { throw new Error('blocked') },
    setItem() { throw new Error('quota exceeded') },
  }
  assert.deepEqual(readThemePreferences(denied), { theme: 'light', palette: 'teal' })
  assert.doesNotThrow(() => writeThemePreferences({ theme: 'dark', palette: 'rose' }, denied))
  assert.deepEqual(readThemePreferences(null), { theme: 'light', palette: 'teal' })
})

test('first-paint bootstrap restores the same palette and mode as the runtime', () => {
  const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8')
  const script = html.match(/<script>([\s\S]*?)<\/script>/)?.[1]
  assert.ok(script)
  for (const palette of [...PALETTE_OPTIONS.map(option => option.id), 'invalid', null]) {
    for (const theme of ['light', 'dark', 'invalid', null]) {
      const storage = memoryStorage({ 'storyforge-theme': theme, 'storyforge-palette': palette })
      const classes = new Set()
      const root = {
        dataset: {},
        style: {},
        classList: {
          add: name => classes.add(name),
          toggle: (name, value) => value ? classes.add(name) : classes.delete(name),
        },
      }
      runInNewContext(script, {
        localStorage: storage,
        document: { documentElement: root, querySelector: () => ({ setAttribute() {} }) },
      })
      const expected = readThemePreferences(storage)
      assert.equal(root.dataset.palette, expected.palette)
      assert.equal(classes.has('dark'), expected.theme === 'dark')
      assert.equal(root.style.colorScheme, expected.theme)
    }
  }
})
