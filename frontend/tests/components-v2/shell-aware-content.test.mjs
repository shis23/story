import { describe, it, expect, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { computed } from 'vue'
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

  it('does not duplicate an opening shell inside a message when the page owns its URL', () => {
    const wrapper = mount(ShellAwareContent, {
      props: { content: `$('body').load('${HOME}')` },
      global: {
        provide: {
          storyforgeCardShellLayout: {
            suppressedMessageShellUrls: computed(() => [HOME]),
          },
        },
        stubs: {
          CardShellHost: CardShellHostStub,
          RichContent: true,
        },
      },
    })

    expect(wrapper.findComponent(CardShellHostStub).exists()).toBe(false)
  })

  it('keeps a different opening URL mounted by the message', () => {
    const customOpening =
      'https://testingcf.jsdelivr.net/gh/The-poem-of-destiny/FrontEnd-for-destined-journey@1.6.2/dist/custom_start/index.html'
    const wrapper = mount(ShellAwareContent, {
      props: { content: `$('body').load('${HOME}') $('body').load('${customOpening}')` },
      global: {
        provide: {
          storyforgeCardShellLayout: {
            suppressedMessageShellUrls: computed(() => [HOME]),
          },
        },
        stubs: {
          CardShellHost: CardShellHostStub,
          RichContent: true,
        },
      },
    })

    const shellHosts = wrapper.findAllComponents(CardShellHostStub)
    expect(shellHosts).toHaveLength(1)
    expect(shellHosts[0].props('url')).toBe(customOpening)
  })
})
