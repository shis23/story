import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import CardShellFloatingStatus from '../../src/components/CardShellFloatingStatus.vue'

describe('CardShellFloatingStatus', () => {
  it('shows only a floating status orb until the reader asks for the full panel', async () => {
    const wrapper = mount(CardShellFloatingStatus, {
      attachTo: document.body,
      slots: { default: '<div data-testid="status-shell">完整状态</div>' },
    })

    const orb = wrapper.get('button[aria-label="打开当前状态"]')
    expect(orb.attributes('aria-expanded')).toBe('false')
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false)
    expect(wrapper.find('[data-testid="status-shell"]').exists()).toBe(false)

    await orb.trigger('click')
    expect(orb.attributes('aria-expanded')).toBe('true')
    const dialog = document.querySelector('[role="dialog"]')
    const statusShell = document.querySelector('[data-testid="status-shell"]')
    const closeButton = document.querySelector('button[aria-label="关闭当前状态"]')
    expect(dialog).toBeTruthy()
    expect(statusShell?.textContent).toBe('完整状态')
    expect(document.activeElement).toBe(closeButton)

    closeButton.click()
    await wrapper.vm.$nextTick()
    expect(document.querySelector('[role="dialog"]')).toBe(null)
    expect(document.activeElement).toBe(orb.element)
    wrapper.unmount()
  })

  it('gives expanded status content the full usable dialog height', async () => {
    const wrapper = mount(CardShellFloatingStatus, {
      attachTo: document.body,
      slots: { default: '<div data-testid="status-shell">状态内容</div>' },
    })

    await wrapper.get('button[aria-label="打开当前状态"]').trigger('click')
    const statusShell = document.querySelector('[data-testid="status-shell"]')

    expect(statusShell?.parentElement?.classList.contains('h-full')).toBe(true)
    wrapper.unmount()
  })
})
