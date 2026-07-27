import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import CampaignVariablesTab from '../../src/components-v2/campaign/CampaignVariablesTab.vue'

describe('CampaignVariablesTab', () => {
  let invoke

  beforeEach(() => {
    invoke = vi.fn((command, args = {}) => {
      const values = {
        get_campaign_variables: [
          { key: 'story_clock', value: '清晨' },
          { key: 'danger_level', value: 2 },
        ],
        get_campaign_variable_schema: [
          { key: 'story_clock', label: '故事时间', value_type: 'string', default: '第1天' },
          {
            key: 'danger_level',
            label: '危险等级',
            value_type: 'int',
            default: 1,
            description: '当前局势的危险程度',
          },
        ],
        list_instances: [
          { id: 'seraphina', name: 'Seraphina', role_type: 'protagonist', definition_id: 'def-1' },
          { id: 'shadowfang', name: 'Shadowfang', role_type: 'supporting' },
        ],
        get_campaign: { id: 'campaign-1', card_id: 'card-1' },
        get_card: {
          id: 'card-1',
          character_definitions: [{
            id: 'def-1',
            variable_schema: [{ key: 'mood', label: '当前情绪' }],
          }],
        },
        set_campaign_variable: null,
        add_campaign_variable: null,
        sync_campaign_variable_schema: { added: 1 },
        set_character_variable: null,
      }
      if (command === 'get_character_variables') {
        return Promise.resolve(args.instanceId === 'seraphina'
          ? [{ key: 'mood', value: '警觉' }]
          : [{ key: 'nearby', value: true }])
      }
      return Promise.resolve(values[command] ?? null)
    })
    const tauriInternals = { invoke }
    globalThis.__TAURI_INTERNALS__ = tauriInternals
    window.__TAURI_INTERNALS__ = tauriInternals
  })

  afterEach(() => {
    delete globalThis.__TAURI_INTERNALS__
    delete window.__TAURI_INTERNALS__
  })

  it('shows global variables and each character variable group together', async () => {
    const wrapper = mount(CampaignVariablesTab, {
      props: { campaignId: 'campaign-1' },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('全局变量')
    expect(wrapper.text()).toContain('故事时间')
    expect(wrapper.text()).toContain('危险等级')
    expect(wrapper.text()).toContain('当前局势的危险程度')
    expect(wrapper.text()).toContain('story_clock')
    expect(wrapper.get('[aria-label="全局变量 story_clock"]').element.value).toBe('清晨')
    expect(wrapper.text()).toContain('角色变量')
    expect(wrapper.text()).toContain('Seraphina')
    expect(wrapper.text()).toContain('当前情绪')
    expect(wrapper.text()).toContain('mood')
    expect(wrapper.get('[aria-label="Seraphina 变量 mood"]').element.value).toBe('警觉')
    expect(wrapper.text()).toContain('Shadowfang')
    expect(wrapper.text()).toContain('nearby')
    const characterGroups = wrapper.findAll('details')
    expect(characterGroups[0].attributes()).toHaveProperty('open')
    expect(characterGroups[1].attributes()).not.toHaveProperty('open')
  })

  it('adds a typed Campaign variable through the global schema form', async () => {
    const wrapper = mount(CampaignVariablesTab, {
      props: { campaignId: 'campaign-1' },
    })
    await flushPromises()

    await wrapper.get('button[aria-label="新增全局变量"]').trigger('click')
    await wrapper.get('[aria-label="变量键名"]').setValue('faction_tension')
    await wrapper.get('[aria-label="变量中文名"]').setValue('阵营紧张度')
    await wrapper.get('[aria-label="变量类型"]').setValue('int')
    await wrapper.get('[aria-label="变量初始值"]').setValue('12')
    await wrapper.get('button[aria-label="保存全局变量"]').trigger('click')
    await flushPromises()

    expect(invoke).toHaveBeenCalledWith('add_campaign_variable', {
      campaignId: 'campaign-1',
      key: 'faction_tension',
      label: '阵营紧张度',
      valueType: 'int',
      defaultValue: 12,
      description: null,
    }, undefined)
  })

  it('can explicitly sync newly imported card globals into an existing Campaign', async () => {
    const wrapper = mount(CampaignVariablesTab, {
      props: { campaignId: 'campaign-1' },
    })
    await flushPromises()

    await wrapper.get('button[aria-label="同步卡片全局变量"]').trigger('click')
    await flushPromises()

    expect(invoke).toHaveBeenCalledWith('sync_campaign_variable_schema', {
      campaignId: 'campaign-1',
    }, undefined)
  })

  it('persists an edited global variable with its original type', async () => {
    const wrapper = mount(CampaignVariablesTab, {
      props: { campaignId: 'campaign-1' },
    })
    await flushPromises()

    await wrapper.get('[aria-label="全局变量 danger_level"]').setValue('4')
    await wrapper.get('[aria-label="全局变量 danger_level"]').trigger('change')
    await flushPromises()

    expect(invoke).toHaveBeenCalledWith('set_campaign_variable', {
      campaignId: 'campaign-1',
      key: 'danger_level',
      value: 4,
    }, undefined)
  })

  it('still shows variables when optional card schema labels fail to load', async () => {
    const defaultInvoke = invoke.getMockImplementation()
    invoke.mockImplementation((command, args) => {
      if (command === 'get_card') return Promise.reject(new Error('card unavailable'))
      return defaultInvoke(command, args)
    })

    const wrapper = mount(CampaignVariablesTab, {
      props: { campaignId: 'campaign-1' },
    })
    await flushPromises()

    expect(wrapper.text()).not.toContain('变量加载失败')
    expect(wrapper.text()).toContain('故事时间')
    expect(wrapper.get('[aria-label="全局变量 story_clock"]').element.value).toBe('清晨')
  })
})
