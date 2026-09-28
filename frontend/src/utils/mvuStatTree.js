// Campaign 变量 DTO 列表 → 嵌套 stat_data 树（供卡壳 Mvu shim 只读底座）。
//
// 变量键是点记法路径（任务已在解析边界归一，normalizeMvuKey 兜住存量记法），
// 按段展开成嵌套对象。__storyforge* 内部命名空间键（卡壳桶等）不进树。
//
// M-28c 冲突优先级（确定性，只与「键的形状 + 值的形状」有关，任意写入顺序同结果）：
//   1. 沿路径下行的中间段若是标量/数组，让位给子树（更具体的路径优先）。
//   2. 叶子槽位已有子树时子树优先：整体标量/数组赋值被忽略；整体值也是对象时
//      按键递归合并（不整体替换）。这条修掉了「先写 主角.生命值 再写标量 主角
//      会整棵覆盖」的顺序相关行为。
//   3. 同一深度的标量冲突按「后到者覆盖」（两条写入本就是同一变量的两次赋值）。

import { normalizeMvuKey } from './mvuKey.js'

function isRecord(value) {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value)
}

/**
 * 叶子赋值：已展开子树优先于整体值，对象整体值递归合并（M-28c 规则 2）。
 * @param {Record<string, any>} node
 * @param {string} leaf
 * @param {unknown} value
 */
function assignLeaf(node, leaf, value) {
  const next = value === undefined ? null : value
  const existing = node[leaf]
  if (!isRecord(existing)) {
    node[leaf] = next
    return
  }
  if (isRecord(next)) {
    for (const [key, child] of Object.entries(next)) {
      assignLeaf(existing, key, child)
    }
  }
  // 其余情况（整体标量/数组）不覆盖已展开的子树
}

export function buildMvuStatDataTree(campaignVariables) {
  const tree = {}
  for (const item of Array.isArray(campaignVariables) ? campaignVariables : []) {
    const rawKey = typeof item?.key === 'string' ? item.key : ''
    if (!rawKey || rawKey.startsWith('__storyforge')) continue
    const segments = normalizeMvuKey(rawKey).split('.').filter(Boolean)
    if (!segments.length) continue
    let node = tree
    for (const segment of segments.slice(0, -1)) {
      // 标量叶与子树同名冲突：已有标量让位给子树（后写路径更具体）
      if (!isRecord(node[segment])) {
        node[segment] = {}
      }
      node = node[segment]
    }
    assignLeaf(node, segments[segments.length - 1], item?.value)
  }
  return tree
}
