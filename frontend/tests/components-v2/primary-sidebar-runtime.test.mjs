import { describe, it, expect, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import PrimarySidebar from '../../src/components-v2/shell/PrimarySidebar.vue'

describe('PrimarySidebar runtime placement', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('renders diagnostics in the sidebar runtime slot instead of the story flow', () => {
    const wrapper = mount(PrimarySidebar, {
      props: { docked: true },
      slots: {
        runtime: '<section data-testid="tavern-helper-sidebar">TavernHelper 6/6</section>',
      },
    })

    expect(wrapper.get('[data-testid="tavern-helper-sidebar"]').text()).toContain('TavernHelper 6/6')
  })

  it('provides a desktop collapse command separate from navigation closing', async () => {
    const wrapper = mount(PrimarySidebar, { props: { docked: true } })
    await wrapper.get('[aria-label="收起侧栏"]').trigger('click')
    expect(wrapper.emitted('collapse')).toHaveLength(1)
    expect(wrapper.emitted('close')).toBeUndefined()
  })
})
