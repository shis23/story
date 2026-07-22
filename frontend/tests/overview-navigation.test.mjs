import test from 'node:test'
import assert from 'node:assert/strict'
import { findLatestCampaignConversation } from '../src/utils/overviewNavigation.js'

test('selects the most recently updated conversation for the active campaign', () => {
  const conversations = [
    { id: 'other', campaign_id: 'campaign-b', updated_at: '2026-07-23T10:00:00Z' },
    { id: 'older', campaign_id: 'campaign-a', updated_at: '2026-07-23T08:00:00Z' },
    { id: 'latest', campaign_id: 'campaign-a', updated_at: '2026-07-23T12:00:00Z' },
  ]

  assert.equal(findLatestCampaignConversation(conversations, 'campaign-a')?.id, 'latest')
})

test('returns null when the active campaign has no conversation to restore', () => {
  assert.equal(findLatestCampaignConversation([], 'campaign-a'), null)
  assert.equal(findLatestCampaignConversation([{ id: 'other', campaign_id: 'campaign-b' }], 'campaign-a'), null)
})
