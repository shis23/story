/**
 * Executable plugin / SillyTavern compatibility matrix.
 *
 * Status vocabulary:
 * - implemented: host emits or bridge handles end-to-end
 * - alias: mapped from another pipeline/host event
 * - derived: synthesized from a primary host event
 * - shim: local iframe helper with intentional StoryForge semantics
 * - degraded: present but visibly incomplete vs full ST
 * - intentionally_unsupported: known ST surface, must not silently succeed as full support
 * - noop: declared constant only; no host emit
 */

import { ST_EVENT_TYPES } from '../plugin-bridge.js'

export const SUPPORT = Object.freeze({
  IMPLEMENTED: 'implemented',
  ALIAS: 'alias',
  DERIVED: 'derived',
  SHIM: 'shim',
  DEGRADED: 'degraded',
  INTENTIONALLY_UNSUPPORTED: 'intentionally_unsupported',
  NOOP: 'noop',
})

/** @typedef {'events'|'slash'|'tavernHelper'|'promptHook'|'permission'|'audit'|'pluginHost'} MatrixSurface */

/**
 * @typedef {object} MatrixEntry
 * @property {string} id
 * @property {MatrixSurface} surface
 * @property {string} name
 * @property {string} status
 * @property {string} [mapsFrom]
 * @property {string} [reason]
 * @property {string[]} [requiresPermissions]
 * @property {boolean} [redactsBodyWithoutReadMemory]
 * @property {string} [testHint]
 */

