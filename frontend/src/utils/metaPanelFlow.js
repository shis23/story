export function sortHealthIssues(issues) {
  return [...(issues || [])].sort((a, b) => {
    if (a?.severity === b?.severity) return 0
    return a?.severity === 'error' ? -1 : 1
  })
}

export async function loadTypedPatchesWithPreview({
  campaignId,
  patches,
  previewTypedPatch,
}) {
  const next = (patches || []).map((patch) => ({ ...patch }))
  if (!campaignId) return next

  await Promise.all(next.map(async (patch) => {
    try {
      const preview = await previewTypedPatch(patch.id, campaignId)
      patch._stale = preview?.stale || false
      if (preview?.diff !== undefined) patch._previewDiff = preview.diff
    } catch {
      patch._stale = false
    }
  }))

  return next
}

export async function proposeRepairsFlow({
  campaignId,
  proposeCampaignRepairs,
  previewTypedPatch,
}) {
  if (!campaignId) return []
  const patches = await proposeCampaignRepairs(campaignId)
  return loadTypedPatchesWithPreview({
    campaignId,
    patches,
    previewTypedPatch,
  })
}

export async function refreshTypedPatchesFlow({
  campaignId,
  listTypedPatches,
  previewTypedPatch,
}) {
  const patches = await listTypedPatches()
  return loadTypedPatchesWithPreview({
    campaignId,
    patches,
    previewTypedPatch,
  })
}

export async function acceptTypedPatchFlow({
  campaignId,
  patchId,
  patches,
  acceptTypedPatch,
  refreshHealth,
}) {
  await acceptTypedPatch(patchId, campaignId)
  const nextPatches = (patches || []).filter((patch) => patch.id !== patchId)
  if (!refreshHealth) {
    return { patches: nextPatches }
  }

  try {
    const healthIssues = await refreshHealth()
    return {
      patches: nextPatches,
      healthIssues,
    }
  } catch (healthError) {
    return {
      patches: nextPatches,
      healthError,
    }
  }
}

export async function dismissTypedPatchFlow({
  patchId,
  patches,
  dismissTypedPatch,
}) {
  await dismissTypedPatch(patchId)
  return (patches || []).filter((patch) => patch.id !== patchId)
}

export async function explainGenerationFlow({
  lastConversationNode,
  explainGeneration,
}) {
  if (!lastConversationNode) return null
  return explainGeneration(
    lastConversationNode.conversation_id,
    lastConversationNode.node_id,
  )
}
