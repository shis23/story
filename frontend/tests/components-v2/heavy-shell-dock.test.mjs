/**
 * L7-A HeavyShellDock：默认收起（零挂载）、展开即挂载、一次一个活跃、
 * 清单变化收起。TavernHelperRuntime 用 stub 替换（不模拟 iframe 执行）。
 */
import { describe, it, expect, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import HeavyShellDock from '../../src/components-v2/st/HeavyShellDock.vue'

const RuntimeStub = {
  name: 'TavernHelperRuntime',
  props: ['shells', 'characterId', 'visible', 'showStatus', 'autoRun', 'placement'],
  template: '<div class="th-runtime-stub">{{ shells?.[0]?.label }}</div>',
}

function heavyShell(label, byteLen = 60_000) {
  return {
    kind: 'tavern_helper_module',
    label,
    entry: { inline_js: { js: '', deferred: true, byte_len: byteLen } },
  }
}

function mountDock(shells) {
  return mount(HeavyShellDock, {
    props: { shells, characterId: 'char-1' },
    global: { stubs: { TavernHelperRuntime: RuntimeStub } },
  })
}

describe('HeavyShellDock', () => {
  let wrapper
  afterEach(() => {
    wrapper?.unmount()
    wrapper = null
  })

  it('renders nothing without heavy shells', () => {
    wrapper = mountDock([])
    expect(wrapper.find('.heavy-shell-dock').exists()).toBe(false)
  })

  it('collapsed by default: chips only, no runtime mounted', () => {
    wrapper = mountDock([heavyShell('bgm 播放器'), heavyShell('图鉴')])
    expect(wrapper.text()).toContain('bgm 播放器')
    expect(wrapper.text()).toContain('图鉴')
    expect(wrapper.find('.th-runtime-stub').exists()).toBe(false)
  })

  it('expands on click (mounts runtime with only that shell) and toggles off', async () => {
    wrapper = mountDock([heavyShell('bgm 播放器'), heavyShell('图鉴')])
    const chip = wrapper.findAll('button').find((b) => b.text().includes('bgm 播放器'))
    await chip.trigger('click')
    const stub = wrapper.findComponent(RuntimeStub)
    expect(stub.exists()).toBe(true)
    expect(stub.props('shells')).toHaveLength(1)
    expect(stub.props('shells')[0].label).toBe('bgm 播放器')
    expect(stub.props('visible')).toBe(true)

    // 再点同一 chip 收起
    await chip.trigger('click')
    expect(wrapper.find('.th-runtime-stub').exists()).toBe(false)
  })

  it('one active at a time: expanding another chip replaces the mounted app', async () => {
    wrapper = mountDock([heavyShell('bgm 播放器'), heavyShell('图鉴')])
    await wrapper
      .findAll('button')
      .find((b) => b.text().includes('bgm 播放器'))
      .trigger('click')
    await wrapper
      .findAll('button')
      .find((b) => b.text().includes('图鉴'))
      .trigger('click')
    const stubs = wrapper.findAllComponents(RuntimeStub)
    expect(stubs).toHaveLength(1)
    expect(stubs[0].props('shells')[0].label).toBe('图鉴')
  })

  it('collapses when the shell list changes (card/campaign switch)', async () => {
    wrapper = mountDock([heavyShell('bgm 播放器')])
    await wrapper
      .findAll('button')
      .find((b) => b.text().includes('bgm 播放器'))
      .trigger('click')
    expect(wrapper.find('.th-runtime-stub').exists()).toBe(true)
    await wrapper.setProps({ shells: [heavyShell('新卡应用')] })
    expect(wrapper.find('.th-runtime-stub').exists()).toBe(false)
  })
})
