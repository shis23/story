/**
 * Campaign 面板子 tab 名称常量与刷新映射。
 */

/** 所有支持的子 tab 名称 */
export const DETAIL_SUB_TABS = ['instances', 'knowledge', 'tasks', 'summaries']

/** sub-tab 名称 → 映射键（与 CampaignPanel.vue 的 refMap 一致） */
export const SUB_TAB_REFS = {
  instances: 'instancesTabRef',
  knowledge: 'knowledgeTabRef',
  tasks: 'tasksTabRef',
  summaries: 'summariesTabRef',
}

/**
 * 查找指定子 tab 对应的 ref 键名。
 *
 * @param {string} subTabName - 'instances' | 'knowledge' | 'tasks' | 'summaries'
 * @returns {string|null} ref 键名，不认识时返回 null
 */
export function subTabRefKey(subTabName) {
  return SUB_TAB_REFS[subTabName] ?? null
}

/**
 * 刷新指定子 tab。
 *
 * 纯函数：不操作 Vue 组件/模板 ref，只处理映射关系。
 * tabRefs 是一个 { [refKey]: { refresh: function|null } } 映射。
 *
 * @param {string} subTabName
 * @param {object} tabRefs - { instancesTabRef: { refresh }, knowledgeTabRef: { refresh }, … }
 * @returns {{ refreshed: boolean, refKey: string|null }}
 *   refreshed: 是否调用成功
 *   refKey: 实际调用的 ref 键名（用于调试）
 */
export function refreshSubTab(subTabName, tabRefs) {
  const refKey = subTabRefKey(subTabName)
  if (!refKey) return { refreshed: false, refKey: null }
  const tabRef = tabRefs[refKey]
  if (!tabRef || typeof tabRef.refresh !== 'function') {
    return { refreshed: false, refKey }
  }
  tabRef.refresh()
  return { refreshed: true, refKey }
}
