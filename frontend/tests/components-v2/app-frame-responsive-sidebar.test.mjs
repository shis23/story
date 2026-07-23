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
