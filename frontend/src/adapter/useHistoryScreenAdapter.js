/**
 * useHistoryScreenAdapter — design/history → store/composable 接线。
 */
import { computed } from 'vue'
import { useCampaignStore } from '../stores/index.js'

export function useHistoryScreenAdapter(handlers = {}) {
  const campaign = useCampaignStore()

  const screenProps = computed(() => ({
    conversations: campaign.conversationHistory,
    title: '会话历史',
  }))

  const screenEvents = {
    open: (conv) => handlers.openConversation?.(conv),
    delete: (conv, event) => handlers.handleDeleteConversation?.(conv, event),
    'new-campaign': () => handlers.openNewCampaign?.(),
  }

  return { screenProps, screenEvents }
}
