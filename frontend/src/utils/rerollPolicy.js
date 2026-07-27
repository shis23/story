/**
 * Resolve safe partial-replay capabilities for one concrete draft variant.
 * Sequential artifacts are reusable only when both the active selector and
 * the recorded provenance identify the sequential pipeline.
 */
export function rerollPolicy(selectedMode, recordedMode, allowLegacyPartial = false) {
  return {
    editorOnly: Boolean(allowLegacyPartial) && recordedMode === 'big_scene',
    sequentialSuffix:
      !allowLegacyPartial
      && selectedMode === 'sequential_crew'
      && recordedMode === 'sequential_crew',
  }
}
