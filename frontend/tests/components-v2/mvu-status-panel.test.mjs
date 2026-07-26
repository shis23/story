import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import MvuStatusPanel from '../../src/components-v2/st/MvuStatusPanel.vue'

const sections = [
  {
    instanceId: 'inst-a',
    instanceName: '江离',
    mvuState: {
      uiBindings: [{ element: 'hp-bar', variable_key: 'hp', display: { kind: 'bar', max: 100 } }],
      variables: [{ key: 'hp', value: 42 }],
      fallbackCount: 0,
    },
  },
  {
    instanceId: 'inst-b',
    instanceName: '沈若萱',
    mvuState: {
      uiBindings: [{ element: 'mood', variable_key: 'mood', display: { kind: 'text' } }],
      variables: [{ key: 'mood', value: '平静' }],
      fallbackCount: 0,
    },
  },
]

describe('MvuStatusPanel', () => {
  it('renders one titled MvuStatusBar per section', () => {
    const wrapper = mount(MvuStatusPanel, { props: { sections } })
    expect(wrapper.find('[data-testid="mvu-status-panel"]').exists()).toBe(true)
    expect(wrapper.text()).toContain('江离')
    expect(wrapper.text()).toContain('沈若萱')
    // MvuStatusBar 真渲染：bar 的数值与 text 的变量值都可见
    expect(wrapper.text()).toContain('42/100')
    expect(wrapper.text()).toContain('平静')
  })

  it('renders nothing when sections are empty', () => {
    const wrapper = mount(MvuStatusPanel, { props: { sections: [] } })
    expect(wrapper.find('[data-testid="mvu-status-panel"]').exists()).toBe(false)
  })

  it('renders interaction buttons and emits interact with the mapping', async () => {
    const interactions = [
      { element_label: '攻击按钮', actions: [{ kind: 'trigger_next_turn', hint: '攻击' }] },
      { element_label: '修炼', actions: [{ kind: 'modify_variable', key: 'exp', value_expr: '+10' }] },
    ]
    const wrapper = mount(MvuStatusPanel, { props: { sections: [], interactions } })
    const buttons = wrapper.findAll('[data-testid="mvu-interaction-bar"] button')
    expect(buttons.length).toBe(2)
    expect(buttons[0].text()).toBe('攻击按钮')

    await buttons[1].trigger('click')
    expect(wrapper.emitted('interact')).toBeTruthy()
    expect(wrapper.emitted('interact')[0][0]).toEqual(interactions[1])
  })

  it('disables interaction buttons while busy', () => {
    const interactions = [{ element_label: '攻击按钮', actions: [] }]
    const wrapper = mount(MvuStatusPanel, { props: { sections: [], interactions, busy: true } })
    const button = wrapper.find('[data-testid="mvu-interaction-bar"] button')
    expect(button.attributes('disabled')).toBeDefined()
  })
})
