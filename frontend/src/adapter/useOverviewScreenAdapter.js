import { computed } from 'vue'
import { useCampaignStore } from '../stores/index.js'

export function useOverviewScreenAdapter(handlers = {}) {
  const campaign = useCampaignStore()

  const screenProps = computed(() => {
    const active = campaign.activeCampaign
    let createdAtText = ''
    if (active?.created_at) {
      try {
        createdAtText = new Date(active.created_at).toLocaleDateString()
      } catch {
        createdAtText = String(active.created_at)
      }
    }
    return {
      campaignName: active?.name || '',
      storyClock: active?.story_clock || '',
      createdAtText,
      conversationCount: campaign.conversationHistory?.length || 0,
    }
  })

  const screenEvents = {
    'open-campaign': () => handlers.openCampaign?.(),
    'view-history': () => handlers.viewHistory?.(),
    'new-campaign': () => handlers.openNewCampaign?.(),
    'continue-writing': () => handlers.continueWriting?.(),
  }

  return { screenProps, screenEvents }
}
