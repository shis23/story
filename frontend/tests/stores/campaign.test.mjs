import test from 'node:test'
import assert from 'node:assert/strict'
import { createPinia, setActivePinia } from 'pinia'
import { useCampaignStore } from '../../src/stores/campaign.js'
import { useWritingStore } from '../../src/stores/writing.js'

function setup() {
  setActivePinia(createPinia())
  return useCampaignStore()
}

test('campaign store 初始状态为 null/空', () => {
  const s = setup()
  assert.equal(s.activeChar, null)
  assert.equal(s.activeCharDetail, null)
  assert.equal(s.activeCampaign, null)
  assert.deepEqual(s.instanceNameMap, {})
  assert.deepEqual(s.conversationHistory, [])
  assert.equal(s.currentConversationId, null)
})

test('activeCampaign 可赋值并持久', () => {
  const s = setup()
  s.activeCampaign = { id: 'c1', name: '寒渊谜塔' }
  assert.equal(s.activeCampaign.id, 'c1')
  assert.equal(s.activeCampaign.name, '寒渊谜塔')
})

test('instanceNameMap 赋值后可读', () => {
  const s = setup()
  s.instanceNameMap = { inst_1: '艾莉丝', inst_2: '凯尔' }
  assert.equal(s.instanceNameMap.inst_1, '艾莉丝')
  assert.equal(Object.keys(s.instanceNameMap).length, 2)
})

test('currentConversationId 赋值后可读', () => {
  const s = setup()
  s.currentConversationId = 'conv_abc'
  assert.equal(s.currentConversationId, 'conv_abc')
})

test('lastConversationNode 在无消息时返回 null', () => {
  const s = setup()
  const writing = useWritingStore()
  writing.messages = []
  s.currentConversationId = 'conv_x'
  assert.equal(s.lastConversationNode, null)
})

test('lastConversationNode 跨 store 读取 writing.messages', () => {
  setActivePinia(createPinia())
  const campaign = useCampaignStore()
  const writing = useWritingStore()
  // findLastAssistantConversationNode 要求:role==='assistant' + variant 有 provenance
  writing.messages = [
    {
      id: 'msg_9',
      role: 'assistant',
      variants: [{ content: '塔顶的火光...', provenance: { director: {} } }],
      active_variant: 0,
    },
  ]
  campaign.currentConversationId = 'conv_1'
  const node = campaign.lastConversationNode
  assert.ok(node, '应找到 conversation node')
  assert.equal(node.conversation_id, 'conv_1')
  assert.equal(node.node_id, 'msg_9')
})

test('conversationHistory 可承载会话列表', () => {
  const s = setup()
  s.conversationHistory = [
    { id: 'conv_1', title: '第一局' },
    { id: 'conv_2', title: '第二局' },
  ]
  assert.equal(s.conversationHistory.length, 2)
  assert.equal(s.conversationHistory[1].title, '第二局')
})
