import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import LogPanel from '../../src/components-v2/debug/LogPanel.vue'

describe('LogPanel level threshold', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    const tauriInternals = {
      invoke(command) {
        if (command === 'log_query') return Promise.resolve([])
        return Promise.resolve(null)
      },
    }
    globalThis.__TAURI_INTERNALS__ = tauriInternals
    window.__TAURI_INTERNALS__ = tauriInternals
  })

  afterEach(() => {
    delete globalThis.__TAURI_INTERNALS__
    delete window.__TAURI_INTERNALS__
    vi.useRealTimers()
  })

  it('describes the selector as a minimum level instead of an exact-level filter', async () => {
    const wrapper = mount(LogPanel)
    await flushPromises()

    expect(wrapper.text()).toContain('最低级别')
    expect(wrapper.get('button[aria-haspopup="listbox"]').text()).toContain('不限')

    await wrapper.get('button[aria-haspopup="listbox"]').trigger('click')
    const options = wrapper.findAll('[role="option"]').map((option) => option.text())
    expect(options).toContain('Warn 及以上')
    expect(options).toContain('Info 及以上')

    wrapper.unmount()
  })
})