/** @type {MatrixEntry[]} */
export const ST_EVENT_MATRIX = [
  { id: 'evt:APP_READY', surface: 'events', name: ST_EVENT_TYPES.APP_READY, status: SUPPORT.IMPLEMENTED, testHint: 'host_direct' },
  { id: 'evt:CHAT_LOADED', surface: 'events', name: ST_EVENT_TYPES.CHAT_LOADED, status: SUPPORT.IMPLEMENTED, testHint: 'host_direct' },
  { id: 'evt:CHAT_CHANGED', surface: 'events', name: ST_EVENT_TYPES.CHAT_CHANGED, status: SUPPORT.DERIVED, mapsFrom: 'MESSAGE_*|CHAT_LOADED', testHint: 'derived' },
  { id: 'evt:MESSAGE_SENT', surface: 'events', name: ST_EVENT_TYPES.MESSAGE_SENT, status: SUPPORT.IMPLEMENTED, redactsBodyWithoutReadMemory: true },
  { id: 'evt:MESSAGE_RECEIVED', surface: 'events', name: ST_EVENT_TYPES.MESSAGE_RECEIVED, status: SUPPORT.IMPLEMENTED, redactsBodyWithoutReadMemory: true },
  { id: 'evt:MESSAGE_UPDATED', surface: 'events', name: ST_EVENT_TYPES.MESSAGE_UPDATED, status: SUPPORT.IMPLEMENTED, redactsBodyWithoutReadMemory: true },
  { id: 'evt:MESSAGE_DELETED', surface: 'events', name: ST_EVENT_TYPES.MESSAGE_DELETED, status: SUPPORT.IMPLEMENTED, redactsBodyWithoutReadMemory: true },
  { id: 'evt:MESSAGE_SWIPED', surface: 'events', name: ST_EVENT_TYPES.MESSAGE_SWIPED, status: SUPPORT.IMPLEMENTED, redactsBodyWithoutReadMemory: true },
  { id: 'evt:USER_MESSAGE_RENDERED', surface: 'events', name: ST_EVENT_TYPES.USER_MESSAGE_RENDERED, status: SUPPORT.DERIVED, mapsFrom: 'MESSAGE_SENT|MESSAGE_UPDATED(user)' },
  { id: 'evt:CHARACTER_MESSAGE_RENDERED', surface: 'events', name: ST_EVENT_TYPES.CHARACTER_MESSAGE_RENDERED, status: SUPPORT.DERIVED, mapsFrom: 'MESSAGE_RECEIVED|MESSAGE_UPDATED|MESSAGE_SWIPED' },
  { id: 'evt:CHARACTER_LOADED', surface: 'events', name: ST_EVENT_TYPES.CHARACTER_LOADED, status: SUPPORT.IMPLEMENTED },
  { id: 'evt:GENERATION_STARTED', surface: 'events', name: ST_EVENT_TYPES.GENERATION_STARTED, status: SUPPORT.ALIAS, mapsFrom: 'pipeline.started' },
  { id: 'evt:STREAM_TOKEN', surface: 'events', name: ST_EVENT_TYPES.STREAM_TOKEN, status: SUPPORT.ALIAS, mapsFrom: 'pipeline.editor_progress', redactsBodyWithoutReadMemory: true },
  { id: 'evt:GENERATION_ENDED', surface: 'events', name: ST_EVENT_TYPES.GENERATION_ENDED, status: SUPPORT.ALIAS, mapsFrom: 'pipeline.draft_ready' },
  { id: 'evt:GENERATION_STOPPED', surface: 'events', name: ST_EVENT_TYPES.GENERATION_STOPPED, status: SUPPORT.ALIAS, mapsFrom: 'pipeline.error', reason: 'alias_defined; host emit depends on pipeline error path' },
  { id: 'evt:GENERATE_BEFORE_COMBINE_PROMPTS', surface: 'events', name: ST_EVENT_TYPES.GENERATE_BEFORE_COMBINE_PROMPTS, status: SUPPORT.IMPLEMENTED, requiresPermissions: ['ModifyPrompt'] },
  { id: 'evt:CHAT_COMPLETION_PROMPT_READY', surface: 'events', name: ST_EVENT_TYPES.CHAT_COMPLETION_PROMPT_READY, status: SUPPORT.IMPLEMENTED, requiresPermissions: ['ModifyPrompt'] },
  { id: 'evt:GENERATE_AFTER_COMBINE_PROMPTS', surface: 'events', name: ST_EVENT_TYPES.GENERATE_AFTER_COMBINE_PROMPTS, status: SUPPORT.INTENTIONALLY_UNSUPPORTED, reason: 'hook_chain_before_and_ready_only', fallback: 'use_before_and_ready_hooks' },
  { id: 'evt:WORLDINFO_SETTINGS_UPDATED', surface: 'events', name: ST_EVENT_TYPES.WORLDINFO_SETTINGS_UPDATED, status: SUPPORT.NOOP, reason: 'worldinfo_injected_silently', fallback: 'host_injects_worldinfo_without_event' },
  { id: 'evt:WORLDINFO_UPDATED', surface: 'events', name: ST_EVENT_TYPES.WORLDINFO_UPDATED, status: SUPPORT.NOOP, reason: 'worldinfo_injected_silently', fallback: 'host_injects_worldinfo_without_event' },
  { id: 'evt:WORLDINFO_FORCE_ACTIVATE', surface: 'events', name: ST_EVENT_TYPES.WORLDINFO_FORCE_ACTIVATE, status: SUPPORT.INTENTIONALLY_UNSUPPORTED, reason: 'no_force_activate_ui', fallback: 'no_op' },
  { id: 'evt:TOOL_CALLS_PERFORMED', surface: 'events', name: ST_EVENT_TYPES.TOOL_CALLS_PERFORMED, status: SUPPORT.INTENTIONALLY_UNSUPPORTED, reason: 'agent_tools_not_exposed_to_st_plugins', fallback: 'no_op' },
  { id: 'evt:TOOL_CALLS_RENDERED', surface: 'events', name: ST_EVENT_TYPES.TOOL_CALLS_RENDERED, status: SUPPORT.INTENTIONALLY_UNSUPPORTED, reason: 'agent_tools_not_exposed_to_st_plugins', fallback: 'no_op' },
  { id: 'evt:GROUP_UPDATED', surface: 'events', name: ST_EVENT_TYPES.GROUP_UPDATED, status: SUPPORT.INTENTIONALLY_UNSUPPORTED, reason: 'no_group_model', fallback: 'no_op' },
  { id: 'evt:GROUP_MEMBER_DRAFTED', surface: 'events', name: ST_EVENT_TYPES.GROUP_MEMBER_DRAFTED, status: SUPPORT.INTENTIONALLY_UNSUPPORTED, reason: 'no_group_model', fallback: 'no_op' },
  { id: 'evt:GROUP_WRAPPER_FINISHED', surface: 'events', name: ST_EVENT_TYPES.GROUP_WRAPPER_FINISHED, status: SUPPORT.INTENTIONALLY_UNSUPPORTED, reason: 'no_group_model', fallback: 'no_op' },
  { id: 'evt:SETTINGS_LOADED', surface: 'events', name: ST_EVENT_TYPES.SETTINGS_LOADED, status: SUPPORT.NOOP, reason: 'settings_not_broadcast_as_st_events', fallback: 'extension_settings_loaded_shim' },
  { id: 'evt:SETTINGS_UPDATED', surface: 'events', name: ST_EVENT_TYPES.SETTINGS_UPDATED, status: SUPPORT.SHIM, reason: 'emitted_locally_from_saveSettingsDebounced' },
  { id: 'evt:EXTENSION_SETTINGS_LOADED', surface: 'events', name: ST_EVENT_TYPES.EXTENSION_SETTINGS_LOADED, status: SUPPORT.SHIM, reason: 'emitted_from_plugin_storage_load' },
  { id: 'evt:EXTENSIONS_FIRST_LOAD', surface: 'events', name: ST_EVENT_TYPES.EXTENSIONS_FIRST_LOAD, status: SUPPORT.NOOP, reason: 'no_extension_marketplace_lifecycle', fallback: 'no_op' },
]

