import { beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import TopBar from '../../src/components-v2/shell/TopBar.vue'
import { useCampaignStore, useUiStore, useWritingStore } from '../../src/stores/index.js'

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

  it('keeps the full title available and uses labelled icon actions', () => {
    const wrapper = mount(TopBar)
    const header = wrapper.get('header')

    expect(header.classes()).toContain('sf-toolbar')
    const titleRail = wrapper.get('[data-topbar-slot="title"]')
    expect(titleRail.classes()).toContain('story-title-rail')
    expect(titleRail.get('[title]').attributes('title')).toBe(useUiStore().pageTitle)

    const summaryButton = wrapper.get('[aria-label="查看总结"]')
    const variableButton = wrapper.get('[aria-label="查看变量"]')
    expect(summaryButton.find('svg').exists()).toBe(true)
    expect(variableButton.find('svg').exists()).toBe(true)
    expect(summaryButton.attributes('title')).toBeTruthy()
    expect(variableButton.attributes('title')).toBeTruthy()
    expect(wrapper.get('[aria-label="过程与调试"]').classes()).toContain('story-topbar-icon')
  })

  it('announces writing without replacing title controls', async () => {
    const wrapper = mount(TopBar)
    const title = wrapper.get('[data-topbar-slot="title"]').element
    useWritingStore().isWriting = true
    await wrapper.vm.$nextTick()
    expect(wrapper.get('[role="status"]').text()).toContain('写作中')
    expect(wrapper.get('[data-testid="writing-activity"]').exists()).toBe(true)
    expect(wrapper.get('[data-topbar-slot="title"]').element).toBe(title)
    useWritingStore().isWriting = false
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-testid="writing-activity"]').exists()).toBe(false)
  })

  it('keeps the sidebar toggle reachable in both desktop states', async () => {
    const wrapper = mount(TopBar, { props: { sidebarDocked: true, sidebarVisible: true } })
    await wrapper.get('[aria-label="收起侧栏"]').trigger('click')
    expect(wrapper.emitted('toggle-sidebar')).toHaveLength(1)
    await wrapper.setProps({ sidebarVisible: false })
    expect(wrapper.get('[aria-label="展开侧栏"]').attributes('aria-expanded')).toBe('false')
    await wrapper.get('[aria-label="展开侧栏"]').trigger('click')
    expect(wrapper.emitted('toggle-sidebar')).toHaveLength(2)
  })
})
