import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import CardShellHost from '../../src/components/CardShellHost.vue'
import { registerShellDraftTarget } from '../../src/utils/shellChatDraft.js'

/**
 * 卡壳 createChatMessages → 桥 `chat_draft`：宿主只把文字交给写作输入框，回复壳并在
 * 状态行提示「看过后点发送」；输入框不收时把原因作为错误回给壳，不发起写作。
 */

function createBackend() {
  const docs = []
  const invoke = vi.fn(async (command, args = {}) => {
    switch (command) {
      case 'get_campaign_variables':
      case 'card_shell_list_allowed_hosts':
        return []
      case 'card_shell_register_doc':
        docs.push(args.html)
        return 'a'.repeat(64)
      default:
        return null
    }
  })
  return { invoke, docs }
}

function bridgeSessionOf(html) {
  const match = /var BRIDGE_SESSION = ("[^"]*");/.exec(html || '')
  expect(match).toBeTruthy()
  return JSON.parse(match[1])
}

async function askChatDraft(session, id, text) {
  const replies = []
  const source = { postMessage: (message) => replies.push(message) }
  window.dispatchEvent(new MessageEvent('message', {
    source,
    data: { __sf_shell_bridge: true, shellSession: session, id, type: 'chat_draft', payload: { text } },
  }))
  await flushPromises()
  return replies
}

describe('CardShellHost chat drafts', () => {
  let backend
  let unregister = null

  beforeEach(() => {
    backend = createBackend()
    const tauriInternals = { invoke: backend.invoke }
    globalThis.__TAURI_INTERNALS__ = tauriInternals
    window.__TAURI_INTERNALS__ = tauriInternals
  })

  afterEach(() => {
    unregister?.()
    unregister = null
    delete globalThis.__TAURI_INTERNALS__
    delete window.__TAURI_INTERNALS__
  })

  it('hands the text to the composer and tells the user to send it, even with the status line hidden', async () => {
    const filled = []
    unregister = registerShellDraftTarget((text) => {
      filled.push(text)
      return 'drafted'
    })
    const wrapper = mount(CardShellHost, {
      props: { campaignId: 'campaign-1', html: '<body><div id="app"></div></body>', label: '自定义开局', showStatusLine: false },
    })
    await flushPromises()

    const replies = await askChatDraft(bridgeSessionOf(backend.docs.at(-1)), 'draft-1', '姓名：艾琳')
    expect(replies).toEqual([{ __sf_shell_bridge_res: 'draft-1', result: { ok: true, drafted: true }, error: null }])
    expect(filled).toEqual(['姓名：艾琳'])
    expect(wrapper.text()).toContain('自定义开局把一段消息填进了写作输入框，看过后点发送')
    // 只填草稿：没有任何写作命令。
    expect(backend.invoke.mock.calls.some(([command]) => /writ|turn/i.test(command))).toBe(false)
    wrapper.unmount()
  })

  it('returns the reason to the shell when the composer keeps its own draft', async () => {
    unregister = registerShellDraftTarget(() => 'draft')
    const wrapper = mount(CardShellHost, {
      props: { campaignId: 'campaign-1', html: '<body><div id="app"></div></body>' },
    })
    await flushPromises()

    const replies = await askChatDraft(bridgeSessionOf(backend.docs.at(-1)), 'draft-2', '姓名：艾琳')
    expect(replies).toHaveLength(1)
    expect(replies[0].result).toBeNull()
    expect(replies[0].error).toContain('没有覆盖')
    expect(wrapper.text()).not.toContain('看过后点发送')
    wrapper.unmount()
  })
})
