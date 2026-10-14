import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import CardShellHost from '../../src/components/CardShellHost.vue'
import { CARD_SHELL_VARIABLES_KEY } from '../../src/utils/cardShellVariableStore.js'

/**
 * 卡壳 TavernHelper setVariables / replaceVariables 的选择器桶持久化。
 *
 * 假后端镜像 Rust 侧写入守卫：`set_campaign_variable` 经
 * `validate_variable_write_key` 拒绝任何 `__` 段（T-06），所以桶键
 * `__storyforge_card_shell_variables` 只能走专用的 `set_card_shell_variables`。
 * 桶写入若仍走通用入口，壳收到的回复带错误、刷新后桶为空。
 */

function createBackend() {
  const variables = new Map()
  const docs = []
  const invoke = vi.fn(async (command, args = {}) => {
    switch (command) {
      case 'get_campaign_variables':
        return [...variables].map(([key, value]) => ({ key, value, last_updated_turn: 0 }))
      case 'set_campaign_variable':
        if (String(args.key).split('.').some((segment) => segment.startsWith('__'))) {
          throw { type: 'validation', message: '变量键名不能使用 __ 内部段' }
        }
        variables.set(args.key, args.value)
        return null
      case 'set_card_shell_variables':
        variables.set(CARD_SHELL_VARIABLES_KEY, args.variables)
        return null
      case 'card_shell_list_allowed_hosts':
        return []
      case 'card_shell_register_doc':
        docs.push(args.html)
        return 'a'.repeat(64)
      default:
        return null
    }
  })
  return { invoke, variables, docs }
}

function bridgeSessionOf(html) {
  const match = /var BRIDGE_SESSION = ("[^"]*");/.exec(html || '')
  expect(match).toBeTruthy()
  return JSON.parse(match[1])
}

async function mountShell() {
  const wrapper = mount(CardShellHost, {
    props: { campaignId: 'campaign-1', html: '<body><div id="app"></div></body>' },
  })
  await flushPromises()
  return wrapper
}

async function askShellVariablesSet(session, id, payload) {
  const replies = []
  const source = { postMessage: (message) => replies.push(message) }
  window.dispatchEvent(new MessageEvent('message', {
    source,
    data: { __sf_shell_bridge: true, shellSession: session, id, type: 'shell_variables_set', payload },
  }))
  await flushPromises()
  return replies
}

describe('CardShellHost selector variable persistence', () => {
  let backend

  beforeEach(() => {
    backend = createBackend()
    const tauriInternals = { invoke: backend.invoke }
    globalThis.__TAURI_INTERNALS__ = tauriInternals
    window.__TAURI_INTERNALS__ = tauriInternals
  })

  afterEach(() => {
    delete globalThis.__TAURI_INTERNALS__
    delete window.__TAURI_INTERNALS__
  })

  it('persists a shell setVariables patch through the dedicated bucket command', async () => {
    const wrapper = await mountShell()
    const session = bridgeSessionOf(backend.docs.at(-1))

    const replies = await askShellVariablesSet(session, 'vars-1', {
      selector: { type: 'chat' },
      variables: { sets: { hp: 3, theme: 'night' }, deletes: [] },
      mode: 'patch',
    })

    expect(replies).toEqual([{ __sf_shell_bridge_res: 'vars-1', result: { ok: true }, error: null }])
    expect(backend.variables.get(CARD_SHELL_VARIABLES_KEY)).toEqual({
      'chat:current': { hp: 3, theme: 'night' },
    })
    expect(backend.invoke).toHaveBeenCalledWith(
      'set_card_shell_variables',
      { campaignId: 'campaign-1', variables: { 'chat:current': { hp: 3, theme: 'night' } } },
      undefined, // @tauri-apps/api/core invoke 的 options 参数
    )
    // 通用入口从不承载内部桶键。
    expect(backend.invoke.mock.calls.some(([command]) => command === 'set_campaign_variable')).toBe(false)
    wrapper.unmount()
  })

  it('a remounted shell starts from the persisted buckets', async () => {
    const first = await mountShell()
    await askShellVariablesSet(bridgeSessionOf(backend.docs.at(-1)), 'vars-2', {
      selector: { type: 'message', message_id: 7 },
      variables: { sets: { mood: 'calm' }, deletes: [] },
      mode: 'patch',
    })
    first.unmount()

    const second = await mountShell()
    expect(backend.docs.at(-1)).toContain('"message:7":{"mood":"calm"}')
    second.unmount()
  })
})
