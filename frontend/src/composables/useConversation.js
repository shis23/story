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
import { errorText } from '../utils/errorText.js'

export function useConversation(handlers = {}) {
  const writing = useWritingStore()
  const campaign = useCampaignStore()
  const ui = useUiStore()
  // 广播 / payload 函数来自 usePluginBridge。AppV2 注入自身实例，使
  // prompt hook 的 generation/cancel 记账与写作路径共享同一份状态；
  // 独立调用方（测试）保留自建实例的回退。
  const { broadcastChatChanged, broadcastPluginEvent, chatEventPayload } =
    handlers.pluginBridge || usePluginBridge()

  // 范围外依赖(由调用方注入):App.vue 里这些函数属于开场白/实例名映射/角色详情逻辑,
  // 不在本 composable 迁移范围。签名见下方注释。
  //   loadInstanceNameMap()        — 重新拉 listInstances 填充 campaign.instanceNameMap
  //   loadCharDetail(id)           — 拉角色详情(含世界书),写 activeCharDetail + 归一化 greeting
  //   applySelectedOpeningMessage()— 用 selectedGreeting 替换 messages(开场白选择)
  const loadInstanceNameMap = handlers.loadInstanceNameMap
  const loadCharDetail = handlers.loadCharDetail
  const applySelectedOpeningMessage = handlers.applySelectedOpeningMessage
  const onConversationOpened = handlers.onConversationOpened || (() => {})

  // applyConversation(App.vue:416-438):node 数据 → messages 数组转换,写入 writingStore.messages。
  // 复用点:恢复对话、重 roll 后刷新、删除后刷新(单一事实源,避免前端臆测 variant 数组)。
  function applyConversation(conv) {
    campaign.currentConversationId = conv.id
    // 换会话后旧会话的采纳小票/质量态不得滞留（MessageItem 按 nodeId 匹配，
    // 同 id 复用时会把上一局的收据显示到新会话消息上）
    writing.pendingReceipt = null
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

  // handleDeleteConversation：一活动一对话 → 删除整局活动（会话+实例/知识/任务/总结）。
  async function handleDeleteConversation(conv, event) {
    if (event) event.stopPropagation()
    const label = conv.card_name || conv.name || conv.id?.slice?.(0, 8) || '这局故事'
    const ok = await confirmDialog(
      `确定删除「${label}」整局活动？\n\n将同时删除：对话正文、角色实例、知识、任务与总结。此操作不可恢复。`,
      { title: '删除整局活动' },
    )
    if (!ok) return
    try {
      await deleteConversation(conv.id)
      // 若删的是当前会话 / 当前活动，清空写作态
      if (conv.id === campaign.currentConversationId) {
        writing.messages = []
        campaign.currentConversationId = null
        broadcastChatChanged('conversation_deleted', { conversationId: conv.id })
      }
      if (conv.campaign_id && campaign.activeCampaign?.id === conv.campaign_id) {
        campaign.activeCampaign = null
      }
      await loadConversationHistory()
      try {
        campaign.activeCampaign = await getActiveCampaign()
      } catch {
        // 活跃活动可能已删，忽略
      }
    } catch (e) {
      console.error('删除活动失败:', e)
      await alertDialog('删除活动失败: ' + errorText(e))
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
      // 上下文切换完成后再回调（ disarm 开场壳 / 回填质量报告等都需要
      // activeCampaign 已同步为新会话的 Campaign，放在前面会读到旧值）
      onConversationOpened(convSummary)
    } catch (e) {
      console.error('打开对话失败:', e)
      await alertDialog('打开对话失败: ' + errorText(e))
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
