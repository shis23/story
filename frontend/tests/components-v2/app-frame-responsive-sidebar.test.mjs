import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import { defineComponent, h, nextTick, onMounted } from 'vue'
import { mount } from '@vue/test-utils'
import AppFrame from '../../src/design/shell/AppFrame.vue'

let sidebarMounts = 0
const mountedFrames = []

const SidebarProbe = defineComponent({
  name: 'SidebarProbe',
  setup() {
    onMounted(() => { sidebarMounts += 1 })
    return () => h('section', { 'data-testid': 'sidebar-probe' }, 'sidebar runtime')
  },
})

function setViewportWidth(width) {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: width })
}

function mountFrame(sidebarOpen = false) {
  const wrapper = mount(AppFrame, {
    attachTo: document.body,
    props: { sidebarOpen },
    slots: { sidebar: h(SidebarProbe) },
  })
  mountedFrames.push(wrapper)
  return wrapper
}

describe('AppFrame responsive sidebar mounting', () => {
  beforeEach(() => {
    sidebarMounts = 0
  })

  afterEach(() => {
    while (mountedFrames.length) mountedFrames.pop().unmount()
  })

  it('mounts exactly one sidebar runtime when the mobile drawer opens', async () => {
    setViewportWidth(390)
    const wrapper = mountFrame(true)
    await nextTick()

    expect(sidebarMounts).toBe(1)
    expect(wrapper.findAll('[data-testid="sidebar-probe"]')).toHaveLength(1)
    expect(wrapper.get('aside').classes()).toContain('w-[min(100vw,var(--layout-sidebar))]')
  })

  it('keeps one hidden runtime mounted while the mobile drawer is closed', async () => {
    setViewportWidth(390)
    const wrapper = mountFrame(false)
    await nextTick()

    expect(sidebarMounts).toBe(1)
    expect(wrapper.findAll('[data-testid="sidebar-probe"]')).toHaveLength(1)
  })

  it('keeps one persistent sidebar runtime on desktop', async () => {
    setViewportWidth(1280)
    const wrapper = mountFrame(false)
    await nextTick()

    expect(sidebarMounts).toBe(1)
    expect(wrapper.findAll('[data-testid="sidebar-probe"]')).toHaveLength(1)
  })

  it('honors desktop collapse without unmounting the sidebar runtime', async () => {
    setViewportWidth(1280)
    const wrapper = mountFrame(false)
    expect(wrapper.get('aside').isVisible()).toBe(true)
    await wrapper.setProps({ sidebarCollapsed: true })
    expect(wrapper.get('aside').isVisible()).toBe(false)
    expect(wrapper.get('aside').attributes('inert')).toBeDefined()
    await wrapper.setProps({ sidebarCollapsed: false })
    expect(wrapper.get('aside').isVisible()).toBe(true)
    expect(sidebarMounts).toBe(1)
  })

  it('clears a stale mobile drawer when the window becomes narrow again', async () => {
    setViewportWidth(1280)
    const wrapper = mountFrame(true)
    setViewportWidth(480)
    window.dispatchEvent(new Event('resize'))
    await nextTick()
    expect(wrapper.emitted('update:sidebarOpen')).toEqual([[false]])
  })

  it('does not recreate a sidebar runtime while crossing the responsive breakpoint', async () => {
    setViewportWidth(1280)
    const wrapper = mountFrame(false)
    await nextTick()

    setViewportWidth(390)
    window.dispatchEvent(new Event('resize'))
    await nextTick()
    setViewportWidth(1280)
    window.dispatchEvent(new Event('resize'))
    await nextTick()

    expect(sidebarMounts).toBe(1)
    expect(wrapper.findAll('[data-testid="sidebar-probe"]')).toHaveLength(1)
  })
})
