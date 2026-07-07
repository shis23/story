export function campaignCardDefinitionCount(card) {
  return card?.definition_count ?? card?.character_count ?? 0
}

export function campaignCardExtractionStatus(card) {
  return card?.extraction_status || (card?.extracted ? 'extracted' : 'unknown')
}

export function campaignCardExtractionLabel(card) {
  const status = campaignCardExtractionStatus(card)
  const definitions = campaignCardDefinitionCount(card)
  if (status === 'extracted' && definitions > 0) return '已识别'
  if (status === 'extracted') return '已识别但无角色定义'
  if (status === 'fallback') return '识别失败，已按单角色处理'
  if (definitions > 0) return '历史状态未知'
  return '未识别'
}

export function campaignCardOptionSuffix(card) {
  const label = campaignCardExtractionLabel(card)
  return label === '已识别' ? '' : `（${label}）`
}

export function preferredCampaignCard(cards) {
  if (!Array.isArray(cards) || cards.length === 0) return null
  return (
    cards.find(card =>
      campaignCardExtractionStatus(card) === 'extracted' && campaignCardDefinitionCount(card) > 0
    ) ||
    cards.find(card => campaignCardDefinitionCount(card) > 0) ||
    cards[0]
  )
}
