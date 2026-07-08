// 迁移自 App.vue:347-385 的开场白选择逻辑。
// 消费 writingStore: greetingOptions / selectedGreeting / selectedGreetingIndex / messages。
// 消费 campaignStore: activeCharDetail / activeChar（用于 role_label）。
//
// 范围外依赖（注入）：
// - broadcastChatChanged(reason, extra)：插件事件广播（App.vue 侧持有）
// - scrollToBottom()：消息容器自动滚动（App.vue 侧持有）
// 二者不在本次迁移范围，由调用方经 options 传入；未传时安全降级为 no-op。

import { useWritingStore } from '../stores/writing.js'
import { useCampaignStore } from '../stores/campaign.js'
import { assistantRoleLabel } from '../utils/roleLabel.js'

/**
 * @param {{
 *   broadcastChatChanged?: (reason: string, extra?: object) => void,
 *   scrollToBottom?: () => void,
 * }} [options]
 */
export function useGreeting(options = {}) {
  const writingStore = useWritingStore()
  const campaignStore = useCampaignStore()

  const broadcastChatChanged = options.broadcastChatChanged || (() => {})
  const scrollToBottom = options.scrollToBottom || (() => {})

  // 来源 App.vue:314-317 getAssistantRoleLabel（legacy 模式下用角色名）。
  // buildOpeningMessage 里优先用 activeCharDetail.name，否则走 assistantRoleLabel 兜底。
  function getAssistantRoleLabel() {
    return assistantRoleLabel(
      writingStore.writingMode,
      campaignStore.activeCampaign?.name,
      campaignStore.activeChar?.name,
    )
  }

  // 来源 App.vue:347-351
  function normalizeGreetingSelection() {
    if (writingStore.selectedGreetingIndex >= writingStore.greetingOptions.length) {
      writingStore.selectedGreetingIndex = 0
    }
  }

  // 来源 App.vue:353-367
  function buildOpeningMessage(content) {
    return {
      id: 'm1',
      role: 'assistant',
      role_label: campaignStore.activeCharDetail?.name || getAssistantRoleLabel(),
      active_variant: 0,
      variants: [{
        id: 'v1',
        content,
        display_content: content,
        status: 'final',
        provenance: null,
      }],
    }
  }

  // 来源 App.vue:369-380
  function applySelectedOpeningMessage() {
    normalizeGreetingSelection()
    if (writingStore.writingMode !== 'legacy') {
      writingStore.messages = []
      broadcastChatChanged('opening_message_cleared')
      return
    }
    const content = writingStore.selectedGreeting?.content
    writingStore.messages = content ? [buildOpeningMessage(content)] : []
    broadcastChatChanged('opening_message_selected', { greetingIndex: writingStore.selectedGreetingIndex })
    scrollToBottom()
  }

  // 来源 App.vue:382-385
  function selectGreeting(index) {
    writingStore.selectedGreetingIndex = index
    applySelectedOpeningMessage()
  }

  return {
    normalizeGreetingSelection,
    buildOpeningMessage,
    applySelectedOpeningMessage,
    selectGreeting,
  }
}
