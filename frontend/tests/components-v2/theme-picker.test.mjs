import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'

describe('ThemePicker', () => {
  let wrapper

  beforeEach(() => {
    vi.resetModules()
    localStorage.clear()
    document.documentElement.classList.remove('dark')
    delete document.documentElement.dataset.palette
    document.head.innerHTML = '<meta name="theme-color" content="#ffffff">'
    document.documentElement.style.setProperty('--color-bg', '#f8f5f7')
  })

  afterEach(() => {
    wrapper?.unmount()
    wrapper = null
    document.documentElement.classList.remove('dark')
    delete document.documentElement.dataset.palette
    document.documentElement.style.removeProperty('--color-bg')
    document.documentElement.style.removeProperty('color-scheme')
    vi.restoreAllMocks()
  })

  async function mountPicker() {
    const { default: ThemePicker } = await import('../../src/components-v2/shell/ThemePicker.vue')
    wrapper = mount(ThemePicker, { attachTo: document.body })
    return wrapper
  }

  it('provides named native radios and applies a palette without dismissing navigation', async () => {
    await mountPicker()
    expect(wrapper.findAll('input[type="radio"]')).toHaveLength(4)
    expect(wrapper.get('input[value="teal"]').element.checked).toBe(true)
    await wrapper.get('input[value="rose"]').setValue()
    expect(document.documentElement.dataset.palette).toBe('rose')
    expect(localStorage.getItem('storyforge-palette')).toBe('rose')
    expect(document.documentElement.classList.contains('dark')).toBe(false)
    expect(wrapper.emitted('close')).toBeUndefined()
  })

  it('restores classic night reading and keeps classic selected when returning to light', async () => {
    localStorage.setItem('storyforge-theme', 'dark')
    localStorage.setItem('storyforge-palette', 'classic')
    await mountPicker()
    expect(wrapper.get('input[aria-label="经典"]').element.checked).toBe(true)
    expect(document.documentElement.dataset.palette).toBe('classic')
    await wrapper.get('[role="switch"]').trigger('click')
    expect(document.documentElement.classList.contains('dark')).toBe(false)
    expect(wrapper.get('input[value="classic"]').element.checked).toBe(true)
    expect(localStorage.getItem('storyforge-palette')).toBe('classic')
  })

  it('retains the palette while toggling night reading and synchronizes browser chrome', async () => {
    localStorage.setItem('storyforge-theme', 'dark')
    localStorage.setItem('storyforge-palette', 'blue')
    await mountPicker()
    expect(wrapper.get('input[value="blue"]').element.checked).toBe(true)
    expect(wrapper.get('[role="switch"]').attributes('aria-checked')).toBe('true')
    await wrapper.get('[role="switch"]').trigger('click')
    expect(document.documentElement.classList.contains('dark')).toBe(false)
    expect(document.documentElement.dataset.palette).toBe('blue')
    expect(localStorage.getItem('storyforge-theme')).toBe('light')
    expect(document.querySelector('meta[name="theme-color"]').content).toBe('#f8f5f7')
  })

  it('ignores invalid selections and updates live even when saving fails', async () => {
    await mountPicker()
    const { useTheme } = await import('../../src/useTheme.js')
    const appearance = useTheme()
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('quota') })
    appearance.setPalette('unknown')
    expect(appearance.palette.value).toBe('teal')
    await wrapper.get('input[value="rose"]').setValue()
    await nextTick()
    expect(document.documentElement.dataset.palette).toBe('rose')
  })
})
