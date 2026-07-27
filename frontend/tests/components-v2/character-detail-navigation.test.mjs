import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import AppV2 from '../../src/AppV2.vue'
import { useUiStore } from '../../src/stores/ui.js'

function installTauriMock() {
  const card = {
    id: 'card-seraphina',
    source_character_id: 'source-seraphina',
    name: 'Seraphina',
    imported_at: '2026-07-27T09:25:41Z',
    extraction_status: 'extracted',
    character_definitions: [
      {
        id: 'definition-seraphina',
        name: 'Seraphina',
        role_type: 'Protagonist',
        group: '',
        persona_prompt: '你是魔法森林 Eldoria 的温柔守护者。',
        behavior_rules: '保护受伤的旅人，不伤害无辜者。',
        base_backstory: ['你长期守护 Eldoria。'],
      },
    ],
  }

  const character = {
    id: 'stored-seraphina',
    source_character_id: 'source-seraphina',
    name: 'Seraphina',
    description: 'A caring guardian of the enchanted forest.',
    personality: 'caring, protective, compassionate',
    scenario: 'You are recovering in her forest glade.',
    first_mes: 'You wake in the safety of Seraphina\'s glade.',
    system_prompt: '',
    spec_version: '3.0',
    tags: ['fantasy', 'magic'],
    world_info_count: 1,
    world_info_entries: [
      { keys: ['eldoria'], content: 'Eldoria is an enchanted forest.' },
    ],
    creator: 'fixture',
  }

  const emptyCommands = new Set([
    'list_conversations',
    'list_campaigns',
    'list_instances',
    'list_presets',
    'list_modules',
    'list_agent_profile_configs',
    'list_global_regex_scripts',
    'list_connection_templates',
    'list_models',
    'list_connections',
    'list_plugins',
    'log_query',
  ])

  globalThis.__TAURI_INTERNALS__ = {
    invoke(command) {
      if (command === 'get_version') return Promise.resolve('test')
      if (command === 'list_cards') return Promise.resolve([card])
      if (command === 'get_card') return Promise.resolve(card)
      if (command === 'get_character') return Promise.resolve(character)
      if (command === 'get_active_connection') return Promise.resolve(null)
      if (command === 'get_active_campaign') return Promise.resolve(null)
      if (emptyCommands.has(command)) return Promise.resolve([])
      return Promise.resolve(null)
    },
    transformCallback() {
      return 1
    },
  }
}

describe('character card detail navigation', () => {
  let wrapper

  beforeEach(() => {
    setActivePinia(createPinia())
    installTauriMock()
  })

  afterEach(async () => {
    wrapper?.unmount()
    await flushPromises()
    wrapper = null
    delete globalThis.__TAURI_INTERNALS__
    document.body.innerHTML = ''
  })

  it('opens a readable card detail with extracted character definitions', async () => {
    wrapper = mount(AppV2, {
      attachTo: document.body,
      global: {
        stubs: {
          MvuJsRuntime: true,
          PluginHost: true,
          teleport: true,
        },
      },
    })
    await flushPromises()

    const ui = useUiStore()
    ui.showCharList = true
    await nextTick()
    await flushPromises()

    expect(document.body.textContent).toContain('角色卡库')
    expect(document.body.textContent).toContain('2026年7月27日')
    expect(document.body.textContent).not.toContain('2026-07-27T09:25:41Z')

    const row = wrapper.find('button[aria-label="查看 Seraphina 详情"]')
    expect(row.exists()).toBe(true)
    const deleteButton = wrapper.find('button[aria-label="删除 Seraphina"]')
    expect(deleteButton.exists()).toBe(true)
    await row.trigger('click')
    await flushPromises()
    await nextTick()

    expect(document.body.textContent).toContain('角色详情')
    expect(document.body.textContent).toContain('Seraphina')
    expect(document.body.textContent).toContain('识别出的角色定义')
    expect(document.body.textContent).toContain('魔法森林 Eldoria 的温柔守护者')
    expect(document.body.textContent).toContain('概览')
    expect(document.body.textContent).toContain('原始资料')
    expect(document.body.textContent).not.toContain('A caring guardian of the enchanted forest.')
    expect(document.body.textContent).not.toContain('林医生')
    expect(document.body.textContent).not.toContain('陈警官')

    const detailPanel = document.querySelector('[role="dialog"][aria-label="角色详情"] > div')
    expect(detailPanel).toBeTruthy()
    expect(detailPanel.className).toContain('h-[min(90dvh,52rem)]')
    expect(detailPanel.className).toContain('rounded-2xl')
    expect(detailPanel.className).not.toContain('rounded-t-2xl')

    const sourceTab = wrapper.findAll('button').find((button) => button.text().includes('原始资料'))
    expect(sourceTab).toBeTruthy()
    await sourceTab.trigger('click')
    await nextTick()
    expect(document.body.textContent).toContain('A caring guardian of the enchanted forest.')
  })
})
