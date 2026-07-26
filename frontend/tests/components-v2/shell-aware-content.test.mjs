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

describe('ShellAwareContent in-place segmentation', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  const childKinds = (wrapper) => Array.from(wrapper.element.children).map((el) => {
    const testid = el.getAttribute && el.getAttribute('data-testid')
    if (testid) return testid
    return el.tagName.toLowerCase()
  })

  it('renders narrative and shells interleaved in original order', () => {
    const doc = '<body><div id="app"></div><script>battle()</script></body>'
    const wrapper = mountWithLayout(
      `叙事A。\n$('body').load('${HOME}')\n叙事B。\n${doc}\n叙事C。`,
      { trusted: [HOME], inlineTriggers: [{ label: '战斗', trigger: '【战斗】' }] },
      { sourceContent: '【战斗】回合开始' },
    )

    // 文本、.load 壳、文本、内联壳、文本——原文顺序
    expect(childKinds(wrapper)).toEqual([
      'rich-content-stub', 'article', 'rich-content-stub', 'article', 'rich-content-stub',
    ])
    const hosts = wrapper.findAllComponents(CardShellHostStub)
    expect(hosts[0].props('url')).toBe(HOME)
    expect(hosts[1].props('html')).toContain('battle()')
  })

  it('confirm card holds the shell position and approve swaps in place', async () => {
    const attacker = 'https://files.catbox.moe/attacker.html'
    const wrapper = mountWithLayout(`A\n$('body').load('${attacker}')\nB`, { trusted: [HOME] })

    expect(childKinds(wrapper)).toEqual([
      'rich-content-stub', 'shell-mount-confirm', 'rich-content-stub',
    ])

    await wrapper.find('[data-testid="shell-mount-approve"]').trigger('click')
    expect(childKinds(wrapper)).toEqual([
      'rich-content-stub', 'article', 'rich-content-stub',
    ])
    expect(wrapper.findComponent(CardShellHostStub).props('url')).toBe(attacker)
  })

  it('suppressed urls leave no shell at their position, surrounding text intact', () => {
    const wrapper = mountWithLayout(
      `A\n$('body').load('${HOME}')\nB`,
      { suppressed: [HOME], trusted: [HOME] },
    )
    expect(childKinds(wrapper)).toEqual(['rich-content-stub', 'rich-content-stub'])
  })

  it('plain messages render as a single untouched text segment', () => {
    const wrapper = mountWithLayout('普通叙事。\n\n\n\n空行保留。', {})
    expect(childKinds(wrapper)).toEqual(['rich-content-stub'])
  })
})
