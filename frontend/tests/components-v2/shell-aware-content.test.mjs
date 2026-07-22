import { describe, it, expect, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import ShellAwareContent from '../../src/components-v2/st/ShellAwareContent.vue'
import { useCampaignStore } from '../../src/stores/campaign.js'

const HOME =
  'https://testingcf.jsdelivr.net/gh/The-poem-of-destiny/FrontEnd-for-destined-journey@1.6.2/dist/home/index.html'

const CardShellHostStub = {
  name: 'CardShellHost',
  props: ['campaignId', 'url'],
  template: '<article data-shell-host>{{ campaignId }}|{{ url }}</article>',
}

describe('ShellAwareContent campaign context', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('binds message-local card shells to the active Campaign worldbook', () => {
    const campaign = useCampaignStore()
    campaign.activeCampaign = { id: 'campaign-message-shell', name: '消息壳验收' }

    const wrapper = mount(ShellAwareContent, {
      props: { content: `$('body').load('${HOME}')` },
      global: {
        stubs: {
          CardShellHost: CardShellHostStub,
          RichContent: true,
        },
      },
    })

    expect(wrapper.findComponent(CardShellHostStub).props('campaignId')).toBe('campaign-message-shell')
  })
})