export const PIPELINE_EVENT_ALIAS_MATRIX = [
  { id: 'alias:started', surface: 'events', name: 'started', status: SUPPORT.ALIAS, mapsFrom: 'pipeline.started→GENERATION_STARTED' },
  { id: 'alias:editor_progress', surface: 'events', name: 'editor_progress', status: SUPPORT.ALIAS, mapsFrom: 'pipeline.editor_progress→STREAM_TOKEN' },
  { id: 'alias:draft_ready', surface: 'events', name: 'draft_ready', status: SUPPORT.ALIAS, mapsFrom: 'pipeline.draft_ready→GENERATION_ENDED' },
  { id: 'alias:committed', surface: 'events', name: 'committed', status: SUPPORT.ALIAS, mapsFrom: 'pipeline.committed→MESSAGE_RECEIVED|CHARACTER_MESSAGE_RENDERED|CHAT_CHANGED', reason: 'terminal_turn_commit_only' },
  { id: 'alias:state_changed_pipeline_committed', surface: 'events', name: 'state_changed{Committed}', status: SUPPORT.INTENTIONALLY_UNSUPPORTED, mapsFrom: 'pipeline.state_changed after append_ai_draft', reason: 'draft_not_user_accept_must_not_fanout_message_events', fallback: 'wait_for_pipeline.committed_or_terminal_turn_markers' },
  { id: 'alias:error', surface: 'events', name: 'error', status: SUPPORT.ALIAS, mapsFrom: 'pipeline.error→GENERATION_STOPPED' },
  { id: 'alias:prompt_hook_request', surface: 'events', name: 'prompt_hook_request', status: SUPPORT.INTENTIONALLY_UNSUPPORTED, reason: 'filtered_from_generic_plugin_broadcast', fallback: 'use_hook_request_channel' },
]

export const SLASH_COMMAND_MATRIX = [
  { id: 'slash:register', surface: 'slash', name: 'registerSlashCommand', status: SUPPORT.IMPLEMENTED },
  { id: 'slash:unregister', surface: 'slash', name: 'unregisterSlashCommand', status: SUPPORT.IMPLEMENTED },
  { id: 'slash:aliases', surface: 'slash', name: 'aliases', status: SUPPORT.IMPLEMENTED },
  { id: 'slash:arg_parse', surface: 'slash', name: 'argument_parsing', status: SUPPORT.IMPLEMENTED },
  { id: 'slash:pipe', surface: 'slash', name: 'pipe_chaining', status: SUPPORT.IMPLEMENTED },
  { id: 'slash:collision', surface: 'slash', name: 'primary_over_alias_collision', status: SUPPORT.IMPLEMENTED },
  { id: 'slash:genraw', surface: 'slash', name: 'genraw', status: SUPPORT.SHIM, reason: 'routes_to_storyforge.llm.generate', fallback: 'llm.generate_with_CallLlm' },
  { id: 'slash:unknown', surface: 'slash', name: 'unknown_command', status: SUPPORT.INTENTIONALLY_UNSUPPORTED, reason: 'must_fail_visibly', fallback: 'unsupported_object_or_pipe_throw' },
]

