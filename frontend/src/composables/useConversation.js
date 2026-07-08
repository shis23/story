// Conversation composable — 迁移自 App.vue:416-527。
// 负责:把后端 Conversation 应用到前端 messages、加载/删除/打开/新建会话。
//
// 消费:useWritingStore(messages / currentConversationId)、useCampaignStore(activeCampaign /
// activeChar / instanceNameMap)、useUiStore(showHistory)、usePluginStore(via usePluginBridge
// 提供 broadcastChatChanged / broadcastPluginEvent / chatEventPayload)。
// 只 import 不修改:tauri-api.js、plugin-bridge.js。

import {
  listConversations,
  deleteConversation,
  getConversation,
  setActiveCampaign,
  getActiveCampaign,
} from '../tauri-api.js'
import { ST_EVENT_TYPES } from '../plugin-bridge.js'
import { alertDialog, confirmDialog } from '../components/base/BaseDialog.js'
import { useWritingStore } from '../stores/writing.js'
import { useCampaignStore } from '../stores/campaign.js'
import { useUiStore } from '../stores/ui.js'
import { usePluginBridge } from './usePluginBridge.js'

export function useConversation(handlers = {}) {
  const writing = useWritingStore()
  const campaign = useCampaignStore()
  const ui = useUiStore()
  // 广播 / payload 函数来自 usePluginBridge。
  const { broadcastChatChanged, broadcastPluginEvent, chatEventPayload } = usePluginBridge()

  // 范围外依赖(由调用方注入):App.vue 里这些函数属于开场白/实例名映射/角色详情逻辑,
  // 不在本 composable 迁移范围。签名见下方注释。
  //   loadInstanceNameMap()        — 重新拉 listInstances 填充 campaign.instanceNameMap
  //   loadCharDetail(id)           — 拉角色详情(含世界书),写 activeCharDetail + 归一化 greeting
  //   applySelectedOpeningMessage()— 用 selectedGreeting 替换 messages(开场白选择)
  const loadInstanceNameMap = handlers.loadInstanceNameMap
  const loadCharDetail = handlers.loadCharDetail
  const applySelectedOpeningMessage = handlers.applySelectedOpeningMessage

  // applyConversation(App.vue:416-438):node 数据 → messages 数组转换,写入 writingStore.messages。
  // 复用点:恢复对话、重 roll 后刷新、删除后刷新(单一事实源,避免前端臆测 variant 数组)。
  function applyConversation(conv) {
    campaign.currentConversationId = conv.id
    writing.messages = conv.nodes
      // 隐藏「所有 variant 都 Discarded」的 node(删除后该消息整体消失)
      .filter((node) => node.variants.some((v) => v.status !== 'Discarded'))
      .map((node) => {
        const active = node.variants[node.active_variant] || node.variants[0]
        return {
          id: node.id,
          role: active.role === 'User' ? 'user' : 'assistant',
          role_label: active.role === 'User' ? '我' : 'AI',
          active_variant: node.active_variant,
          variants: node.variants.map((v) => ({
            id: v.id,
            content: v.content,
            display_content: v.display_content ?? v.content,
            status: v.status === 'Final' ? 'final' : v.status === 'Discarded' ? 'discarded' : 'draft',
            provenance: v.provenance,
          })),
        }
      })
    broadcastChatChanged('conversation_applied', { conversationId: conv.id })
  }

  // loadConversationHistory(App.vue:441-453):listConversations → campaignStore.conversationHistory。
  async function loadConversationHistory() {
    try {
      const convList = await listConversations()
      if (!convList || convList.length === 0) {
        campaign.conversationHistory = []
        return
      }
      convList.sort((a, b) => b.updated_at.localeCompare(a.updated_at))
      campaign.conversationHistory = convList
    } catch (e) {
      console.error('加载会话历史失败:', e)
    }
  }

  // handleDeleteConversation(App.vue:456-473):deleteConversation + 刷新历史。
  async function handleDeleteConversation(conv, event) {
    if (event) event.stopPropagation()
    const ok = await confirmDialog(`确定删除该会话？会话 ${conv.id?.slice(0, 8)} 的所有消息将被清除。`, { title: '删除确认' })
    if (!ok) return
    try {
      await deleteConversation(conv.id)
      // 若删的是当前会话，清空当前对话
      if (conv.id === campaign.currentConversationId) {
        writing.messages = []
        campaign.currentConversationId = null
        broadcastChatChanged('conversation_deleted', { conversationId: conv.id })
      }
      await loadConversationHistory()
    } catch (e) {
      console.error('删除会话失败:', e)
      await alertDialog('删除会话失败: ' + e)
    }
  }

  // openConversation(App.vue:476-518):getConversation → applyConversation,广播 CHAT_LOADED。
  async function openConversation(convSummary) {
    try {
      const conv = await getConversation(convSummary.id)
      if (!conv) return

      applyConversation(conv)
      campaign.currentConversationId = convSummary.id
      ui.showHistory = false

      // 一 Campaign 一对话:切到该会话的 Campaign
      if (convSummary.campaign_id) {
        try {
          await setActiveCampaign(convSummary.campaign_id)
          campaign.activeCampaign = await getActiveCampaign()
          if (loadInstanceNameMap) await loadInstanceNameMap()
          // role_label 用 Campaign 名
          writing.messages.forEach((m) => {
            if (m.role === 'assistant') m.role_label = campaign.activeCampaign?.name || 'AI'
          })
        } catch (e) { console.error('切换 Campaign 失败:', e) }
      } else if (conv.character_id) {
        // legacy 会话(无 Campaign 绑定):加载关联角色卡
        try {
          const chars = await import('../tauri-api.js').then(m => m.listCharacters())
          const char = chars.find((c) => c.id === conv.character_id)
          if (char) {
            campaign.activeChar = char
            if (loadCharDetail) await loadCharDetail(char.id)
            writing.messages.forEach((m) => {
              if (m.role === 'assistant') m.role_label = char.name
            })
          }
        } catch (e) { console.error('加载关联角色卡失败:', e) }
      }
      broadcastPluginEvent(ST_EVENT_TYPES.CHAT_LOADED, chatEventPayload({
        conversationId: campaign.currentConversationId,
        campaignId: convSummary.campaign_id || null,
        characterId: conv.character_id || null,
      }))
    } catch (e) {
      console.error('打开对话失败:', e)
    }
  }

  // startNewConversation(App.vue:521-527):清空 messages + 切到 write 视图。
  function startNewConversation() {
    writing.messages = []
    campaign.currentConversationId = null
    ui.showHistory = false
    if (applySelectedOpeningMessage) applySelectedOpeningMessage()
    broadcastPluginEvent(ST_EVENT_TYPES.CHAT_LOADED, chatEventPayload({ reason: 'new_conversation' }))
  }

  return {
    applyConversation,
    loadConversationHistory,
    handleDeleteConversation,
    openConversation,
    startNewConversation,
  }
}
