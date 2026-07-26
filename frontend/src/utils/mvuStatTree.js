// Campaign 变量 DTO 列表 → 嵌套 stat_data 树（供卡壳 Mvu shim 只读底座）。
//
// 变量键是点记法路径（任务已在解析边界归一，normalizeMvuKey 兜住存量记法），
// 按段展开成嵌套对象。__storyforge* 内部命名空间键（卡壳桶等）不进树。

import { normalizeMvuKey } from './mvuKey.js'

function isRecord(value) {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value)
}

export function buildMvuStatDataTree(campaignVariables) {
  const tree = {}
  for (const item of Array.isArray(campaignVariables) ? campaignVariables : []) {
    const rawKey = typeof item?.key === 'string' ? item.key : ''
    if (!rawKey || rawKey.startsWith('__storyforge')) continue
    const segments = normalizeMvuKey(rawKey).split('.').filter(Boolean)
    if (!segments.length) continue
    let node = tree
    let blocked = false
    for (const segment of segments.slice(0, -1)) {
      if (!isRecord(node[segment])) {
        // 标量叶与子树同名冲突：已有标量让位给子树（后写路径更具体）
        node[segment] = {}
      }
      node = node[segment]
      if (!isRecord(node)) {
        blocked = true
        break
      }
    }
    if (blocked) continue
    const leaf = segments[segments.length - 1]
    const value = item?.value
    node[leaf] = value === undefined ? null : value
  }
  return tree
}
