import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import NewCampaignForm from '../../src/components-v2/campaign/NewCampaignForm.vue'

const OverlayStub = { template: '<section><slot /></section>' }
const ButtonStub = {
  props: ['disabled', 'loading'],
  emits: ['click'],
  template: '<button :disabled="disabled || loading" @click="$emit(\'click\')"><slot /></button>',
}
const InputStub = {
  props: ['modelValue'],
  emits: ['update:modelValue'],
  template: '<input :value="modelValue" @input="$emit(\'update:modelValue\', $event.target.value)" />',
}
const SelectStub = { template: '<div />' }

describe('NewCampaignForm opening handoff', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    const tauriInternals = {
      invoke(command) {
        const values = {
          list_cards: [{ id: 'card-1', name: '测试卡', extraction_status: 'complete', definitions_count: 1 }],
          get_card: { id: 'card-1', name: '测试卡', first_mes: '第一条消息' },
          create_campaign: { id: 'campaign-1', conversation_id: 'conversation-1' },
          get_active_campaign: { id: 'campaign-1', name: '新活动' },
          get_conversation: { id: 'conversation-1', campaign_id: 'campaign-1', nodes: [] },
        }
        return Promise.resolve(values[command] ?? null)
      },
    }
    globalThis.__TAURI_INTERNALS__ = tauriInternals
    window.__TAURI_INTERNALS__ = tauriInternals
  })

  afterEach(() => {
    delete globalThis.__TAURI_INTERNALS__
    delete window.__TAURI_INTERNALS__
  })

  it('arms the opening shell when a new Campaign creates its first conversation', async () => {
    const openingShellStarted = vi.fn()
    const wrapper = mount(NewCampaignForm, {
      props: {
        show: false,
        openingShellStarted,
        loadInstanceNameMap: vi.fn(),
        applyConversation: vi.fn(),
        broadcastPluginEvent: vi.fn(),
        loadConversationHistory: vi.fn(),
      },
      global: {
        stubs: {
          Overlay: OverlayStub,
          Button: ButtonStub,
          Input: InputStub,
          Select: SelectStub,
        },
      },
    })

    await wrapper.setProps({ show: true })
    await flushPromises()
    await wrapper.get('input').setValue('新测试')
    await wrapper.findAll('button').at(-1).trigger('click')
    await flushPromises()

    expect(openingShellStarted).toHaveBeenCalledWith('conversation-1')
    wrapper.unmount()
  })
})
