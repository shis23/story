import test from 'node:test'
import assert from 'node:assert/strict'
import {
  campaignCardDefinitionCount,
  campaignCardExtractionLabel,
  campaignCardOptionSuffix,
  campaignCardExtractionStatus,
  preferredCampaignCard,
} from '../src/utils/campaignCardStatus.js'

test('normalizes campaign card extraction status and definition counts', () => {
  assert.equal(campaignCardDefinitionCount({ definition_count: 2, character_count: 1 }), 2)
  assert.equal(campaignCardDefinitionCount({ character_count: 1 }), 1)
  assert.equal(campaignCardDefinitionCount(null), 0)

  assert.equal(campaignCardExtractionStatus({ extraction_status: 'fallback', extracted: true }), 'fallback')
  assert.equal(campaignCardExtractionStatus({ extracted: true }), 'extracted')
  assert.equal(campaignCardExtractionStatus({ extracted: false }), 'unknown')
})

test('labels fallback and legacy usable cards without treating them as unrecognized', () => {
  assert.equal(campaignCardExtractionLabel({ extraction_status: 'extracted', definition_count: 1 }), '已识别')
  assert.equal(campaignCardExtractionLabel({ extraction_status: 'extracted', definition_count: 0 }), '已识别但无角色定义')
  assert.equal(campaignCardExtractionLabel({ extraction_status: 'fallback', definition_count: 1 }), '识别失败，已按单角色处理')
  assert.equal(campaignCardExtractionLabel({ extraction_status: 'unknown', definition_count: 1 }), '历史状态未知')
  assert.equal(campaignCardExtractionLabel({ extraction_status: 'unknown', definition_count: 0 }), '未识别')

  assert.equal(campaignCardOptionSuffix({ extraction_status: 'extracted', definition_count: 1 }), '')
  assert.equal(campaignCardOptionSuffix({ extraction_status: 'extracted', definition_count: 0 }), '（已识别但无角色定义）')
  assert.equal(campaignCardOptionSuffix({ extraction_status: 'fallback', definition_count: 1 }), '（识别失败，已按单角色处理）')
  assert.equal(campaignCardOptionSuffix({ extraction_status: 'unknown', definition_count: 1 }), '（历史状态未知）')
  assert.equal(campaignCardOptionSuffix({ extraction_status: 'unknown', definition_count: 0 }), '（未识别）')
})

test('prefers extracted cards, then any card with usable definitions, then the first card', () => {
  const empty = { id: 'empty', extraction_status: 'unknown', definition_count: 0 }
  const emptyExtracted = { id: 'empty-extracted', extraction_status: 'extracted', definition_count: 0 }
  const fallback = { id: 'fallback', extraction_status: 'fallback', definition_count: 1 }
  const legacyUsable = { id: 'legacy', extraction_status: 'unknown', character_count: 1 }
  const extracted = { id: 'extracted', extraction_status: 'extracted', definition_count: 2 }

  assert.equal(preferredCampaignCard([empty, fallback, extracted])?.id, 'extracted')
  assert.equal(preferredCampaignCard([emptyExtracted, fallback])?.id, 'fallback')
  assert.equal(preferredCampaignCard([empty, fallback, legacyUsable])?.id, 'fallback')
  assert.equal(preferredCampaignCard([empty, legacyUsable])?.id, 'legacy')
  assert.equal(preferredCampaignCard([empty])?.id, 'empty')
  assert.equal(preferredCampaignCard([]), null)
  assert.equal(preferredCampaignCard(null), null)
})
