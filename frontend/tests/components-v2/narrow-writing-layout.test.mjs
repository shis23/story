import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import MessageItem from '../../src/design/writing/MessageItem.vue'

describe('MessageItem narrow-window layout', () => {
  it('keeps assistant metadata and actions in one non-wrapping footer row', () => {
    const wrapper = mount(MessageItem, {
      props: {
        canBranch: true,
        message: {
          id: 'message-1',
          role: 'assistant',
          role_label: 'test1',
          active_variant: 0,
          variants: [{
            content: '正文',
            status: 'final',
            provenance: { seed: 1785148239109713700 },
          }],
        },
      },
    })

    const footer = wrapper.get('[data-testid="message-footer"]')
    const actions = wrapper.get('[data-testid="message-actions"]')

    expect(footer.classes()).toContain('flex-nowrap')
    expect(footer.classes()).not.toContain('overflow-hidden')
    expect(actions.classes()).toContain('whitespace-nowrap')
    expect(actions.classes()).toContain('shrink-0')
    expect(actions.text()).toContain('编辑')
    expect(actions.text()).toContain('已采纳')
    expect(actions.text()).toContain('重 roll')
    expect(actions.text()).toContain('添变体')
    expect(actions.text()).toContain('分支')
    expect(actions.text()).toContain('删除')
  })
})
