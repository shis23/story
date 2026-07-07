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
    uiBindings,
    variables: Array.isArray(variables) ? variables : [],
    fallbackCount: fallbackFragments.length,
  }
}

export function shouldApplyInstanceMvuLoad({ expandedInstanceId, instanceId, token, currentToken }) {
  return !!instanceId && expandedInstanceId === instanceId && token === currentToken
}