export const TAVERN_HELPER_MATRIX = [
  { id: 'th:eventOn', surface: 'tavernHelper', name: 'eventOn', status: SUPPORT.IMPLEMENTED },
  { id: 'th:eventEmitAndWait', surface: 'tavernHelper', name: 'eventEmitAndWait', status: SUPPORT.IMPLEMENTED },
  { id: 'th:promptHooks', surface: 'tavernHelper', name: 'promptHooks', status: SUPPORT.IMPLEMENTED, requiresPermissions: ['ModifyPrompt'] },
  { id: 'th:variables', surface: 'tavernHelper', name: 'getVariables/setVariables', status: SUPPORT.SHIM, reason: 'selector_local_or_backend', fallback: 'local_storage_or_backend_variables' },
  { id: 'th:chatMessages', surface: 'tavernHelper', name: 'getChatMessages/setChatMessages', status: SUPPORT.SHIM, reason: 'local_chat_mirror', fallback: 'iframe_local_chat_array' },
  { id: 'th:slash', surface: 'tavernHelper', name: 'registerSlashCommand/triggerSlash', status: SUPPORT.IMPLEMENTED },
  { id: 'th:statusBar', surface: 'tavernHelper', name: 'setStatusBar', status: SUPPORT.IMPLEMENTED },
  { id: 'th:storage', surface: 'tavernHelper', name: 'storageGet/storageSet', status: SUPPORT.IMPLEMENTED },
  { id: 'th:generate', surface: 'tavernHelper', name: 'generate', status: SUPPORT.IMPLEMENTED, requiresPermissions: ['CallLlm'] },
  { id: 'th:saveChat', surface: 'tavernHelper', name: 'saveChat', status: SUPPORT.DEGRADED, reason: 'local_mirror_only_no_host_persist', fallback: 'injectable_host_adapter_or_local_mirror_true' },
  { id: 'th:callGenericPopup', surface: 'tavernHelper', name: 'callGenericPopup', status: SUPPORT.DEGRADED, reason: 'no_ui_returns_default_or_null', fallback: 'defaultValue_or_null_confirm' },
  { id: 'th:getRequestHeaders', surface: 'tavernHelper', name: 'getRequestHeaders', status: SUPPORT.DEGRADED, reason: 'static_json_content_type_only', fallback: 'content_type_json_only_no_auth' },
  { id: 'th:groups', surface: 'tavernHelper', name: 'groups', status: SUPPORT.INTENTIONALLY_UNSUPPORTED, reason: 'no_group_model', fallback: 'empty_groups_array' },
]

export const PROMPT_HOOK_MATRIX = [
  { id: 'hook:order', surface: 'promptHook', name: 'sequential_plugin_order', status: SUPPORT.IMPLEMENTED },
  { id: 'hook:permission', surface: 'promptHook', name: 'ModifyPrompt_gate', status: SUPPORT.IMPLEMENTED, requiresPermissions: ['ModifyPrompt'] },
  { id: 'hook:fail_open', surface: 'promptHook', name: 'fail_open_on_error', status: SUPPORT.IMPLEMENTED, failPolicy: 'fail_open' },
  { id: 'hook:timeout', surface: 'promptHook', name: 'per_plugin_timeout', status: SUPPORT.IMPLEMENTED, reason: 'DEFAULT_PLUGIN_HOOK_TIMEOUT_MS', failPolicy: 'fail_open' },
  { id: 'hook:cancel', surface: 'promptHook', name: 'cancellation', status: SUPPORT.IMPLEMENTED, failPolicy: 'fail_closed' },
  { id: 'hook:duplicate_id', surface: 'promptHook', name: 'duplicate_or_late_response_id', status: SUPPORT.IMPLEMENTED, reason: 'ignored_after_settle' },
  { id: 'hook:mutation_boundary', surface: 'promptHook', name: 'payload_mutation_boundary', status: SUPPORT.IMPLEMENTED, reason: 'intent_prompt_messages_only' },
  { id: 'hook:budget', surface: 'promptHook', name: 'per_plugin_payload_budget', status: SUPPORT.IMPLEMENTED, reason: 'maxPayloadBytesPerPlugin', failPolicy: 'fail_open' },
  { id: 'hook:revocation', surface: 'promptHook', name: 'runtime_permission_revocation', status: SUPPORT.IMPLEMENTED, failPolicy: 'fail_open' },
  { id: 'hook:audit', surface: 'promptHook', name: 'redacted_audit_ring', status: SUPPORT.IMPLEMENTED },
  { id: 'hook:correlation', surface: 'promptHook', name: 'generation_correlation_ids', status: SUPPORT.IMPLEMENTED },
]

