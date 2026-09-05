export const PALETTE_OPTIONS = [
  { id: 'teal', label: '青绿' },
  { id: 'blue', label: '蓝灰' },
  { id: 'rose', label: '玫红' },
  { id: 'classic', label: '经典' },
]

function browserStorage() {
  try { return globalThis.localStorage } catch { return null }
}

export function readThemePreferences(storage = browserStorage()) {
  const read = key => {
    try { return storage?.getItem(key) } catch { return null }
  }
  const palette = read('storyforge-palette')
  return {
    theme: read('storyforge-theme') === 'dark' ? 'dark' : 'light',
    palette: PALETTE_OPTIONS.some(option => option.id === palette) ? palette : 'teal',
  }
}

export function writeThemePreferences({ theme, palette }, storage = browserStorage()) {
  for (const [key, value] of [['storyforge-theme', theme], ['storyforge-palette', palette]]) {
    try { storage?.setItem(key, value) } catch {
      // Storage can be blocked or full; the live selection must still work.
    }
  }
}
