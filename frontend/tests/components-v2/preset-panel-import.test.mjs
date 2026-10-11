import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'

const dialogOpen = vi.fn()
const readFile = vi.fn()
const alertDialog = vi.fn(() => Promise.resolve())

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: dialogOpen }))
vi.mock('@tauri-apps/plugin-fs', () => ({ readFile }))
vi.mock('../../src/components/base/BaseDialog.js', () => ({
  alertDialog,
  confirmDialog: vi.fn(() => Promise.resolve(false)),
}))

const PanelHostStub = {
  props: ['show', 'title', 'side'],
  template: '<section><slot name="header" /><slot /></section>',
}

// 2026-10-11 审计：预设面板空态写着「请先导入」，但此前没有任何导入入口
//（importPreset wrapper 零调用）。本测试锁定导入按钮 → import_preset → 刷新列表。
describe('PresetPanel preset import', () => {
  let invoke
  let presets

  beforeEach(() => {
    presets = []
    invoke = vi.fn((command, args) => {
      if (command === 'list_presets') return Promise.resolve(presets)
      if (command === 'list_global_regex_scripts') return Promise.resolve([])
      if (command === 'import_preset') {
        presets = [
          {
            id: 'preset-1',
            name: '导入的预设',
            prompt_count: 2,
            regex_count: 1,
            imported_at: '2026-10-11T00:00:00Z',
            active: false,
          },
        ]
        return Promise.resolve(`预设 '导入的预设' (id: preset-1) 导入成功，收到 ${args.data.length} 字节`)
      }
      return Promise.resolve(null)
    })
    const tauriInternals = { invoke }
    globalThis.__TAURI_INTERNALS__ = tauriInternals
    window.__TAURI_INTERNALS__ = tauriInternals
    dialogOpen.mockReset()
    readFile.mockReset()
    alertDialog.mockClear()
  })

  afterEach(() => {
    delete globalThis.__TAURI_INTERNALS__
    delete window.__TAURI_INTERNALS__
  })

  async function mountPanel() {
    const { default: PresetPanel } = await import('../../src/components-v2/config/PresetPanel.vue')
    const wrapper = mount(PresetPanel, {
      global: { stubs: { PanelHost: PanelHostStub } },
    })
    await flushPromises()
    return wrapper
  }

  function importButton(wrapper) {
    return wrapper.findAll('button').find((b) => b.text().includes('导入预设'))
  }

  it('imports the selected ST preset file and refreshes the list', async () => {
    dialogOpen.mockResolvedValue('C:/presets/st.json')
    readFile.mockResolvedValue(new Uint8Array([123, 125]))
    const wrapper = await mountPanel()

    expect(wrapper.text()).toContain('还没有预设')
    const button = importButton(wrapper)
    expect(button, 'empty state promises an import entry, so the button must exist').toBeTruthy()

    await button.trigger('click')
    await flushPromises()

    expect(readFile).toHaveBeenCalledWith('C:/presets/st.json')
    const importCall = invoke.mock.calls.find(([command]) => command === 'import_preset')
    expect(importCall?.[1]).toEqual({ data: [123, 125] })
    expect(wrapper.text()).toContain('导入的预设')
    expect(alertDialog).toHaveBeenCalledWith(expect.stringContaining('导入成功'))
  })

  it('does nothing when the file dialog is cancelled', async () => {
    dialogOpen.mockResolvedValue(null)
    const wrapper = await mountPanel()

    await importButton(wrapper).trigger('click')
    await flushPromises()

    expect(readFile).not.toHaveBeenCalled()
    expect(invoke.mock.calls.some(([command]) => command === 'import_preset')).toBe(false)
    expect(alertDialog).not.toHaveBeenCalled()
  })
})
