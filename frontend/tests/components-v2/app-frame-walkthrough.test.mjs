/**
 * AppFrame 生产壳 + 深浅主题 + 窄屏主流程 / 弹层走查（happy-dom）。
 * 用 mock Tauri IPC 装配真实 AppV2；stub 隐藏 runtime 避免 tauri event unlisten。
 */
import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import AppV2 from '../../src/AppV2.vue'
import { useUiStore } from '../../src/stores/ui.js'

function installTauriMock() {
  const calls = []
  const mockValue = (command) => {
    switch (command) {
      case 'get_version':
        return 'walkthrough'
      case 'get_active_connection':
        return {
          id: 'mock-conn',
          name: 'Mock Local LLM',
          base_url: 'http://127.0.0.1:11434/v1',
          protocol: 'OpenAI',
          model: 'mock-model',
          tool_mode: 'native',
          active: true,
        }
      case 'list_plugins':
        return [
          {
            id: 'walk-plugin',
            name: 'Walk Plugin',
            version: '0.0.1',
            enabled: true,
            permissions: [],
            ui_slots: [],
            manifest: { name: 'Walk Plugin' },
          },
        ]
      case 'list_conversations':
      case 'list_cards':
      case 'list_campaigns':
      case 'list_instances':
      case 'list_presets':
      case 'list_modules':
      case 'list_agent_profile_configs':
      case 'list_global_regex_scripts':
      case 'list_connection_templates':
      case 'list_models':
      case 'list_connections':
      case 'log_query':
        return []
      case 'get_active_campaign':
      case 'get_conversation':
      case 'get_active_preset':
      case 'get_active_agent_profile_config':
      case 'get_agent_profile_config':
      case 'get_active_profile':
      case 'get_card':
        return null
      case 'log_export_bundle':
        return { exported_at: new Date().toISOString(), counts: {} }
      default:
        return null
    }
  }

  globalThis.__TAURI_INTERNALS__ = {
    invoke(command, args) {
      calls.push({ command, args })
      return Promise.resolve(mockValue(command))
    },
    transformCallback(callback) {
      const id = calls.length + 1
      globalThis[`_${id}`] = callback
      return id
    },
  }
  return calls
}

