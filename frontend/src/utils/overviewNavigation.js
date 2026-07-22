/**
 * Return the most recently updated conversation belonging to a Campaign.
 * The overview action must restore this conversation rather than presenting a
 * blank writing surface while the Campaign already has story state.
 *
 * @param {Array<{campaign_id?: string, updated_at?: string}>} conversations
 * @param {string | null | undefined} campaignId
 * @returns {object | null}
 */
export function findLatestCampaignConversation(conversations, campaignId) {
  if (!campaignId || !Array.isArray(conversations)) return null

  return conversations
    .filter((conversation) => conversation?.campaign_id === campaignId)
    .reduce((latest, conversation) => {
      if (!latest) return conversation
      return String(conversation.updated_at || '') > String(latest.updated_at || '')
        ? conversation
        : latest
    }, null)
}
