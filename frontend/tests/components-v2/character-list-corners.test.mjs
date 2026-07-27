import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import BaseOverlay from '../../src/components/base/BaseOverlay.vue'
import CharacterList from '../../src/components/CharacterList.vue'

describe('character card manager corners', () => {
  beforeEach(() => {
    const tauriInternals = {
      invoke: vi.fn((command) => {
        if (command === 'list_cards') return Promise.resolve([])
        return Promise.resolve(null)
      }),
    }
    globalThis.__TAURI_INTERNALS__ = tauriInternals
    window.__TAURI_INTERNALS__ = tauriInternals
  })

  afterEach(() => {
    delete globalThis.__TAURI_INTERNALS__
    delete window.__TAURI_INTERNALS__
  })

  it('lets a side overlay disable its exposed-edge rounding', () => {
    const wrapper = mount(BaseOverlay, {
      props: {
        modelValue: true,
        position: 'left',
        size: 'drawer',
        sideRounded: false,
      },
      global: {
        stubs: { Teleport: true },
      },
    })

    const panel = wrapper.get('[role="dialog"] > div')
    expect(panel.classes()).not.toContain('rounded-r-2xl')
  })

  it('makes the character card manager square on its right edge', () => {
    const wrapper = mount(CharacterList, {
      global: {
        stubs: { Teleport: true },
      },
    })

    expect(wrapper.getComponent(BaseOverlay).props('sideRounded')).toBe(false)
  })
})
