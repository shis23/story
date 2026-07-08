// 来源 App.vue:1008-1017 makeForkCampaignName。
// 纯函数化：base 由调用方传入（useMessageVariants 传 activeCampaign.value?.name || 'Campaign'）。
// 时间戳格式沿用原实现：zh-CN 月/日 时:分。

/**
 * 生成 Campaign 分支名。
 * @param {string} base - 基础名（通常为被分支 Campaign 的 name）；为空时兜底 'Campaign'。
 * @returns {string} `${base} 分支 ${stamp}`，stamp 为 zh-CN 月/日 时:分。
 */
export function makeForkCampaignName(base) {
  const baseName = base || 'Campaign'
  const stamp = new Date().toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  })
  return `${baseName} 分支 ${stamp}`
}
