// 合并 App.vue:314-317 getAssistantRoleLabel 与 86-90 streamingRoleLabel 的重复逻辑。
// 两者本质一致：campaign 模式用 campaign 名，legacy 模式用角色名，兜底 'AI'。
// 纯函数化：writingMode / campaignName / charName 由调用方传入。

/**
 * 计算助手消息的角色标签。
 * @param {'campaign'|'legacy'|'none'} writingMode - 写作模式
 * @param {string} [campaignName] - Campaign 名称（campaign 模式使用）
 * @param {string} [charName] - 角色名（legacy 模式使用）
 * @returns {string} 角色标签，无匹配时兜底 'AI'
 */
export function assistantRoleLabel(writingMode, campaignName, charName) {
  if (writingMode === 'campaign') return campaignName || 'AI'
  if (writingMode === 'legacy') return charName || 'AI'
  return 'AI'
}
