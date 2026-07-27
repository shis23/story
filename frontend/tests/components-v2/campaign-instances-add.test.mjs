import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import CampaignInstancesTab from '../../src/components-v2/campaign/CampaignInstancesTab.vue'

const OverlayStub = {
  props: ['show', 'title'],
  template: '<section v-if="show" role="dialog" :aria-label="title"><slot /></section>',
}

describe('CampaignInstancesTab adding characters', () => {
  let invoke

  beforeEach(() => {
    invoke = vi.fn((command) => {
      const values = {
        list_instances: [
          {
            id: 'inst-hero',
            campaign_id: 'campaign-1',
            definition_id: 'def-hero',
            name: '你（主角）',
            role_type: 'protagonist',
            is_temporary: false,
            variables: [],
          },
          {
            id: 'inst-helper',
            campaign_id: 'campaign-1',
            definition_id: 'def-helper',
            name: '索拉莉娅',
            role_type: 'supporting',
            is_temporary: false,
            variables: [],
          },
        ],
        get_campaign: { id: 'campaign-1', card_id: 'card-1' },
        get_card: {
          id: 'card-1',
          source_character_id: 'source-1',
          character_definitions: [
            { id: 'def-hero', name: '你（主角）', role_type: 'protagonist' },
            { id: 'def-helper', name: '索拉莉娅', role_type: 'supporting' },
            {
              id: 'def-extra',
              name: '眼镜女生',
              role_type: 'extra',
              group: '学生',
              persona_prompt: '谨慎而好奇',
            },
          ],
        },
        add_campaign_instance: {
          id: 'inst-new',
          campaign_id: 'campaign-1',
          definition_id: 'def-extra',
          name: '眼镜女生',
          role_type: 'extra',
          is_temporary: false,
          variables: [],
        },
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

  function mountTab() {
    return mount(CampaignInstancesTab, {
      props: { campaignId: 'campaign-1' },
      global: {
        stubs: {
          Overlay: OverlayStub,
          MvuStatusBar: true,
        },
      },
    })
  }

  it('shows a persistent add entry and localized role types', async () => {
    const wrapper = mountTab()
    await flushPromises()

    expect(wrapper.get('button[aria-label="添加角色"]').exists()).toBe(true)
    expect(wrapper.text()).toContain('主角')
    expect(wrapper.text()).toContain('常驻配角')
  })

  it('lists only card definitions not yet in the Campaign and adds the selected one', async () => {
    const wrapper = mountTab()
    await flushPromises()

    await wrapper.get('button[aria-label="添加角色"]').trigger('click')
    await flushPromises()

    const dialog = wrapper.get('[role="dialog"]')
    expect(dialog.text()).toContain('眼镜女生')
    expect(dialog.text()).not.toContain('索拉莉娅')

    await dialog.get('input[value="def-extra"]').setValue()
    await dialog.get('button[aria-label="将所选卡内角色加入本局"]').trigger('click')
    await flushPromises()

    expect(invoke).toHaveBeenCalledWith('add_campaign_instance', {
      campaignId: 'campaign-1',
      definitionId: 'def-extra',
      name: null,
      persona: null,
      behavior: null,
    }, undefined)
  })

  it('creates a custom temporary character with persona and behavior', async () => {
    const wrapper = mountTab()
    await flushPromises()

    await wrapper.get('button[aria-label="添加角色"]').trigger('click')
    await flushPromises()
    await wrapper.get('button[aria-label="切换到新建临时角色"]').trigger('click')
    await wrapper.get('input[aria-label="临时角色名称"]').setValue('渡鸦信使')
    await wrapper.get('textarea[aria-label="临时角色人设"]').setValue('寡言、警觉，披着湿斗篷')
    await wrapper.get('textarea[aria-label="临时角色行为规则"]').setValue('只交付密信，不主动解释来源')
    await wrapper.get('button[aria-label="创建临时角色"]').trigger('click')
    await flushPromises()

    expect(invoke).toHaveBeenCalledWith('add_campaign_instance', {
      campaignId: 'campaign-1',
      definitionId: null,
      name: '渡鸦信使',
      persona: '寡言、警觉，披着湿斗篷',
      behavior: '只交付密信，不主动解释来源',
    }, undefined)
  })
})
