export function metaToolResultKind(result) {
  return typeof result?.kind === 'string' ? result.kind : ''
}

export function hasMetaToolResult(message) {
  const kind = metaToolResultKind(message?.tool_result)
  return kind !== '' && kind !== 'none'
}

export function worldInfoReportFromToolResult(result) {
  return metaToolResultKind(result) === 'world_info_report' ? result : null
}

export function cardReportFromToolResult(result) {
  return metaToolResultKind(result) === 'card_report' ? result : null
}

export function patchProposalFromToolResult(result) {
  return metaToolResultKind(result) === 'patch_proposed' ? result : null
}