export const PERMISSION_MATRIX = [
  { id: 'perm:event_subscription', surface: 'permission', name: 'event_subscriptions_filter', status: SUPPORT.IMPLEMENTED },
  { id: 'perm:read_memory_redaction', surface: 'permission', name: 'body_redaction_without_ReadMemory', status: SUPPORT.IMPLEMENTED, requiresPermissions: ['ReadMemory'], redactsBodyWithoutReadMemory: true },
  { id: 'perm:modify_prompt', surface: 'permission', name: 'ModifyPrompt', status: SUPPORT.IMPLEMENTED },
  { id: 'perm:variables_split', surface: 'permission', name: 'ReadVariables_vs_WriteVariables', status: SUPPORT.IMPLEMENTED },
  { id: 'perm:runtime_revocation', surface: 'permission', name: 'runtime_permission_recheck', status: SUPPORT.IMPLEMENTED },
]

export const AUDIT_MATRIX = [
  { id: 'audit:identity', surface: 'audit', name: 'pluginId_pluginName_event_stage', status: SUPPORT.IMPLEMENTED },
  { id: 'audit:timing', surface: 'audit', name: 'durationMs', status: SUPPORT.IMPLEMENTED },
  { id: 'audit:outcome', surface: 'audit', name: 'status_changedKeys_error_summary', status: SUPPORT.IMPLEMENTED },
  { id: 'audit:no_prompt_body', surface: 'audit', name: 'no_full_prompt_or_messages', status: SUPPORT.IMPLEMENTED },
  { id: 'audit:no_secrets', surface: 'audit', name: 'no_api_keys_or_private_memory', status: SUPPORT.IMPLEMENTED },
  { id: 'audit:export', surface: 'audit', name: 'exportPromptHookAudit', status: SUPPORT.IMPLEMENTED },
  { id: 'audit:query', surface: 'audit', name: 'query_filter_pagination', status: SUPPORT.IMPLEMENTED },
  { id: 'audit:chain', surface: 'audit', name: 'local_integrity_hash_chain', status: SUPPORT.IMPLEMENTED, reason: 'fnv1a_no_trusted_head_not_crypto_seal' },
  { id: 'audit:retention', surface: 'audit', name: 'bounded_retention', status: SUPPORT.IMPLEMENTED },
]

export const PLUGIN_HOST_MATRIX = [
  { id: 'host:mount_unmount', surface: 'pluginHost', name: 'iframe_mount_unmount', status: SUPPORT.IMPLEMENTED },
  { id: 'host:hidden_hook_hosts', surface: 'pluginHost', name: 'hook_host_ready_wait', status: SUPPORT.IMPLEMENTED },
  { id: 'host:status_slots', surface: 'pluginHost', name: 'named_slot_isolation', status: SUPPORT.IMPLEMENTED },
  { id: 'host:message_correlation', surface: 'pluginHost', name: 'event_id_correlation', status: SUPPORT.IMPLEMENTED },
  { id: 'host:mock_ui', surface: 'pluginHost', name: 'browser_ipc_mock', status: SUPPORT.DEGRADED, reason: 'labelled_mock_ui_only', fallback: 'deterministic_mock_host_only' },
]

/** Full matrix used by table-driven tests. */
export const PLUGIN_COMPAT_MATRIX = Object.freeze([
  ...ST_EVENT_MATRIX,
  ...PIPELINE_EVENT_ALIAS_MATRIX,
  ...SLASH_COMMAND_MATRIX,
  ...TAVERN_HELPER_MATRIX,
  ...PROMPT_HOOK_MATRIX,
  ...PERMISSION_MATRIX,
  ...AUDIT_MATRIX,
  ...PLUGIN_HOST_MATRIX,
])

export function listMatrixBySurface(surface) {
  return PLUGIN_COMPAT_MATRIX.filter((entry) => entry.surface === surface)
}

export function findMatrixEntry(id) {
  return PLUGIN_COMPAT_MATRIX.find((entry) => entry.id === id) || null
}

export function matrixEventNames() {
  return ST_EVENT_MATRIX.map((entry) => entry.name)
}

