import { beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import TopBar from '../../src/components-v2/shell/TopBar.vue'
import { useCampaignStore, useUiStore } from '../../src/stores/index.js'

describe('TopBar Campaign state shortcuts', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    const campaign = useCampaignStore()
    campaign.activeCampaign = {
      id: 'campaign-1',
      name: 'Eldoria',
      story_clock: '清晨',
    }
  })

  it('opens the Campaign panel directly on summaries', async () => {
    const ui = useUiStore()
    const wrapper = mount(TopBar)

    expect(wrapper.get('[aria-label="故事状态"]').classes()).not.toContain('hidden')
    await wrapper.get('[aria-label="查看总结"]').trigger('click')

    expect(ui.showCampaignPanel).toBe(true)
    expect(ui.campaignPanelTab).toBe('summaries')
  })

  it('opens the Campaign panel directly on variables', async () => {
    const ui = useUiStore()
    const wrapper = mount(TopBar)

    await wrapper.get('[aria-label="查看变量"]').trigger('click')

    expect(ui.showCampaignPanel).toBe(true)
    expect(ui.campaignPanelTab).toBe('variables')
  })

  it('keeps the title on a balanced center rail and uses responsive icon actions', () => {
    const wrapper = mount(TopBar)
    const header = wrapper.get('header')

    expect(header.classes()).toContain('grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)]')
    const titleRail = wrapper.get('[data-topbar-slot="title"]')
    expect(titleRail.classes()).toContain('story-title-rail')

    const summaryButton = wrapper.get('[aria-label="查看总结"]')
    const variableButton = wrapper.get('[aria-label="查看变量"]')
    expect(summaryButton.find('svg').exists()).toBe(true)
    expect(variableButton.find('svg').exists()).toBe(true)
    expect(summaryButton.get('span').classes()).toContain('xs:inline')
    expect(wrapper.get('[aria-label="过程与调试"]').classes()).toContain('story-topbar-icon')
  })
})