describe('AppFrame production walkthrough', () => {
  let wrapper
  let calls

  beforeEach(() => {
    localStorage.clear()
    document.documentElement.classList.remove('dark')
    setActivePinia(createPinia())
    calls = installTauriMock()
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1280 })
  })

  afterEach(async () => {
    if (wrapper) {
      wrapper.unmount()
      await flushPromises()
    }
    wrapper = null
    document.documentElement.classList.remove('dark')
    localStorage.clear()
    delete globalThis.__TAURI_INTERNALS__
  })

  async function mountApp() {
    wrapper = mount(AppV2, {
      attachTo: document.body,
      global: {
        stubs: {
          // 冻结 runtime：真实组件依赖 tauri event listen，happy-dom 无完整 IPC
          MvuJsRuntime: true,
          PluginHost: true,
          teleport: true,
        },
      },
    })
    await flushPromises()
    await nextTick()
    return wrapper
  }

  it('mounts AppFrame shell; default history empty matches product routing', async () => {
    await mountApp()
    expect(document.documentElement.classList.contains('dark')).toBe(false)
    expect(wrapper.find('.sf-safe-screen.h-dvh').exists()).toBe(true)
    // showHistory 默认 true 且无 campaign → currentView=history
    expect(wrapper.text()).toContain('还没有写下第一笔')
    expect(wrapper.text()).toContain('StoryForge')
    expect(wrapper.find('header').exists()).toBe(true)
    const cmds = calls.map((c) => c.command)
    expect(cmds).toContain('get_version')
    expect(cmds).toContain('get_active_connection')
    expect(cmds).toContain('list_plugins')
  })

  it('switches to write view and shows design EmptyHero', async () => {
    await mountApp()
    const ui = useUiStore()
    ui.viewWrite()
    await nextTick()
    expect(wrapper.text()).toContain('开始你的故事')
    expect(wrapper.text()).toContain('导入角色卡')
    expect(wrapper.text()).toContain('新建 Campaign')
  })

  it('toggles night-read theme via sidebar control', async () => {
    await mountApp()
    const themeBtn = wrapper.findAll('button').find((b) => /夜读模式|切换为浅色/.test(b.text()))
    expect(themeBtn).toBeTruthy()
    await themeBtn.trigger('click')
    await nextTick()
    expect(document.documentElement.classList.contains('dark')).toBe(true)
    expect(localStorage.getItem('storyforge-theme')).toBe('dark')
    const lightBtn = wrapper.findAll('button').find((b) => /切换为浅色|夜读模式/.test(b.text()))
    await lightBtn.trigger('click')
    await nextTick()
    expect(document.documentElement.classList.contains('dark')).toBe(false)
  })

  it('opens inspector overlay without crushing main column', async () => {
    await mountApp()
    const ui = useUiStore()
    ui.viewWrite()
    await nextTick()
    const debugBtn = wrapper.find('header button[aria-label="过程与调试"]')
    expect(debugBtn.exists()).toBe(true)
    await debugBtn.trigger('click')
    await nextTick()
    expect(wrapper.text()).toContain('调试')
    expect(wrapper.text()).toMatch(/流水线|插件事件|Hook|日志/)
    // 主写作空态仍在
    expect(wrapper.text()).toContain('开始你的故事')
  })

  it('opens connection panel from sidebar (overlay dialog path)', async () => {
    await mountApp()
    const connBtn = wrapper.findAll('button').find((b) => b.text().trim() === '连接' || b.text().includes('连接'))
    expect(connBtn).toBeTruthy()
    await connBtn.trigger('click')
    await flushPromises()
    await nextTick()
    expect(wrapper.text()).toMatch(/连接|LLM|模型|Mock Local/)
  })

  it('invokes list_plugins so PluginHost wiring still runs', async () => {
    await mountApp()
    expect(calls.map((c) => c.command)).toContain('list_plugins')
  })

  it('mobile menu opens sidebar drawer path', async () => {
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 390 })
    await mountApp()
    const menuBtn = wrapper.find('header button[aria-label="菜单"]')
    expect(menuBtn.exists()).toBe(true)
    await menuBtn.trigger('click')
    await nextTick()
    const newBtns = wrapper.findAll('button').filter((b) => b.text().includes('新建 Campaign'))
    expect(newBtns.length).toBeGreaterThan(0)
  })

  it('collapses and restores desktop navigation through the production shell', async () => {
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1280 })
    await mountApp()
    const ui = useUiStore()
    await wrapper.get('aside [aria-label="收起侧栏"]').trigger('click')
    await nextTick()
    expect(ui.sidebarCollapsed).toBe(true)
    expect(wrapper.get('aside').isVisible()).toBe(false)
    await wrapper.get('header [aria-label="展开侧栏"]').trigger('click')
    await nextTick()
    expect(ui.sidebarCollapsed).toBe(false)
    expect(wrapper.get('aside').isVisible()).toBe(true)
  })

  it.each([
    ['CampaignPanel', 'showCampaignPanel'],
    ['CharacterList', 'showCharList'],
    ['ConnectionConfigPanel', 'showConnConfig'],
    ['MetaPanel', 'showMetaPanel'],
    ['PresetPanel', 'showPresetPanel'],
    ['PluginPanel', 'showPluginPanel'],
    ['AgentProfileManager', 'showAgentProfile'],
  ])('closing %s does not unexpectedly open mobile navigation', async (name, state) => {
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 390 })
    await mountApp()
    const ui = useUiStore()
    ui.showSidebar = false
    ui[state] = true
    await flushPromises()
    await nextTick()
    wrapper.getComponent({ name }).vm.$emit('close')
    await flushPromises()
    expect(ui[state]).toBe(false)
    expect(ui.showSidebar).toBe(false)
  })
})
