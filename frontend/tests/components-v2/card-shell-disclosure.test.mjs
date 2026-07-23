import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import CardShellDisclosure from '../../src/components/CardShellDisclosure.vue'

describe('CardShellDisclosure', () => {
  it('keeps status content collapsed until the reader explicitly expands it', async () => {
    const wrapper = mount(CardShellDisclosure, {
      props: { label: '当前状态' },
      slots: { default: '<div data-testid="status-shell">状态内容</div>' },
    })

    const toggle = wrapper.get('button')
    expect(toggle.attributes('aria-expanded')).toBe('false')
    expect(wrapper.find('[data-testid="status-shell"]').exists()).toBe(false)

    await toggle.trigger('click')
    expect(toggle.attributes('aria-expanded')).toBe('true')
    expect(wrapper.get('[data-testid="status-shell"]').exists()).toBe(true)

    await toggle.trigger('click')
    expect(wrapper.find('[data-testid="status-shell"]').exists()).toBe(false)
  })
})
