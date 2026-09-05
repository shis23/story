import { describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import WritingScreen from '../../src/design/writing/WritingScreen.vue'
import MessageItem from '../../src/design/writing/MessageItem.vue'
import { createPinia, setActivePinia } from 'pinia'
import { useMessageVariants } from '../../src/composables/useMessageVariants.js'
import { useWritingScreenAdapter } from '../../src/adapter/useWritingScreenAdapter.js'
import { useCampaignStore, useWritingStore } from '../../src/stores/index.js'

const message = {
  id: 'node-1', role: 'assistant', role_label: 'AI', active_variant: 0,
  variants: [{ id: 'variant-1', content: 'Original', status: 'draft' }],
}
const button = (wrapper, text) => wrapper.findAll('button').find((b) => b.text() === text)

describe('message command contracts', () => {
  it('shows cancellation as stopped instead of completed', () => {
    const wrapper = mount(WritingScreen, {
      props: { messages: [message], pipeline: { state: 'idle', stateLabel: '已停止' } },
    })
    expect(wrapper.get('[data-testid="story-status"]').text()).toBe('已停止')
    expect(wrapper.text()).not.toContain('已完成')
    wrapper.unmount()
  })

  it('passes add-variant from the real component through adapter and composable to IPC', async () => {
    setActivePinia(createPinia())
    const campaign = useCampaignStore()
    const writing = useWritingStore()
    campaign.activeCampaign = { id: 'campaign-1', name: 'Test' }
    campaign.currentConversationId = 'conversation-1'
    writing.messages = structuredClone([message])
    const addVariantApi = vi.fn(async () => 1)
    const handlers = useMessageVariants({ addVariantApi })
    const { screenProps, screenEvents } = useWritingScreenAdapter(handlers)
    const wrapper = mount(WritingScreen, {
      props: { ...screenProps.value, onAddVariant: screenEvents['add-variant'] },
    })
    await button(wrapper, '添变体').trigger('click')
    await flushPromises()
    expect(addVariantApi).toHaveBeenCalledWith('conversation-1', 'node-1', '', null)
    expect(writing.messages[0].active_variant).toBe(1)
    wrapper.unmount()
  })
  it('emits the nodeId required by addVariant', async () => {
    const wrapper = mount(MessageItem, { props: { message } })
    await button(wrapper, '添变体').trigger('click')
    expect(wrapper.emitted('add-variant')).toEqual([[{ nodeId: 'node-1' }]])
    wrapper.unmount()
  })

  it('does not add or select variants after acceptance', async () => {
    const accepted = {
      ...message,
      variants: [{ ...message.variants[0], status: 'final' }, { ...message.variants[0], id: 'old' }],
    }
    const wrapper = mount(MessageItem, { props: { message: accepted } })
    expect(button(wrapper, '添变体').element.disabled).toBe(true)
    await button(wrapper, '添变体').trigger('click')
    const older = wrapper.findAll('button').find((b) => b.text().includes('变体 B'))
    expect(older.element.disabled).toBe(true)
    await older.trigger('click')
    expect(wrapper.emitted('add-variant')).toBeUndefined()
    expect(wrapper.emitted('switch-variant')).toBeUndefined()
    wrapper.unmount()
  })

  it.each([false, 'reject'])('retains unsaved input on %s through the complete presentation chain', async (result) => {
    const saveVariant = vi.fn(async () => {
      if (result === 'reject') throw new Error('Offline')
      return result
    })
    const wrapper = mount(WritingScreen, { props: { messages: [message], saveVariant } })
    await button(wrapper, '编辑').trigger('click')
    await wrapper.get('textarea').setValue('Unsaved')
    await button(wrapper, '保存').trigger('click')
    await flushPromises()
    expect(saveVariant).toHaveBeenCalledWith({ nodeId: 'node-1', newContent: 'Unsaved' })
    expect(wrapper.get('textarea').element.value).toBe('Unsaved')
    wrapper.unmount()
  })

  it('waits for success and prevents duplicate submissions', async () => {
    let finish
    const saveVariant = vi.fn(() => new Promise((resolve) => { finish = resolve }))
    const wrapper = mount(MessageItem, { props: { message, saveVariant } })
    await button(wrapper, '编辑').trigger('click')
    await button(wrapper, '保存').trigger('click')
    expect(wrapper.get('textarea').exists()).toBe(true)
    expect(button(wrapper, '保存').attributes('disabled')).toBeDefined()
    expect(button(wrapper, '取消').attributes('disabled')).toBeDefined()
    finish(true)
    await flushPromises()
    expect(wrapper.find('textarea').exists()).toBe(false)
    expect(saveVariant).toHaveBeenCalledTimes(1)
    wrapper.unmount()
  })

  it('only exposes branching at the committed head', () => {
    const final = { ...message, variants: [{ ...message.variants[0], status: 'final' }] }
    const wrapper = mount(WritingScreen, {
      props: { canBranch: true, messages: [{ ...final, id: 'old' }, final] },
    })
    expect(wrapper.findAll('button').filter((b) => b.text() === '分支')).toHaveLength(1)
    wrapper.unmount()
  })
})
