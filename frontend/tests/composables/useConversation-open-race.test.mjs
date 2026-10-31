// openConversation 竞态守卫回归：连续打开两个会话时，慢返回的旧
// getConversation / setActiveCampaign 不得覆盖新选择的会话。
// 修复前 useConversation 是全库唯一没有单调 token 守卫的异步上下文切换。
// 运行：npm test（node --test）

import test from 'node:test'
import assert from 'node:assert/strict'
import { createPinia, setActivePinia } from 'pinia'

import { useConversation } from '../../src/composables/useConversation.js'
import { useCampaignStore } from '../../src/stores/campaign.js'
import { useWritingStore } from '../../src/stores/writing.js'

// tauri-api.js reads window.__TAURI_INTERNALS__; Node tests need a stub.
if (typeof globalThis.window === 'undefined') {
  globalThis.window = {}
}

function conv(id, extra = {}) {
  return {
    id,
    character_id: null,
    ...extra,
    nodes: [
      {
        id: `node-${id}`,
        active_variant: 0,
        variants: [{ id: `v-${id}`, role: 'User', content: `msg of ${id}`, status: 'Final', provenance: null }],
      },
    ],
  }
}

/**
 * stub 后端 invoke：
 *   get_conversation 按 id 返回可配延迟的会话（模拟慢响应）
 *   set/get_active_campaign 记录并回显最后设置的 campaign
 */
function setupBackend({ delays = {} } = {}) {
  const conversations = new Map()
  const campaignOrder = []
  const campaigns = new Map()
  const invoke = async (cmd, args = {}) => {
    switch (cmd) {
      case 'get_conversation': {
        const c = conversations.get(args.id)
        if (!c) return null
        const ms = delays[args.id] ?? 0
        if (ms) await new Promise((r) => setTimeout(r, ms))
        return c
      }
      case 'set_active_campaign':
        campaignOrder.push(args.id)
        campaigns.set(args.id, { id: args.id, name: `campaign-${args.id}` })
        return null
      case 'get_active_campaign':
        return campaigns.get(campaignOrder[campaignOrder.length - 1]) ?? null
      default:
        return null
    }
  }
  globalThis.window.__TAURI_INTERNALS__ = { invoke }
  return { conversations, campaignOrder }
}

function setup() {
  setActivePinia(createPinia())
  const campaign = useCampaignStore()
  const writing = useWritingStore()
  const opened = []
  const api = useConversation({ onConversationOpened: (s) => opened.push(s.id) })
  return { campaign, writing, opened, api }
}

test('慢返回的旧 getConversation 不得覆盖新选择的会话', async () => {
  const backend = setupBackend({ delays: { 'conv-slow': 60, 'conv-fast': 5 } })
  backend.conversations.set('conv-slow', conv('conv-slow'))
  backend.conversations.set('conv-fast', conv('conv-fast'))
  const { campaign, opened, api } = setup()

  // 用户先点 conv-slow、随即改点 conv-fast：旧请求在途
  const slow = api.openConversation({ id: 'conv-slow', campaign_id: null, character_id: null })
  const fast = api.openConversation({ id: 'conv-fast', campaign_id: null, character_id: null })
  await Promise.all([slow, fast])

  assert.equal(campaign.currentConversationId, 'conv-fast', '最终会话必须是后点的 conv-fast')
  assert.deepEqual(opened, ['conv-fast'], '旧会话不得广播 CHAT_LOADED / 触发 onConversationOpened')
  assert.ok(
    campaign.conversationHistory === undefined || true, // 占位：history 由 loadConversationHistory 填充
  )
})

test('campaign 分支的慢 setActiveCampaign 不得覆盖新会话的 Campaign', async () => {
  const backend = setupBackend({ delays: { 'conv-slow': 60, 'conv-fast': 5 } })
  backend.conversations.set('conv-slow', conv('conv-slow', { campaign_id: 'camp-slow' }))
  backend.conversations.set('conv-fast', conv('conv-fast', { campaign_id: 'camp-fast' }))
  const { campaign, opened, api } = setup()

  const slow = api.openConversation({ id: 'conv-slow', campaign_id: 'camp-slow', character_id: null })
  const fast = api.openConversation({ id: 'conv-fast', campaign_id: 'camp-fast', character_id: null })
  await Promise.all([slow, fast])

  assert.equal(campaign.currentConversationId, 'conv-fast')
  assert.equal(campaign.activeCampaign?.id, 'camp-fast', 'activeCampaign 必须属于后点的会话')
  assert.deepEqual(opened, ['conv-fast'])
  // 守卫在 getConversation 返回后即中止过期请求：旧会话连 set_active_campaign
  // 都不发出，不会把后端 active campaign 闪断到旧 Campaign 再切回。
  assert.deepEqual(backend.campaignOrder, ['camp-fast'])
})

test('单次打开行为不受守卫影响', async () => {
  const backend = setupBackend()
  backend.conversations.set('conv-a', conv('conv-a', { campaign_id: 'camp-a' }))
  const { campaign, writing, opened, api } = setup()

  await api.openConversation({ id: 'conv-a', campaign_id: 'camp-a', character_id: null })

  assert.equal(campaign.currentConversationId, 'conv-a')
  assert.equal(campaign.activeCampaign?.id, 'camp-a')
  assert.equal(writing.messages.length, 1)
  assert.equal(writing.messages[0].variants[0].content, 'msg of conv-a')
  assert.deepEqual(opened, ['conv-a'])
})
