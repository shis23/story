import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import CampaignWorldInfoTab from '../../src/components-v2/campaign/CampaignWorldInfoTab.vue'

describe('CampaignWorldInfoTab compact layout', () => {
  beforeEach(() => {
    const invoke = vi.fn((command) => {
      if (command === 'list_campaign_world_info') {
        return Promise.resolve({
          campaign_id: 'campaign-1',
          entry_count: 1,
          constant_count: 0,
          selective_count: 1,
          entries: [{
            index: 0,
            route: 'Selective',
            keys: ['eldoria', 'wood', 'forest'],
            content: '{{user}}: "What is Eldoria?"',
            source: 'card_template',
          }],
        })
      }
      return Promise.resolve(null)
    })
    const tauriInternals = { invoke }
    globalThis.__TAURI_INTERNALS__ = tauriInternals
    window.__TAURI_INTERNALS__ = tauriInternals
  })

  afterEach(() => {
    delete globalThis.__TAURI_INTERNALS__
    delete window.__TAURI_INTERNALS__
  })

  it('renders each entry as a full-width ledger row instead of a cramped table', async () => {
    const wrapper = mount(CampaignWorldInfoTab, {
      props: { campaignId: 'campaign-1' },
    })
    await flushPromises()

    const entry = wrapper.get('[data-testid="world-info-entry"]')
    const actions = wrapper.get('[data-testid="world-info-entry-actions"]')

    expect(wrapper.find('table').exists()).toBe(false)
    expect(entry.text()).toContain('Selective')
    expect(entry.text()).toContain('eldoria')
    expect(entry.text()).toContain('What is Eldoria?')
    expect(entry.text()).toContain('卡模板')
    expect(actions.classes()).toContain('whitespace-nowrap')
  })
})
