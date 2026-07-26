export function findInstanceDefinition(card, instance) {
  const definitionId = instance?.definition_id
  if (!definitionId || !Array.isArray(card?.character_definitions)) return null
  return card.character_definitions.find((definition) => definition?.id === definitionId) || null
}

export function buildInstanceMvuStatusBarProps({ instance, card, translationDetail, variables }) {
  const definition = findInstanceDefinition(card, instance)
  const translation = translationDetail?.translation
  if (!definition || !translation) return null

  const uiBindings = Array.isArray(translation.ui_bindings) ? translation.ui_bindings : []
  const fallbackFragments = Array.isArray(translation.fallback_fragments)
    ? translation.fallback_fragments
    : []

  if (uiBindings.length === 0 && fallbackFragments.length === 0) return null

  return {
    definitionId: definition.id,
    sourceCharacterId: card?.source_character_id || translationDetail?.source_character_id || null,
    // 模板键（{角色名} 段）在本实例节按此名展开取值
    instanceName: instance?.name || '',
    uiBindings,
    variables: Array.isArray(variables) ? variables : [],
    fallbackCount: fallbackFragments.length,
  }
}

export function shouldApplyInstanceMvuLoad({ expandedInstanceId, instanceId, token, currentToken }) {
  return !!instanceId && expandedInstanceId === instanceId && token === currentToken
}

// campaign 级变量打底，实例变量同 key 覆盖（ui_bindings 的 key 两边都可能指）
export function mergeVariablesForMvuStatus(campaignVariables, instanceVariables) {
  const merged = new Map()
  for (const v of Array.isArray(campaignVariables) ? campaignVariables : []) {
    if (v && typeof v.key === 'string') merged.set(v.key, v.value)
  }
  for (const v of Array.isArray(instanceVariables) ? instanceVariables : []) {
    if (v && typeof v.key === 'string') merged.set(v.key, v.value)
  }
  return Array.from(merged, ([key, value]) => ({ key, value }))
}

/**
 * 写作面状态面板装配：每个绑定到卡 definition 的实例一节。
 * 临时实例 / 无 definition / 翻译无可渲染内容的实例被跳过（复用
 * buildInstanceMvuStatusBarProps 的过滤语义）。
 */
export function buildCampaignMvuStatusSections({ card, translationDetail, instances, campaignVariables }) {
  if (!Array.isArray(instances)) return []
  const sections = []
  for (const instance of instances) {
    const mvuState = buildInstanceMvuStatusBarProps({
      instance,
      card,
      translationDetail,
      variables: mergeVariablesForMvuStatus(campaignVariables, instance?.variables),
    })
    if (mvuState) {
      sections.push({
        instanceId: instance.id,
        instanceName: instance.name || '',
        mvuState,
      })
    }
  }
  return sections
}
