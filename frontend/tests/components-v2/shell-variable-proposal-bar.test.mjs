import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import ShellVariableProposalBar from '../../src/components-v2/st/ShellVariableProposalBar.vue'

const proposals = [
  { id: 'svp_1', key: 'hp', value: 10, source: 'card-shell' },
  { id: 'svp_2', key: 'mood', value: '愤怒', source: 'card-shell' },
]

describe('ShellVariableProposalBar', () => {
  it('renders nothing without proposals', () => {
    const wrapper = mount(ShellVariableProposalBar, { props: { proposals: [] } })
    expect(wrapper.find('[data-testid="shell-var-proposal-bar"]').exists()).toBe(false)
  })

  it('lists pending proposals with key and value preview', () => {
    const wrapper = mount(ShellVariableProposalBar, { props: { proposals } })
    expect(wrapper.text()).toContain('卡片请求写入 2 个变量')
    const rows = wrapper.findAll('[data-testid="shell-var-proposal-row"]')
    expect(rows.length).toBe(2)
    expect(rows[0].text()).toContain('hp')
    expect(rows[0].text()).toContain('10')
    expect(rows[1].text()).toContain('愤怒')
  })

  it('emits apply/reject with the proposal id', async () => {
    const wrapper = mount(ShellVariableProposalBar, { props: { proposals } })
    await wrapper.find('[data-testid="shell-var-apply-svp_1"]').trigger('click')
    expect(wrapper.emitted('apply')[0]).toEqual(['svp_1'])
    await wrapper.find('[data-testid="shell-var-reject-svp_2"]').trigger('click')
    expect(wrapper.emitted('reject')[0]).toEqual(['svp_2'])
  })

  it('emits apply-all / reject-all and disables while busy', async () => {
    const wrapper = mount(ShellVariableProposalBar, { props: { proposals } })
    await wrapper.find('[data-testid="shell-var-apply-all"]').trigger('click')
    expect(wrapper.emitted('apply-all')).toBeTruthy()
    await wrapper.find('[data-testid="shell-var-reject-all"]').trigger('click')
    expect(wrapper.emitted('reject-all')).toBeTruthy()

    const busy = mount(ShellVariableProposalBar, { props: { proposals, busy: true } })
    expect(busy.find('[data-testid="shell-var-apply-all"]').attributes('disabled')).toBeDefined()
    expect(busy.find('[data-testid="shell-var-apply-svp_1"]').attributes('disabled')).toBeDefined()
  })
})