export function assertMatrixCoversStEventTypes(eventTypes = ST_EVENT_TYPES) {
  const covered = new Set(matrixEventNames())
  const missing = Object.values(eventTypes).filter((name) => !covered.has(name))
  return {
    ok: missing.length === 0,
    missing,
    coveredCount: covered.size,
  }
}

export function entriesRequiringExplicitBehavior() {
  return PLUGIN_COMPAT_MATRIX.filter((entry) => (
    entry.status === SUPPORT.INTENTIONALLY_UNSUPPORTED
    || entry.status === SUPPORT.NOOP
    || entry.status === SUPPORT.DEGRADED
  ))
}

/**
 * Every non-supported row must declare an explicit reason (and preferably a
 * fallback). Used by report generation and matrix tests.
 */
export function assertNonSupportedRowsHaveReasons(matrix = PLUGIN_COMPAT_MATRIX) {
  const nonSupported = matrix.filter((entry) => (
    entry.status === SUPPORT.INTENTIONALLY_UNSUPPORTED
    || entry.status === SUPPORT.NOOP
    || entry.status === SUPPORT.DEGRADED
  ))
  const missing = nonSupported.filter((entry) => !entry.reason)
  return {
    ok: missing.length === 0,
    missing: missing.map((entry) => entry.id),
    total: nonSupported.length,
  }
}

export function countByStatus(matrix = PLUGIN_COMPAT_MATRIX) {
  const counts = {
    [SUPPORT.IMPLEMENTED]: 0,
    [SUPPORT.ALIAS]: 0,
    [SUPPORT.DERIVED]: 0,
    [SUPPORT.SHIM]: 0,
    [SUPPORT.DEGRADED]: 0,
    [SUPPORT.INTENTIONALLY_UNSUPPORTED]: 0,
    [SUPPORT.NOOP]: 0,
  }
  for (const entry of matrix) {
    counts[entry.status] = (counts[entry.status] || 0) + 1
  }
  return counts
}

export function countBySurface(matrix = PLUGIN_COMPAT_MATRIX) {
  const counts = {}
  for (const entry of matrix) {
    counts[entry.surface] = (counts[entry.surface] || 0) + 1
  }
  return counts
}

export function supportedEventEntries() {
  return ST_EVENT_MATRIX.filter((entry) => (
    entry.status === SUPPORT.IMPLEMENTED
    || entry.status === SUPPORT.ALIAS
    || entry.status === SUPPORT.DERIVED
    || entry.status === SUPPORT.SHIM
  ))
}

/**
 * Classify a slash outcome for matrix assertions.
 * Unknown commands must not look like a successful no-op.
 */
export function classifySlashOutcome(result, error = null) {
  if (error) {
    return {
      ok: false,
      visibleFailure: true,
      kind: 'error',
      message: String(error?.message || error),
    }
  }
  if (result && typeof result === 'object' && result.unsupported === true) {
    return {
      ok: false,
      visibleFailure: true,
      kind: 'unsupported',
      message: String(result.reason || result.message || 'unsupported'),
    }
  }
  if (result === undefined) {
    return {
      ok: false,
      visibleFailure: false,
      kind: 'silent_undefined',
      message: 'unknown slash returned undefined',
    }
  }
  return {
    ok: true,
    visibleFailure: false,
    kind: 'value',
    message: '',
  }
}

/**
 * Classify degraded TavernHelper helpers so silent success is detectable.
 */
export function classifyDegradedHelperResult(name, result) {
  if (result && typeof result === 'object' && result.degraded === true) {
    return {
      visible: true,
      name,
      reason: String(result.reason || 'degraded'),
    }
  }
  return {
    visible: false,
    name,
    reason: 'missing_degraded_marker',
  }
}

export function auditRecordLooksSafe(record) {
  const json = JSON.stringify(record ?? {})
  const banned = [
    'api_key',
    'apiKey',
    'Authorization',
    'SF_SECRET_',
    'private prompt body',
    'private message body',
    'sk-',
  ]
  const hits = banned.filter((token) => json.includes(token))
  const hasSummaryShape = !record
    || record.inputSummary === undefined
    || (record.inputSummary && typeof record.inputSummary === 'object')
  return {
    ok: hits.length === 0 && hasSummaryShape,
    hits,
  }
}
