/**
 * 从 Provenance 提取子 Agent 的可重 roll 角色列表。
 *
 * @param {object|null|undefined} provenance
 *   { seed, subagent_results: [{ character_id, display_name, fallback_reason, … }] }
 * @returns {Array<{ id: string, label: string }>}
 *   id = 稳定 character_id；label = display_name || character_id 优先
 */
export function subagentRolesFromProvenance(provenance) {
  if (!provenance?.subagent_results) return []
  return provenance.subagent_results
    .filter(s => s.character_id)
    .map(s => ({
      id: s.character_id,
      label: s.display_name || s.character_id,
    }))
}
