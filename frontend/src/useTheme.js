import { ref, watch } from 'vue'
import { PALETTE_OPTIONS, readThemePreferences, writeThemePreferences } from './themePreferences.js'

const initial = readThemePreferences()
const theme = ref(initial.theme)
const palette = ref(initial.palette)

function applyTheme() {
  if (typeof document === 'undefined') return
  const root = document.documentElement
  root.classList.toggle('dark', theme.value === 'dark')
  root.dataset.palette = palette.value
  root.style.colorScheme = theme.value
  // Read the active CSS token so browser chrome cannot drift from the palette.
  const meta = document.querySelector('meta[name="theme-color"]')
  const background = getComputedStyle(root).getPropertyValue('--color-bg').trim()
  if (meta && background) meta.setAttribute('content', background)
}

applyTheme()

// One application-wide subscription, independent of sidebar visibility.
watch([theme, palette], () => {
  applyTheme()
  writeThemePreferences({ theme: theme.value, palette: palette.value })
}, { flush: 'sync' })

export function useTheme() {
  function toggle() {
    theme.value = theme.value === 'dark' ? 'light' : 'dark'
  }

  function setPalette(value) {
    if (PALETTE_OPTIONS.some(option => option.id === value)) palette.value = value
  }

  return { theme, palette, palettes: PALETTE_OPTIONS, toggle, setPalette }
}
