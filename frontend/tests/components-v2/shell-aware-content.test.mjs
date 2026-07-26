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

function mountWithLayout(content, layout = {}) {
  return mount(ShellAwareContent, {
    props: { content },
    global: {
      provide: {
        storyforgeCardShellLayout: {
          suppressedMessageShellUrls: computed(() => layout.suppressed || []),
          trustedMessageShellUrls: computed(() => layout.trusted || []),
        },
      },
      stubs: {
        CardShellHost: CardShellHostStub,
        RichContent: true,
      },
    },
  })
}

describe('ShellAwareContent campaign context', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('binds message-local card shells to the active Campaign worldbook', () => {
    const campaign = useCampaignStore()
    campaign.activeCampaign = { id: 'campaign-message-shell', name: '消息壳验收' }

    const wrapper = mountWithLayout(`$('body').load('${HOME}')`, { trusted: [HOME] })

    expect(wrapper.findComponent(CardShellHostStub).props('campaignId')).toBe('campaign-message-shell')
  })

  it('does not duplicate an opening shell inside a message when the page owns its URL', () => {
    const wrapper = mountWithLayout(`$('body').load('${HOME}')`, {
      suppressed: [HOME],
      trusted: [HOME],
    })

    expect(wrapper.findComponent(CardShellHostStub).exists()).toBe(false)
  })

  it('keeps a different opening URL mounted by the message', () => {
    const customOpening =
      'https://testingcf.jsdelivr.net/gh/The-poem-of-destiny/FrontEnd-for-destined-journey@1.6.2/dist/custom_start/index.html'
    const wrapper = mountWithLayout(
      `$('body').load('${HOME}') $('body').load('${customOpening}')`,
      { suppressed: [HOME], trusted: [HOME, customOpening] },
    )

    const shellHosts = wrapper.findAllComponents(CardShellHostStub)
    expect(shellHosts).toHaveLength(1)
    expect(shellHosts[0].props('url')).toBe(customOpening)
  })
})

describe('ShellAwareContent mount trust gate (H3)', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  const ATTACKER = 'https://files.catbox.moe/attacker.html'

  it('gates unregistered .load urls behind an explicit confirmation', async () => {
    const wrapper = mountWithLayout(`$('body').load('${ATTACKER}')`, { trusted: [HOME] })

    // 未注册 URL：不挂载，出确认卡
    expect(wrapper.findComponent(CardShellHostStub).exists()).toBe(false)
    const confirm = wrapper.find('[data-testid="shell-mount-confirm"]')
    expect(confirm.exists()).toBe(true)
    expect(confirm.text()).toContain(ATTACKER)

    // 用户显式放行 → 挂载
    await wrapper.find('[data-testid="shell-mount-approve"]').trigger('click')
    expect(wrapper.findComponent(CardShellHostStub).props('url')).toBe(ATTACKER)
    expect(wrapper.find('[data-testid="shell-mount-confirm"]').exists()).toBe(false)
  })

  it('fails closed when no layout/trust context is provided', () => {
    const wrapper = mount(ShellAwareContent, {
      props: { content: `$('body').load('${HOME}')` },
      global: {
        stubs: {
          CardShellHost: CardShellHostStub,
          RichContent: true,
        },
      },
    })

    expect(wrapper.findComponent(CardShellHostStub).exists()).toBe(false)
    expect(wrapper.find('[data-testid="shell-mount-confirm"]').exists()).toBe(true)
  })
})
