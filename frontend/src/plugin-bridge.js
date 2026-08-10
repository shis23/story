/**
 * M4 插件 postMessage 桥接协议
 *
 * 宿主侧（PluginHost.vue）监听 iframe message → 权限校验 → 调 Tauri → 回传结果
 * 插件侧：通过注入的 window.storyforge 对象调用 API，底层是 postMessage
 */

// ─── 协议常量 ──────────────────────────────────────────────────────────────

export const MSG_REQUEST = 'sf:api:request'
export const MSG_RESPONSE = 'sf:api:response'
export const MSG_EVENT = 'sf:api:event'
export const MSG_HOOK_REQUEST = 'sf:hook:request'
export const MSG_HOOK_RESPONSE = 'sf:hook:response'
export const MSG_MOUNT = 'sf:ui:mount'
export const DEFAULT_PLUGIN_HOOK_TIMEOUT_MS = 5000
export const PROMPT_HOOK_PERMISSION = 'ModifyPrompt'
export const READ_MEMORY_PERMISSION = 'ReadMemory'

// Host-side persistence adapters. Default import is a no-op until a host wires
// a real adapter; this keeps `chat.save` deterministic and out of tauri-app.
import {
  applyPersistenceAdapters as applyAdapters,
  createDefaultSaveChatAdapter,
  classifySaveChatResult,
  PERSISTENCE_DEGRADED_REASON,
} from './utils/pluginPersistence.js'

export const ST_EVENT_TYPES = Object.freeze({
  APP_READY: 'APP_READY',
  CHAT_CHANGED: 'CHAT_CHANGED',
  CHAT_LOADED: 'CHAT_LOADED',
  MESSAGE_RECEIVED: 'MESSAGE_RECEIVED',
  MESSAGE_SENT: 'MESSAGE_SENT',
  MESSAGE_UPDATED: 'MESSAGE_UPDATED',
  MESSAGE_DELETED: 'MESSAGE_DELETED',
  MESSAGE_SWIPED: 'MESSAGE_SWIPED',
  GENERATION_STARTED: 'GENERATION_STARTED',
  GENERATION_STOPPED: 'GENERATION_STOPPED',
  GENERATION_ENDED: 'GENERATION_ENDED',
  STREAM_TOKEN: 'STREAM_TOKEN',
  CHARACTER_LOADED: 'CHARACTER_LOADED',
  CHARACTER_MESSAGE_RENDERED: 'CHARACTER_MESSAGE_RENDERED',
  USER_MESSAGE_RENDERED: 'USER_MESSAGE_RENDERED',
  WORLDINFO_SETTINGS_UPDATED: 'WORLDINFO_SETTINGS_UPDATED',
  WORLDINFO_UPDATED: 'WORLDINFO_UPDATED',
  WORLDINFO_FORCE_ACTIVATE: 'WORLDINFO_FORCE_ACTIVATE',
  GENERATE_BEFORE_COMBINE_PROMPTS: 'GENERATE_BEFORE_COMBINE_PROMPTS',
  GENERATE_AFTER_COMBINE_PROMPTS: 'GENERATE_AFTER_COMBINE_PROMPTS',
  CHAT_COMPLETION_PROMPT_READY: 'CHAT_COMPLETION_PROMPT_READY',
  TOOL_CALLS_PERFORMED: 'TOOL_CALLS_PERFORMED',
  TOOL_CALLS_RENDERED: 'TOOL_CALLS_RENDERED',
  GROUP_UPDATED: 'GROUP_UPDATED',
  GROUP_MEMBER_DRAFTED: 'GROUP_MEMBER_DRAFTED',
  GROUP_WRAPPER_FINISHED: 'GROUP_WRAPPER_FINISHED',
  SETTINGS_LOADED: 'SETTINGS_LOADED',
  SETTINGS_UPDATED: 'SETTINGS_UPDATED',
  EXTENSION_SETTINGS_LOADED: 'EXTENSION_SETTINGS_LOADED',
  EXTENSIONS_FIRST_LOAD: 'EXTENSIONS_FIRST_LOAD',
})

const ST_EVENT_ALIASES = {
  started: ['GENERATION_STARTED'],
  editor_progress: ['STREAM_TOKEN'],
  draft_ready: ['GENERATION_ENDED'],
  committed: ['MESSAGE_RECEIVED', 'CHARACTER_MESSAGE_RENDERED', 'CHAT_CHANGED'],
  error: ['GENERATION_STOPPED'],
}

function uniqueEventNames(names) {
  const seen = new Set()
  return names.filter((name) => {
    if (!name || seen.has(name)) return false
    seen.add(name)
    return true
  })
}

function deriveSillyTavernHostEventNames(eventName, data = {}) {
  const names = [eventName]
  const role = data?.role
  if (eventName === ST_EVENT_TYPES.MESSAGE_SENT) {
    names.push(ST_EVENT_TYPES.USER_MESSAGE_RENDERED, ST_EVENT_TYPES.CHAT_CHANGED)
  } else if (eventName === ST_EVENT_TYPES.MESSAGE_RECEIVED) {
    names.push(ST_EVENT_TYPES.CHARACTER_MESSAGE_RENDERED, ST_EVENT_TYPES.CHAT_CHANGED)
  } else if (eventName === ST_EVENT_TYPES.MESSAGE_UPDATED) {
    names.push(role === 'user' ? ST_EVENT_TYPES.USER_MESSAGE_RENDERED : ST_EVENT_TYPES.CHARACTER_MESSAGE_RENDERED)
    names.push(ST_EVENT_TYPES.CHAT_CHANGED)
  } else if (eventName === ST_EVENT_TYPES.MESSAGE_SWIPED) {
    names.push(ST_EVENT_TYPES.CHARACTER_MESSAGE_RENDERED, ST_EVENT_TYPES.CHAT_CHANGED)
  } else if (eventName === ST_EVENT_TYPES.MESSAGE_DELETED || eventName === ST_EVENT_TYPES.CHAT_LOADED) {
    names.push(ST_EVENT_TYPES.CHAT_CHANGED)
  }
  return uniqueEventNames(names)
}

const HOST_PLUGIN_STORAGE_FALLBACK = new Map()
const HOST_SAVE_CHAT_REQUESTS_BY_ADAPTER = new WeakMap()
const MAX_DEDUPED_SAVE_CHAT_REQUESTS = 128

function saveChatRequestsForAdapter(adapter) {
  let requests = HOST_SAVE_CHAT_REQUESTS_BY_ADAPTER.get(adapter)
  if (!requests) {
    requests = new Map()
    HOST_SAVE_CHAT_REQUESTS_BY_ADAPTER.set(adapter, requests)
  }
  return requests
}

function hostPluginStorageKey(pluginId, key) {
  return [
    'sf_host_plugin_storage',
    encodeURIComponent(String(pluginId)),
    encodeURIComponent(String(key)),
  ].join(':')
}

function legacyHostPluginStorageKey(pluginId, key) {
  return `sf_host_plugin_storage_${pluginId}_${key}`
}

function readHostPluginStorageByKey(storageKey) {
  try {
    if (typeof globalThis !== 'undefined' && globalThis.localStorage) {
      const raw = globalThis.localStorage.getItem(storageKey)
      return raw === null ? null : JSON.parse(raw)
    }
  } catch {
    // Fall back to process-local storage below.
  }
  return HOST_PLUGIN_STORAGE_FALLBACK.has(storageKey)
    ? HOST_PLUGIN_STORAGE_FALLBACK.get(storageKey)
    : null
}

function readHostPluginStorage(pluginId, key) {
  const storageKey = hostPluginStorageKey(pluginId, key)
  const value = readHostPluginStorageByKey(storageKey)
  if (value !== null) return value

  const legacyValue = readHostPluginStorageByKey(legacyHostPluginStorageKey(pluginId, key))
  if (legacyValue !== null) {
    writeHostPluginStorage(pluginId, key, legacyValue)
  }
  return legacyValue
}

function writeHostPluginStorage(pluginId, key, value) {
  const storageKey = hostPluginStorageKey(pluginId, key)
  try {
    if (typeof globalThis !== 'undefined' && globalThis.localStorage) {
      globalThis.localStorage.setItem(storageKey, JSON.stringify(value))
    }
  } catch {
    // Keep the fallback map updated even when host localStorage is unavailable.
  }
  HOST_PLUGIN_STORAGE_FALLBACK.set(storageKey, value)
}

// A bounded host-local key for idempotent chat persistence. It never leaves the
// host or logs the chat body. Repeating the exact snapshot after an iframe
// timeout joins the original in-flight persistence request instead of writing
// it twice.
function saveChatSnapshotKey(pluginId, chat) {
  let snapshot = ''
  try {
    snapshot = JSON.stringify(chat ?? [])
  } catch {
    snapshot = '[unserializable-chat]'
  }
  let hash = 2166136261
  for (let index = 0; index < snapshot.length; index += 1) {
    hash ^= snapshot.charCodeAt(index)
    hash = Math.imul(hash, 16777619)
  }
  return `${String(pluginId)}:${(hash >>> 0).toString(16)}:${snapshot.length}`
}

// ─── 方法 → 权限 + Tauri 命令映射 ──────────────────────────────────────────

export const API_METHODS = {
  'character.list':   { permission: 'ReadCharacters',  command: 'plugin_list_characters', params: (_p, pluginId) => ({ pluginId }) },
  'character.get':    { permission: 'ReadCharacters',  command: 'plugin_read_character',  params: (p, pluginId) => ({ pluginId, characterId: p.id }) },
  'worldInfo.search': { permission: 'ReadWorldInfo',   command: 'plugin_read_world_info', params: (p, pluginId) => ({ pluginId, characterId: p.characterId }) },
  // L4 严格门禁：走插件通道命令，后端 PluginRegistry 二次校验 ReadMemory。
  // 未注册的插件（含卡壳虚拟插件）后端直接拒绝，前端权限数组不再是唯一边界。
  'memory.getRecent': { permission: 'ReadMemory',      command: 'plugin_get_conversation',  params: (p, pluginId) => ({ pluginId, id: p.conversationId }) },
  // Gate 8 审查 P2-C1: 读=读、写=写——仅持 WriteVariables 的插件不得读变量。
  'variables.get':    { permission: 'ReadVariables',     command: 'plugin_get_variable', params: (p, pluginId) => ({ pluginId, campaignId: p.campaignId, instanceId: p.instanceId }) },
  'variables.set':    { permission: 'WriteVariables',  command: 'plugin_set_variable', params: (p, pluginId) => ({ pluginId, campaignId: p.campaignId, instanceId: p.instanceId, key: p.key, value: p.value }) },
  'storage.get':      { permission: null,              command: null },  // 本地 localStorage，不走后端
  'storage.set':      { permission: null,              command: null },
  // Host-side adapter routes (no backend command). Defaults are degraded shims;
  // production can inject real adapters without editing tauri-app storage code.
  'chat.save':        { permission: null,              command: null },
  'ui.popup':         { permission: null,              command: null },
  'ui.requestHeaders':{ permission: null,              command: null },
  // Gate 8 复评：后端 start_writing 的 on_event 是必填 Channel，插件沙箱
  // 无法提供事件通道——该 API 在生产必然失败（Tauri 参数反序列化缺必填
  // Channel 直接报错）。显式标记 unsupported，返回清晰错误而非 invoke 一个
  // 注定失败的调用（旧测试曾固化「只传 intent」的错误契约）。
  'llm.generate':     { permission: 'CallLlm', command: null, unsupported: 'llm.generate 不支持：start_writing 需要宿主注入 onEvent 事件通道，插件通道无法提供；请使用宿主写作流程' },
}

function requiredPermissions(method) {
  if (Array.isArray(method.permissions)) return method.permissions
  return method.permission ? [method.permission] : []
}

function hasAnyPermission(plugin, permissions) {
  if (!permissions.length) return true
  return permissions.some((permission) => plugin.permissions?.includes(permission))
}

export function canModifyPrompt(plugin) {
  return hasAnyPermission(plugin, [PROMPT_HOOK_PERMISSION])
}

export function canReadMemory(plugin) {
  return !plugin || hasAnyPermission(plugin, [READ_MEMORY_PERMISSION])
}

function pluginEventSubscriptions(plugin) {
  const subscriptions = plugin?.event_subscriptions || plugin?.manifest?.event_subscriptions || []
  return Array.isArray(subscriptions)
    ? subscriptions.map((event) => String(event).trim()).filter(Boolean)
    : []
}

function isSubscribedToPluginEvent(plugin, eventName) {
  if (!plugin) return true
  const subscriptions = pluginEventSubscriptions(plugin)
  if (!subscriptions.length) return false
  return subscriptions.includes('*') || subscriptions.includes(eventName)
}

const SENSITIVE_EVENT_FIELDS = new Set([
  'content',
  'displaycontent',
  'text',
  'token',
  'delta',
  'messages',
  'prompt',
  'intent',
  'raw',
  // Body-like / diagnostic fields that can carry private text without ReadMemory.
  'message',
  'errormessage',
  'error',
  'stack',
  'stderr',
  'stdout',
  'detail',
  'details',
  'body',
  'payload',
  'responsebody',
  'apikey',
  'authorization',
  'password',
  'secret',
  'credential',
  'privatememory',
])

function isSensitiveEventField(key) {
  const normalized = String(key || '').replace(/[^a-z0-9]/gi, '').toLowerCase()
  return SENSITIVE_EVENT_FIELDS.has(normalized)
}

function sanitizePluginEventData(value) {
  if (Array.isArray(value)) {
    return value.map((item) => sanitizePluginEventData(item))
  }
  if (!value || typeof value !== 'object') {
    return value
  }

  const sanitized = {}
  for (const [key, child] of Object.entries(value)) {
    if (isSensitiveEventField(key)) continue
    sanitized[key] = sanitizePluginEventData(child)
  }
  return sanitized
}

function eventPayloadOptions(plugin) {
  return {
    includeSensitive: !plugin || canReadMemory(plugin),
  }
}

// ─── PipelineEvent → 插件事件映射 ─────────────────────────────────────────

function createPluginEventPayload(pipelineEvent, options = {}) {
  const includeSensitive = options.includeSensitive !== false
  const data = pipelineEvent?.data && typeof pipelineEvent.data === 'object'
    ? pipelineEvent.data
    : {}
  const eventData = includeSensitive ? data : sanitizePluginEventData(data)
  const payload = {
    ...eventData,
    event_type: pipelineEvent.event_type,
    data: eventData,
  }
  if (includeSensitive) {
    payload.raw = pipelineEvent
  }

  if (includeSensitive && pipelineEvent.event_type === 'editor_progress') {
    payload.token = eventData.delta || ''
    payload.text = eventData.delta || ''
  } else if (includeSensitive && pipelineEvent.event_type === 'draft_ready') {
    payload.text = eventData.text || ''
  } else if (includeSensitive && pipelineEvent.event_type === 'error') {
    payload.message = eventData.message || ''
  }

  return payload
}

/**
 * Only true terminal turn commits may fan out to MESSAGE_RECEIVED /
 * CHARACTER_MESSAGE_RENDERED / CHAT_CHANGED.
 *
 * Pipeline `state_changed{Committed}` after `append_ai_draft` is NOT a user
 * Accept: the variant is still Draft and may be discarded. Mapping that state
 * to ST message events would let plugins mutate variables/network/storage
 * before Accept with no rollback path.
 *
 * Allowed sources:
 * - event_type === 'committed' (PipelineEvent::Committed)
 * - explicit terminal markers on the payload (turn/attempt accepted final)
 */
export function isTerminalTurnCommitEvent(pipelineEvent) {
  if (!pipelineEvent?.event_type) return false
  if (pipelineEvent.event_type === 'committed') return true
  if (pipelineEvent.event_type !== 'state_changed') return false
  const data = pipelineEvent.data && typeof pipelineEvent.data === 'object'
    ? pipelineEvent.data
    : {}
  // Explicit accept / finalization markers only. Bare pipeline state labels
  // like "Committed" after draft write are intentionally excluded.
  if (data.terminalTurnCommit === true || data.turnCommitted === true || data.accepted === true) {
    return true
  }
  const turnStatus = String(data.turnStatus || data.turn_status || '').toLowerCase()
  const attemptStatus = String(data.attemptStatus || data.attempt_status || '').toLowerCase()
  const variantStatus = String(data.variantStatus || data.variant_status || '').toLowerCase()
  if (
    (turnStatus === 'committed' || turnStatus === 'degraded')
    && (attemptStatus === 'committed' || attemptStatus === 'final' || variantStatus === 'final')
  ) {
    return true
  }
  return false
}

/**
 * 将 Tauri WritingEvent 映射为插件可订阅事件。
 *
 * 同时发送 StoryForge 原生事件名（pipeline.xxx / xxx）和少量 ST 常用别名；
 * ST 99 事件全集仍由后续兼容层继续补齐。
 */
export function mapPipelineEventToPluginEvents(pipelineEvent, plugin = null) {
  if (!pipelineEvent?.event_type) return []
  if (pipelineEvent.event_type === 'prompt_hook_request') return []

  const payload = createPluginEventPayload(pipelineEvent, eventPayloadOptions(plugin))
  const names = [
    `pipeline.${pipelineEvent.event_type}`,
    pipelineEvent.event_type,
  ]
  // Only terminal turn commits get ST message/chat aliases. Bare
  // state_changed{Committed} after draft append must not.
  if (isTerminalTurnCommitEvent(pipelineEvent)) {
    names.push('committed', ...(ST_EVENT_ALIASES.committed || []))
  } else if (pipelineEvent.event_type !== 'committed') {
    names.push(...(ST_EVENT_ALIASES[pipelineEvent.event_type] || []))
  }

  return uniqueEventNames(names)
    .filter((name) => {
      return isSubscribedToPluginEvent(plugin, name)
    })
    .map((name) => ({ event: name, data: payload }))
}

function mapGenericPluginEvent(eventName, data, plugin = null) {
  if (!eventName) return []
  const payload = data && typeof data === 'object' ? data : {}
  const eventData = canReadMemory(plugin) ? payload : sanitizePluginEventData(payload)
  return deriveSillyTavernHostEventNames(eventName, payload)
    .filter((name) => isSubscribedToPluginEvent(plugin, name))
    .map((name) => ({ event: name, data: eventData }))
}

/**
 * 将 App.vue 事件 feed 中的一条记录规范化为 PluginHost 可发送的事件数组。
 * 支持历史的 PipelineEvent 记录，也支持 `CHAT_CHANGED` 等宿主通用事件。
 */
export function mapPluginEventRecordToPluginEvents(record, plugin = null) {
  if (!record) return []
  if (record.event_type) return mapPipelineEventToPluginEvents(record, plugin)

  const event = record.event
  if (event?.event_type) return mapPipelineEventToPluginEvents(event, plugin)
  if (typeof event === 'string') return mapGenericPluginEvent(event, record.data, plugin)
  if (typeof record.name === 'string') return mapGenericPluginEvent(record.name, record.data, plugin)
  if (typeof event?.name === 'string') return mapGenericPluginEvent(event.name, event.data ?? record.data, plugin)
  if (typeof event?.event === 'string') return mapGenericPluginEvent(event.event, event.data ?? record.data, plugin)

  return []
}

// ─── 注入到 iframe 的 window.storyforge stub 脚本 ──────────────────────────

/**
 * 生成注入到 iframe srcdoc 前部的 <script> 内容
 * 创建 window.storyforge 对象，所有 API 调用通过 postMessage 发送给宿主
 */
function defaultHostOrigin() {
  if (typeof window !== 'undefined' && window.location?.origin) {
    return window.location.origin
  }
  return '*'
}

function normalizeTargetOrigin(origin) {
  return typeof origin === 'string' && origin.trim() ? origin.trim() : '*'
}

function responseTargetOrigin(event) {
  const origin = event?.origin
  return origin && origin !== 'null' ? origin : '*'
}

function postResponse(event, payload) {
  if (event?.source && typeof event.source.postMessage === 'function') {
    event.source.postMessage(payload, responseTargetOrigin(event))
  }
}

export function generateBridgeScript(pluginId, hostOrigin = defaultHostOrigin()) {
  const targetOrigin = normalizeTargetOrigin(hostOrigin)
  return `<script>
(function() {
  let _reqId = 0;
  const _callbacks = {};
  const _eventTypes = ${JSON.stringify(ST_EVENT_TYPES)};
  const _eventListeners = {};
  const _slashCommands = [];
  const _chat = [];
  const _macros = {};
  const _tools = {};
  const _hostOrigin = ${JSON.stringify(targetOrigin)};
  const _uiSlotAliases = {
    slash_command: 'slash',
    status: 'statusbar',
    status_bar: 'statusbar',
    statusBar: 'statusbar',
  };
  const _popupTypes = {
    TEXT: 1,
    CONFIRM: 2,
    INPUT: 3,
    DISPLAY: 4,
    CROP: 5,
  };
  const _popupResults = {
    AFFIRMATIVE: 1,
    NEGATIVE: 0,
    CANCELLED: null,
    CUSTOM1: 1001,
    CUSTOM2: 1002,
    CUSTOM3: 1003,
    CUSTOM4: 1004,
    CUSTOM5: 1005,
    CUSTOM6: 1006,
    CUSTOM7: 1007,
    CUSTOM8: 1008,
    CUSTOM9: 1009,
  };

  function _postToHost(message) {
    if (!_canPostToHost()) {
      return false;
    }
    parent.postMessage(message, _hostOrigin);
    return true;
  }

  function _canPostToHost() {
    return typeof parent !== 'undefined' && parent && typeof parent.postMessage === 'function';
  }

  function _listenerList(eventName) {
    if (!_eventListeners[eventName]) {
      _eventListeners[eventName] = [];
    }
    return _eventListeners[eventName];
  }

  function _off(eventName, callback) {
    const listeners = _eventListeners[eventName];
    if (!listeners) return;
    const index = listeners.indexOf(callback);
    if (index >= 0) listeners.splice(index, 1);
  }

  function _on(eventName, callback) {
    if (typeof callback !== 'function') return function() {};
    _listenerList(eventName).push(callback);
    return function() { _off(eventName, callback); };
  }

  function _once(eventName, callback) {
    if (typeof callback !== 'function') return function() {};
    function wrapped() {
      _off(eventName, wrapped);
      return callback.apply(null, arguments);
    }
    return _on(eventName, wrapped);
  }

  function _makeFirst(eventName, callback) {
    if (typeof callback !== 'function') return function() {};
    _off(eventName, callback);
    _listenerList(eventName).unshift(callback);
    return function() { _off(eventName, callback); };
  }

  function _makeLast(eventName, callback) {
    if (typeof callback !== 'function') return function() {};
    _off(eventName, callback);
    _listenerList(eventName).push(callback);
    return function() { _off(eventName, callback); };
  }

  function _dispatch(eventName) {
    const args = Array.prototype.slice.call(arguments, 1);
    const listeners = (_eventListeners[eventName] || []).slice();
    listeners.forEach(function(fn) {
      try { fn.apply(null, args); } catch(err) { console.error(err); }
    });
  }

  async function _emit(eventName) {
    const args = Array.prototype.slice.call(arguments, 1);
    const listeners = (_eventListeners[eventName] || []).slice();
    for (let i = 0; i < listeners.length; i++) {
      try {
        await listeners[i].apply(null, args);
      } catch(err) {
        console.error(err);
      }
    }
  }

  function _emitAndWait(eventName) {
    const args = Array.prototype.slice.call(arguments, 1);
    const listeners = (_eventListeners[eventName] || []).slice();
    let currentArgs = args;
    return listeners.reduce(function(chain, fn) {
      return chain.then(function() {
        return fn.apply(null, currentArgs);
      }).then(function(result) {
        if (result && typeof result === 'object') {
          currentArgs = currentArgs.length <= 1
            ? [result]
            : [result].concat(currentArgs.slice(1));
        }
        return currentArgs[0];
      });
    }, Promise.resolve()).then(function() { return currentArgs[0]; });
  }

  function _normalizeUiSlot(slotName) {
    return _uiSlotAliases[slotName] || slotName;
  }

  function _mountToSlot(slotName, html) {
    _postToHost({
      type: '${MSG_MOUNT}',
      pluginId: ${JSON.stringify(pluginId)},
      slot: _normalizeUiSlot(slotName),
      html: html,
    });
  }

  function _normalizeSlashCommand(command, callback, aliases) {
    if (typeof command === 'string') {
      return {
        name: command,
        callback: typeof callback === 'function' ? callback : function() {},
        aliases: Array.isArray(aliases) ? aliases : [],
      };
    }
    if (command && typeof command === 'object') {
      return {
        name: command.name || command.command || '',
        callback: typeof command.callback === 'function' ? command.callback : function() {},
        aliases: Array.isArray(command.aliases) ? command.aliases : [],
        helpString: command.helpString || command.help || '',
        returns: command.returns,
        namedArgumentList: command.namedArgumentList || [],
        unnamedArgumentList: command.unnamedArgumentList || [],
      };
    }
    return { name: '', callback: function() {}, aliases: [] };
  }

  function _registerSlashCommand(command, callback, aliases) {
    const normalized = _normalizeSlashCommand(command, callback, aliases);
    if (!normalized.name) return normalized;
    const aliasSet = {};
    normalized.aliases = (normalized.aliases || [])
      .map(function(alias) { return String(alias || '').trim(); })
      .filter(function(alias) {
        if (!alias || alias === normalized.name || aliasSet[alias]) return false;
        aliasSet[alias] = true;
        return !_slashCommands.some(function(item) {
          return item.name === alias && item.name !== normalized.name;
        });
      });
    const reserved = [normalized.name].concat(normalized.aliases);
    _slashCommands.forEach(function(item) {
      if (!item || item.name === normalized.name || !Array.isArray(item.aliases)) return;
      item.aliases = item.aliases.filter(function(alias) {
        return reserved.indexOf(alias) < 0;
      });
    });
    const existingIndex = _slashCommands.findIndex(function(item) {
      return item.name === normalized.name;
    });
    if (existingIndex >= 0) {
      _slashCommands.splice(existingIndex, 1, normalized);
    } else {
      _slashCommands.push(normalized);
    }
    return normalized;
  }

  function _findSlashCommandIndex(name) {
    const commandName = String(name || '').trim();
    if (!commandName) return -1;
    const primaryIndex = _slashCommands.findIndex(function(item) {
      return item.name === commandName;
    });
    if (primaryIndex >= 0) return primaryIndex;
    return _slashCommands.findIndex(function(item) {
      return item.aliases.indexOf(commandName) >= 0;
    });
  }

  function _unregisterSlashCommand(name) {
    const index = _findSlashCommandIndex(name);
    if (index < 0) return false;
    _slashCommands.splice(index, 1);
    return true;
  }

  function _unsupportedSlashResult(name, options) {
    const commandName = String(name || '').trim() || '<empty>';
    const message = 'Unsupported slash command: ' + commandName + ' is not registered';
    const result = {
      ok: false,
      unsupported: true,
      reason: 'unsupported_slash_command',
      command: commandName,
      message: message,
    };
    // Pipes need a hard failure so later segments do not run on silent success.
    // Single-command ST callers keep a non-throwing object for compatibility.
    if (options && options.throwing) {
      const error = new Error(message);
      error.code = 'SLASH_UNSUPPORTED';
      error.unsupported = true;
      error.command = commandName;
      error.result = result;
      throw error;
    }
    return result;
  }

  function _invokeSlashCommand(name, args, options) {
    const index = _findSlashCommandIndex(name);
    const command = index >= 0 ? _slashCommands[index] : null;
    if (!command) return _invokeBuiltinSlashCommand(name, args, options);
    return command.callback.apply(null, args);
  }

  function _invokeBuiltinSlashCommand(name, args, options) {
    if (name === 'genraw') {
      return window.storyforge.llm.generate(args && args.length ? args[0] : '');
    }
    return _unsupportedSlashResult(name, options);
  }

  function _isPromiseLike(value) {
    return value && typeof value.then === 'function';
  }

  function _splitSlashPipeline(input) {
    const segments = [];
    let current = '';
    let quote = '';
    const text = String(input || '');
    for (let i = 0; i < text.length; i++) {
      const ch = text.charAt(i);
      if ((ch === '"' || ch === "'") && text.charAt(i - 1) !== '\\\\') {
        quote = quote === ch ? '' : quote || ch;
        current += ch;
      } else if (ch === '|' && !quote) {
        if (current.trim()) segments.push(current.trim());
        current = '';
      } else {
        current += ch;
      }
    }
    if (current.trim()) segments.push(current.trim());
    return segments;
  }

  function _executeSlashInvocation(input) {
    const segments = _splitSlashPipeline(input);
    const multiSegment = segments.length > 1;
    let previousResult;
    let chain = null;
    function runSegment(index, resolvedPipe) {
      const parsed = _parseSlashInvocation(segments[index]);
      if (!parsed) return resolvedPipe;
      if (index > 0) {
        parsed.pipe = resolvedPipe;
        parsed.previousResult = resolvedPipe;
      }
      // Multi-segment pipes fail closed on unknown commands so later stages never run.
      return _invokeSlashCommand(parsed.name, [parsed.args, parsed], {
        throwing: multiSegment,
      });
    }
    for (let i = 0; i < segments.length; i++) {
      if (chain) {
        chain = chain.then(function(resolvedPipe) {
          return runSegment(i, resolvedPipe);
        });
      } else {
        previousResult = runSegment(i, previousResult);
        if (_isPromiseLike(previousResult)) {
          chain = Promise.resolve(previousResult);
        }
      }
    }
    return chain || previousResult;
  }

  function _triggerSlashCommand(name) {
    const args = Array.prototype.slice.call(arguments, 1);
    const trimmedName = typeof name === 'string' ? name.trim() : '';
    if (args.length === 0 && (trimmedName.charAt(0) === '/' || /\\s|\\|/.test(trimmedName))) {
      return _executeSlashInvocation(name);
    }
    return _invokeSlashCommand(trimmedName, args, { throwing: false });
  }

  function _parseSlashTokens(rawArgs) {
    const tokens = [];
    String(rawArgs || '').replace(/"([^"]*)"|'([^']*)'|(\\S+)/g, function(_, dq, sq, bare) {
      tokens.push(dq || sq || bare || '');
      return '';
    });
    return tokens;
  }

  function _parseSlashArguments(rawArgs) {
    const tokens = _parseSlashTokens(rawArgs);
    const namedArgs = {};
    const unnamedArgs = [];

    for (let i = 0; i < tokens.length; i++) {
      const token = tokens[i];
      if (token.indexOf('--') === 0 && token.length > 2) {
        const body = token.slice(2);
        const eqIndex = body.indexOf('=');
        if (eqIndex >= 0) {
          namedArgs[body.slice(0, eqIndex)] = body.slice(eqIndex + 1);
        } else if (i + 1 < tokens.length && tokens[i + 1].indexOf('--') !== 0) {
          namedArgs[body] = tokens[i + 1];
          i += 1;
        } else {
          namedArgs[body] = true;
        }
      } else if (/^[A-Za-z_][A-Za-z0-9_.-]*=/.test(token)) {
        const eqIndex = token.indexOf('=');
        namedArgs[token.slice(0, eqIndex)] = token.slice(eqIndex + 1);
      } else {
        unnamedArgs.push(token);
      }
    }

    return { namedArgs: namedArgs, unnamedArgs: unnamedArgs };
  }

  function _parseSlashInvocation(input) {
    const trimmed = String(input || '').trim();
    if (!trimmed) return null;
    const withoutSlash = trimmed.charAt(0) === '/' ? trimmed.slice(1) : trimmed;
    const match = withoutSlash.match(/^(\\S+)(?:\\s+([\\s\\S]*))?$/);
    if (!match) return null;
    const rawArgs = (match[2] || '').trim();
    const parsedArgs = _parseSlashArguments(rawArgs);
    return {
      name: match[1],
      args: rawArgs,
      rawArgs: rawArgs,
      namedArgs: parsedArgs.namedArgs,
      unnamedArgs: parsedArgs.unnamedArgs,
      source: 'slash',
      input: input,
    };
  }

  function _isVariableSelector(value) {
    return value && typeof value === 'object' && !Array.isArray(value);
  }

  function _looksLikeVariableSelector(value) {
    if (!_isVariableSelector(value) || typeof value.type !== 'string') return false;
    const keys = Object.keys(value);
    const hasOnlyKeys = function(allowed) {
      return keys.every(function(key) { return allowed.includes(key); });
    };

    if (value.type === 'message') {
      return hasOnlyKeys(['type', 'message_id', 'messageId']) && ('message_id' in value || 'messageId' in value);
    }
    if (value.type === 'preset') {
      return hasOnlyKeys(['type', 'preset_id', 'presetId', 'name']);
    }
    // ST card shells also use character/script bags (FrontEnd-for-destined-journey).
    if (['local', 'chat', 'global', 'character', 'script'].includes(value.type)) {
      return hasOnlyKeys(['type']);
    }
    return false;
  }

  function _stableJson(value) {
    if (!_isVariableSelector(value)) return JSON.stringify(value);
    const keys = Object.keys(value).sort();
    const normalized = {};
    keys.forEach(function(key) { normalized[key] = value[key]; });
    return JSON.stringify(normalized);
  }

  function _variableSelectorKey(selector) {
    const scope = selector || { type: 'local' };
    return 'sf_plugin_' + ${JSON.stringify(pluginId)} + '_variables_' + _stableJson(scope);
  }

  function _safeLocalStorageGet(key) {
    try {
      if (typeof localStorage === 'undefined') return null;
      return localStorage.getItem(key);
    } catch {
      return null;
    }
  }

  function _safeLocalStorageSet(key, value) {
    try {
      if (typeof localStorage === 'undefined') return false;
      localStorage.setItem(key, value);
      return true;
    } catch {
      return false;
    }
  }

  function _readSelectorVariables(selector) {
    try {
      const parsed = JSON.parse(_safeLocalStorageGet(_variableSelectorKey(selector)) || '{}');
      return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed : {};
    } catch {
      return {};
    }
  }

  function _writeSelectorVariables(selector, variables) {
    const next = variables && typeof variables === 'object' && !Array.isArray(variables) ? variables : {};
    _safeLocalStorageSet(_variableSelectorKey(selector), JSON.stringify(next));
    return next;
  }

  function _getVariables(selectorOrCampaignId, instanceId) {
    if (arguments.length <= 1 && (_isVariableSelector(selectorOrCampaignId) || selectorOrCampaignId == null)) {
      return _readSelectorVariables(selectorOrCampaignId);
    }
    return window.storyforge.variables.get(selectorOrCampaignId, instanceId);
  }

  function _setVariables(selectorOrCampaignId, instanceIdOrVariables, key, value) {
    if (_looksLikeVariableSelector(selectorOrCampaignId) && arguments.length <= 2) {
      return _writeSelectorVariables(selectorOrCampaignId, instanceIdOrVariables);
    }
    if (_looksLikeVariableSelector(instanceIdOrVariables) && arguments.length <= 2) {
      return _writeSelectorVariables(instanceIdOrVariables, selectorOrCampaignId);
    }
    return window.storyforge.variables.set(selectorOrCampaignId, instanceIdOrVariables, key, value);
  }

  function _getVariable(selectorOrCampaignId, instanceIdOrKey, maybeKey) {
    if (_isVariableSelector(selectorOrCampaignId) || selectorOrCampaignId == null) {
      const variables = _readSelectorVariables(selectorOrCampaignId);
      return variables[instanceIdOrKey];
    }
    return window.storyforge.variables.get(selectorOrCampaignId, instanceIdOrKey).then(function(variables) {
      const key = maybeKey;
      if (Array.isArray(variables)) {
        const found = variables.find(function(item) { return item && item.key === key; });
        return found ? found.value : undefined;
      }
      return variables && typeof variables === 'object' ? variables[key] : undefined;
    });
  }

  function _setVariable(selectorOrCampaignId, instanceIdOrKey, keyOrValue, maybeValue) {
    if (_looksLikeVariableSelector(selectorOrCampaignId) || selectorOrCampaignId == null) {
      const variables = _readSelectorVariables(selectorOrCampaignId);
      variables[instanceIdOrKey] = keyOrValue;
      return _writeSelectorVariables(selectorOrCampaignId, variables);
    }
    return window.storyforge.variables.set(selectorOrCampaignId, instanceIdOrKey, keyOrValue, maybeValue);
  }

  function _insertOrAssignVariables(selector, variables) {
    const targetSelector = _looksLikeVariableSelector(selector) ? selector : variables;
    const patchSource = _looksLikeVariableSelector(selector) ? variables : selector;
    const current = _readSelectorVariables(targetSelector);
    const patch = patchSource && typeof patchSource === 'object' && !Array.isArray(patchSource) ? patchSource : {};
    return _writeSelectorVariables(targetSelector, Object.assign({}, current, patch));
  }

  function _replaceVariables(selector, variables) {
    const targetSelector = _looksLikeVariableSelector(selector) ? selector : variables;
    const nextVariables = _looksLikeVariableSelector(selector) ? variables : selector;
    return _writeSelectorVariables(targetSelector, nextVariables);
  }

  function _updateVariablesWith(selector, updater) {
    const targetSelector = _looksLikeVariableSelector(selector) ? selector : updater;
    const updaterOrNext = _looksLikeVariableSelector(selector) ? updater : selector;
    const current = _readSelectorVariables(targetSelector);
    const next = typeof updaterOrNext === 'function' ? updaterOrNext(Object.assign({}, current)) : updaterOrNext;
    if (next && typeof next === 'object' && !Array.isArray(next)) {
      return _writeSelectorVariables(targetSelector, next);
    }
    return current;
  }

  function _normalizeChatMessage(message, index) {
    const source = message && typeof message === 'object' ? message : {};
    const normalized = Object.assign({}, source);
    normalized.message_id = source.message_id ?? source.id ?? index;
    normalized.message = source.message ?? source.mes ?? '';
    normalized.mes = source.mes ?? normalized.message;
    return normalized;
  }

  function _toMessageIndex(value) {
    if (Number.isInteger(value)) return value;
    if (typeof value === 'string' && value.trim() !== '') {
      const parsed = Number(value);
      if (Number.isInteger(parsed)) return parsed;
    }
    return null;
  }

  function _messageIndexFromPayload(payload) {
    const source = payload && typeof payload === 'object' ? payload : {};
    return _toMessageIndex(
      source.message_id
      ?? source.message_index
      ?? source.messageIndex
      ?? source.message?.message_id
      ?? source.message?.message_index
      ?? source.message?.messageIndex
    );
  }

  function _hostMessageIdFromPayload(payload) {
    const source = payload && typeof payload === 'object' ? payload : {};
    const value = source.messageId ?? source.message_id ?? source.id ?? source.message?.id ?? source.message?.message_id;
    return value === undefined || value === null ? null : String(value);
  }

  function _findChatMessageIndex(payload) {
    const numericIndex = _messageIndexFromPayload(payload);
    if (numericIndex !== null) return numericIndex;
    const hostId = _hostMessageIdFromPayload(payload);
    if (!hostId) return null;
    const found = _chat.findIndex(function(message) {
      if (!message || typeof message !== 'object') return false;
      return String(message.host_message_id ?? message.id ?? '') === hostId;
    });
    return found >= 0 ? found : null;
  }

  function _roleName(role) {
    return role === 'user' ? 'User' : 'Assistant';
  }

  function _messageFromEventPayload(payload, fallbackIndex) {
    const source = payload && typeof payload === 'object' ? payload : {};
    const nested = source.message && typeof source.message === 'object' ? source.message : {};
    const message = Object.assign({}, nested, source);
    delete message.message;
    const index = fallbackIndex;
    const role = message.role || (message.is_user ? 'user' : 'assistant');
    const text = message.mes ?? message.content ?? message.displayContent ?? message.display_content ?? message.text ?? message.message ?? '';
    message.message_id = index;
    message.id = message.id ?? index;
    message.role = role;
    message.name = message.name || _roleName(role);
    message.is_user = message.is_user ?? role === 'user';
    message.mes = text;
    message.message = text;
    if (source.messageId !== undefined) message.host_message_id = source.messageId;
    if (source.variantId !== undefined) message.variant_id = source.variantId;
    return message;
  }

  function _reindexChatFrom(start) {
    for (let i = Math.max(0, start || 0); i < _chat.length; i++) {
      if (_chat[i] && typeof _chat[i] === 'object') {
        _chat[i].message_id = i;
      }
    }
  }

  function _upsertChatMessage(payload) {
    const index = _findChatMessageIndex(payload) ?? _messageIndexFromPayload(payload) ?? _chat.length;
    const current = _chat[index] && typeof _chat[index] === 'object' ? _chat[index] : {};
    _chat[index] = Object.assign({}, current, _messageFromEventPayload(payload, index));
    return _chat[index];
  }

  function _deleteChatMessage(payload) {
    const index = _findChatMessageIndex(payload);
    if (index === null || index < 0 || index >= _chat.length) return false;
    _chat.splice(index);
    return true;
  }

  function _syncChatFromHostEvent(eventName, payload) {
    if (eventName === _eventTypes.MESSAGE_SENT || eventName === _eventTypes.MESSAGE_RECEIVED) {
      _upsertChatMessage(payload);
      return;
    }
    if (eventName === _eventTypes.MESSAGE_UPDATED || eventName === _eventTypes.MESSAGE_SWIPED) {
      const index = _findChatMessageIndex(payload);
      if (index === null) {
        _upsertChatMessage(payload);
        return;
      }
      const current = _chat[index] && typeof _chat[index] === 'object' ? _chat[index] : {};
      _chat[index] = Object.assign({}, current, _messageFromEventPayload(payload, index));
      return;
    }
    if (eventName === _eventTypes.MESSAGE_DELETED) {
      _deleteChatMessage(payload);
    }
  }

  function _getLastMessageId() {
    return _chat.length - 1;
  }

  function _getCurrentMessageId() {
    return _getLastMessageId();
  }

  function _getCurrentChatId() {
    return window.currentChatId || window.chatId || '';
  }

  function _getChatMessages(messageId) {
    const numericId = messageId === undefined || messageId === null || messageId === ''
      ? null
      : Number(messageId);
    const end = Number.isInteger(numericId) ? Math.min(numericId, _chat.length - 1) : _chat.length - 1;
    if (end < 0) return [];
    return _chat.slice(0, end + 1).map(_normalizeChatMessage);
  }

  function _setChatMessage(message, messageId) {
    const id = Number.isInteger(messageId) ? messageId : Number(messageId);
    if (!Number.isFinite(id) || id < 0) return Promise.resolve(false);
    const patch = message && typeof message === 'object' ? message : { message: String(message ?? '') };
    const current = _chat[id] && typeof _chat[id] === 'object' ? _chat[id] : {};
    const next = Object.assign({}, current, patch, { message_id: id });
    if ('message' in patch && !('mes' in patch)) next.mes = patch.message;
    if ('mes' in patch && !('message' in patch)) next.message = patch.mes;
    _chat[id] = next;
    return Promise.resolve(true);
  }

  function _setChatMessages(messages) {
    const updates = Array.isArray(messages) ? messages : [messages];
    updates.forEach(function(update) {
      if (!update || typeof update !== 'object') return;
      const id = update.message_id ?? update.id;
      _setChatMessage(update, Number(id));
    });
    return Promise.resolve(true);
  }

  function _degradedSaveChatPending() {
    // Local chat mirror only fallback — no host persistence result.
    // Keep ST boolean compatibility: awaited value is true, while the
    // promise object itself carries a visible degraded marker.
    const pending = Promise.resolve(true);
    pending.ok = true;
    pending.degraded = true;
    pending.reason = 'local_mirror_only_no_host_persist';
    pending.persistedAt = null;
    pending.outcomeUnknown = false;
    return pending;
  }

  function _stampSaveChatPending(pending, result) {
    pending.ok = result?.ok !== false;
    pending.degraded = Boolean(result?.degraded);
    pending.reason = result?.reason || null;
    pending.persistedAt = result?.persistedAt ?? null;
    pending.outcomeUnknown = Boolean(result?.outcomeUnknown);
    return pending;
  }

  function _saveChat() {
    // Try the host-side persistence adapter first (chat.save route). When the
    // host is unavailable, rejects, times out, or returns a degraded marker,
    // fall back to the local-mirror degraded promise so ST plugins keep working
    // (await saveChat() resolves truthy === ST boolean compatibility).
    if (!_canPostToHost()) return _degradedSaveChatPending();

    // A single stampable promise object so synchronous property reads before
    // resolution AND the resolved value both reflect host/degraded state.
    let resolveSave
    let settled = false
    const pending = new Promise((resolve) => { resolveSave = resolve })
    pending.ok = true
    pending.degraded = true
    pending.reason = 'local_mirror_only_no_host_persist'
    pending.persistedAt = null
    pending.outcomeUnknown = false
    // Generation token correlates this iframe call. Host-side snapshot
    // idempotency owns durable retry safety; ignoring a late response alone is
    // not enough to prevent a second persistence side effect.
    const generation = String(++_reqId) + ':saveChat'

    const finish = function(result) {
      if (settled) return
      settled = true
      _stampSaveChatPending(pending, result || {})
      resolveSave(true)
    }

    // A timeout means persistence may still complete at the host. Preserve ST's
    // truthy await contract, but expose outcome_unknown instead of claiming a
    // known local-only fallback.
    const timer = setTimeout(function() {
      finish({
        ok: false,
        degraded: true,
        reason: 'persist_outcome_unknown',
        persistedAt: null,
        timedOut: true,
        outcomeUnknown: true,
        generation: generation,
      })
    }, 500)

    _call('chat.save', { chat: _chat, generation: generation })
      .then(function(result) {
        clearTimeout(timer)
        // Late success after timeout must not flip the already-settled promise
        // or imply a second durable write for the same saveChat call.
        if (settled) return
        finish(result)
      })
      .catch(function() {
        clearTimeout(timer)
        if (settled) return
        finish(null)
      })

    return pending
  }

  function _degradedPopupValue(type, defaultValue) {
    if (defaultValue !== undefined) return String(defaultValue)
    if (type === _popupTypes.CONFIRM || String(type || '').toLowerCase() === 'confirm') return null
    return ''
  }

  function _callGenericPopup(html, type, defaultValue) {
    // Host popup adapter when available; race a short timeout so missing host
    // handlers never hang ST plugins forever.
    if (_canPostToHost()) {
      return Promise.race([
        _call('ui.popup', {
          html: html,
          type: type,
          defaultValue: defaultValue,
        }).then(function(result) {
          if (result && typeof result === 'object' && 'value' in result) return result.value
          return result
        }),
        new Promise(function(resolve) {
          setTimeout(function() {
            resolve(_degradedPopupValue(type, defaultValue))
          }, 250)
        }),
      ]).catch(function() {
        return _degradedPopupValue(type, defaultValue)
      })
    }
    return Promise.resolve(_degradedPopupValue(type, defaultValue))
  }

  function _getRequestHeaders() {
    // Host request-headers adapter when available; always redacted server-side.
    // Synchronous ST API: return the last cached host headers or the static shim.
    // Fire-and-forget refresh must not block or hang callers.
    if (_canPostToHost()) {
      Promise.race([
        _call('ui.requestHeaders', {}),
        new Promise(function(resolve) { setTimeout(function() { resolve(null) }, 250) }),
      ]).then(function(headers) {
        if (headers && typeof headers === 'object') {
          window.__sfRequestHeadersCache = headers
        }
      }).catch(function() {})
      if (window.__sfRequestHeadersCache && typeof window.__sfRequestHeadersCache === 'object') {
        return window.__sfRequestHeadersCache
      }
    }
    return { 'Content-Type': 'application/json' };
  }

  function _registerMacro(name, callback) {
    if (typeof name === 'string' && name) {
      _macros[name] = typeof callback === 'function' ? callback : function() { return ''; };
    }
    return callback;
  }

  function _unregisterMacro(name) {
    delete _macros[name];
  }

  function _extensionSettingsKey() {
    return 'sf_plugin_' + ${JSON.stringify(pluginId)} + '_extension_settings';
  }

  function _readExtensionSettings() {
    try {
      const parsed = JSON.parse(_safeLocalStorageGet(_extensionSettingsKey()) || '{}');
      return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed : {};
    } catch {
      return {};
    }
  }

  function _writeExtensionSettings() {
    return _safeLocalStorageSet(_extensionSettingsKey(), JSON.stringify(window.extension_settings || {}));
  }

  function _saveSettingsDebounced() {
    _writeExtensionSettings();
    if (_canPostToHost()) {
      _call('storage.set', { key: 'extension_settings', value: window.extension_settings || {} })
        .catch(function(err) { console.error('[StoryForge plugin] saveSettingsDebounced failed', err); });
    }
    _emit(_eventTypes.SETTINGS_UPDATED, { extension_settings: window.extension_settings });
    return true;
  }

  function _loadExtensionSettingsFromHost() {
    if (!_canPostToHost()) {
      _emit(_eventTypes.EXTENSION_SETTINGS_LOADED, { extension_settings: window.extension_settings });
      return;
    }
    _call('storage.get', { key: 'extension_settings' })
      .then(function(saved) {
        if (saved && typeof saved === 'object' && !Array.isArray(saved)) {
          Object.assign(window.extension_settings, saved);
          window.extension_settings.storyforge = window.extension_settings.storyforge || {};
          window.extension_settings.storyforge.statusBar = window.extension_settings.storyforge.statusBar || {};
          window.extension_settings.storyforge.status_bar = window.extension_settings.storyforge.statusBar;
          _writeExtensionSettings();
        }
        return _emit(_eventTypes.EXTENSION_SETTINGS_LOADED, { extension_settings: window.extension_settings });
      })
      .catch(function() {
        _emit(_eventTypes.EXTENSION_SETTINGS_LOADED, { extension_settings: window.extension_settings });
      });
  }

  function _storageKey(key) {
    return 'sf_plugin_' + ${JSON.stringify(pluginId)} + '_' + key;
  }

  function _storageGet(key) {
    const raw = _safeLocalStorageGet(_storageKey(key));
    if (raw !== null) {
      try { return JSON.parse(raw); }
      catch { return null; }
    }
    if (!_canPostToHost()) return null;
    _call('storage.get', { key: key }).then(function(value) {
      if (value !== null && value !== undefined) {
        _safeLocalStorageSet(_storageKey(key), JSON.stringify(value));
      }
      return value ?? null;
    }).catch(function() { return null; });
    return null;
  }

  function _storageSet(key, value) {
    const cached = _safeLocalStorageSet(_storageKey(key), JSON.stringify(value));
    if (!_canPostToHost()) return cached;
    const pending = _call('storage.set', { key: key, value: value })
      .catch(function(err) { console.error('[StoryForge plugin] storage.set failed', err); });
    return cached ? true : pending;
  }

  function _getContext() {
    const currentChatId = _getCurrentChatId();
    return {
      chat: _chat,
      name1: window.SillyTavern?.name1 || 'User',
      name2: window.SillyTavern?.name2 || 'Assistant',
      characters: window.characters || [],
      groups: window.groups || [],
      this_chid: window.this_chid ?? null,
      characterId: window.this_chid ?? null,
      selected_group: window.selected_group ?? null,
      groupId: window.selected_group ?? null,
      chatId: currentChatId,
      currentChatId: currentChatId,
      getCurrentChatId: function() { return window.currentChatId || window.chatId || ''; },
      chat_metadata: window.chat_metadata || {},
      extensionSettings: window.extension_settings,
      extension_settings: window.extension_settings,
      eventSource: window.eventSource,
      event_types: _eventTypes,
      eventTypes: _eventTypes,
      tavern_events: _eventTypes,
      TavernHelper: window.TavernHelper,
      saveSettingsDebounced: window.saveSettingsDebounced,
    };
  }

  function _createToolManager() {
    return {
      tools: _tools,
      registerTool: function(tool) {
        if (tool && typeof tool === 'object' && tool.name) {
          _tools[tool.name] = tool;
        }
        return tool;
      },
      unregisterTool: function(name) {
        delete _tools[name];
      },
      getTool: function(name) {
        return _tools[name];
      },
    };
  }

  function _createTavernHelper() {
    const onGenerateBeforeCombinePrompts = (callback) => window.storyforge.events.on(_eventTypes.GENERATE_BEFORE_COMBINE_PROMPTS, callback);
    const onChatCompletionPromptReady = (callback) => window.storyforge.events.on(_eventTypes.CHAT_COMPLETION_PROMPT_READY, callback);
    return {
      getCharacters: () => window.storyforge.character.list(),
      getCharacter: (id) => window.storyforge.character.get(id),
      searchWorldInfo: (characterId) => window.storyforge.worldInfo.search(characterId),
      getRecentMessages: (conversationId) => window.storyforge.memory.getRecent(conversationId),
      getVariables: _getVariables,
      setVariables: _setVariables,
      getVariable: _getVariable,
      setVariable: _setVariable,
      insertOrAssignVariables: _insertOrAssignVariables,
      replaceVariables: _replaceVariables,
      updateVariablesWith: _updateVariablesWith,
      getChatMessages: _getChatMessages,
      setChatMessages: _setChatMessages,
      getLastMessageId: _getLastMessageId,
      getCurrentMessageId: _getCurrentMessageId,
      setChatMessage: _setChatMessage,
      getContext: _getContext,
      eventOn: (eventName, callback) => window.storyforge.events.on(eventName, callback),
      eventOnce: (eventName, callback) => window.storyforge.events.once(eventName, callback),
      eventOff: (eventName, callback) => window.storyforge.events.off(eventName, callback),
      eventEmit: (eventName, payload) => window.storyforge.events.emit(eventName, payload),
      eventEmitAndWait: (eventName, payload) => window.storyforge.events.emitAndWait(eventName, payload),
      onGenerateBeforeCombinePrompts,
      onChatCompletionPromptReady,
      promptHooks: {
        onGenerateBeforeCombinePrompts,
        onChatCompletionPromptReady,
      },
      registerSlashCommand: _registerSlashCommand,
      unregisterSlashCommand: _unregisterSlashCommand,
      triggerSlash: _triggerSlashCommand,
      triggerSlashCommand: _triggerSlashCommand,
      setStatusBar: (html) => window.storyforge.statusBar.set(html),
      clearStatusBar: () => window.storyforge.statusBar.clear(),
      mountToSlot: (slot, html) => window.storyforge.ui.mountToSlot(slot, html),
      storageGet: (key) => window.storyforge.storage.get(key),
      storageSet: (key, value) => window.storyforge.storage.set(key, value),
      generate: (prompt) => window.storyforge.llm.generate(prompt),
      saveChat: _saveChat,
      callGenericPopup: _callGenericPopup,
      getRequestHeaders: _getRequestHeaders,
      registerMacro: _registerMacro,
      unregisterMacro: _unregisterMacro,
    };
  }

  function _createSillyTavern() {
    return {
      chat: _chat,
      name1: 'User',
      name2: 'Assistant',
      POPUP_TYPE: Object.assign({}, _popupTypes),
      POPUP_RESULT: Object.assign({}, _popupResults),
      characters: window.characters || [],
      groups: window.groups || [],
      chat_metadata: window.chat_metadata || {},
      extension_settings: window.extension_settings,
      extensionSettings: window.extension_settings,
      this_chid: window.this_chid ?? null,
      characterId: window.this_chid ?? null,
      getCurrentChatId: _getCurrentChatId,
      getCurrentMessageId: _getCurrentMessageId,
      getContext: _getContext,
      event_types: _eventTypes,
      eventTypes: _eventTypes,
      tavern_events: _eventTypes,
      saveChat: _saveChat,
      callGenericPopup: _callGenericPopup,
      getRequestHeaders: _getRequestHeaders,
      ToolManager: _createToolManager(),
      registerMacro: _registerMacro,
      unregisterMacro: _unregisterMacro,
    };
  }

  window.storyforge = {
    pluginId: ${JSON.stringify(pluginId)},

    character: {
      list: () => _call('character.list', {}),
      get: (id) => _call('character.get', { id }),
    },

    worldInfo: {
      search: (characterId) => _call('worldInfo.search', { characterId }),
    },

    memory: {
      getRecent: (conversationId) => _call('memory.getRecent', { conversationId }),
    },

    variables: {
      get: (campaignId, instanceId) => _call('variables.get', { campaignId, instanceId }),
      set: (campaignId, instanceId, key, value) => _call('variables.set', { campaignId, instanceId, key, value }),
    },

    storage: {
      get: _storageGet,
      set: _storageSet,
    },

    llm: {
      generate: (prompt) => _call('llm.generate', prompt && typeof prompt === 'object' ? prompt : { prompt }),
    },

    ui: {
      mountToSlot: _mountToSlot,
      setStatusBar: (html) => _mountToSlot('statusbar', html),
      clearStatusBar: () => _mountToSlot('statusbar', ''),
    },

    events: {
      _listeners: _eventListeners,
      on: _on,
      once: _once,
      off: _off,
      removeListener: _off,
      makeFirst: _makeFirst,
      makeLast: _makeLast,
      emit: _emit,
      emitAndWait: _emitAndWait,
    },

    slashCommands: {
      list: () => _slashCommands.slice(),
      register: _registerSlashCommand,
      unregister: _unregisterSlashCommand,
      trigger: _triggerSlashCommand,
    },

    statusBar: {
      set: (html) => _mountToSlot('statusbar', html),
      clear: () => _mountToSlot('statusbar', ''),
    },
  };

  window.extension_settings = window.extension_settings || _readExtensionSettings();
  window.extension_settings.storyforge = window.extension_settings.storyforge || {};
  window.extension_settings.storyforge.statusBar = window.extension_settings.storyforge.statusBar || {};
  window.extension_settings.storyforge.status_bar = window.extension_settings.storyforge.statusBar;
  window.characters = window.characters || [];
  window.groups = window.groups || [];
  window.chat_metadata = window.chat_metadata || {};

  window.event_types = _eventTypes;
  window.eventTypes = _eventTypes;
  window.tavern_events = window.tavern_events || _eventTypes;
  window.eventSource = {
    on: _on,
    once: _once,
    makeFirst: _makeFirst,
    makeLast: _makeLast,
    removeListener: _off,
    off: _off,
    emit: _emit,
    emitAndWait: _emitAndWait,
  };
  window.registerSlashCommand = _registerSlashCommand;
  window.unregisterSlashCommand = _unregisterSlashCommand;
  window.triggerSlashCommand = _triggerSlashCommand;
  window.triggerSlash = _triggerSlashCommand;
  window.triggerSlashTag = _triggerSlashCommand;
  window.executeSlashCommands = window.executeSlashCommands || _triggerSlashCommand;
  window.SlashCommand = window.SlashCommand || {
    fromProps: function(props) { return props || {}; },
  };
  window.SlashCommandParser = window.SlashCommandParser || {};
  window.SlashCommandParser.commands = _slashCommands;
  window.SlashCommandParser.addCommandObject = function(command) {
    return _registerSlashCommand(command);
  };
  window.SlashCommandParser.removeCommandObject = function(command) {
    return _unregisterSlashCommand(command && typeof command === 'object'
      ? (command.name || command.command)
      : command);
  };
  window.SlashCommandParser.removeCommand = window.SlashCommandParser.removeCommandObject;
  window.TavernHelper = window.TavernHelper || _createTavernHelper();
  window.tavernHelper = window.TavernHelper;
  window.SillyTavern = window.SillyTavern || _createSillyTavern();
  if (!Array.isArray(window.SillyTavern.chat)) window.SillyTavern.chat = _chat;
  window.SillyTavern.chat = window.SillyTavern.chat || _chat;
  window.SillyTavern.saveChat = window.SillyTavern.saveChat || _saveChat;
  window.SillyTavern.callGenericPopup = window.SillyTavern.callGenericPopup || _callGenericPopup;
  window.SillyTavern.getRequestHeaders = window.SillyTavern.getRequestHeaders || _getRequestHeaders;
  window.SillyTavern.getContext = window.SillyTavern.getContext || _getContext;
  window.SillyTavern.event_types = window.SillyTavern.event_types || _eventTypes;
  window.SillyTavern.eventTypes = window.SillyTavern.eventTypes || _eventTypes;
  window.SillyTavern.tavern_events = window.SillyTavern.tavern_events || _eventTypes;
  window.SillyTavern.POPUP_TYPE = Object.assign(window.SillyTavern.POPUP_TYPE || {}, _popupTypes);
  window.SillyTavern.POPUP_RESULT = Object.assign(window.SillyTavern.POPUP_RESULT || {}, _popupResults);
  window.SillyTavern.characters = window.SillyTavern.characters || window.characters;
  window.SillyTavern.groups = window.SillyTavern.groups || window.groups;
  window.SillyTavern.chat_metadata = window.SillyTavern.chat_metadata || window.chat_metadata;
  window.SillyTavern.extension_settings = window.SillyTavern.extension_settings || window.extension_settings;
  window.SillyTavern.extensionSettings = window.SillyTavern.extensionSettings || window.extension_settings;
  window.SillyTavern.this_chid = window.SillyTavern.this_chid ?? window.this_chid ?? null;
  window.SillyTavern.characterId = window.SillyTavern.characterId ?? window.this_chid ?? null;
  window.SillyTavern.getCurrentChatId = window.SillyTavern.getCurrentChatId || _getCurrentChatId;
  window.SillyTavern.getCurrentMessageId = window.SillyTavern.getCurrentMessageId || _getCurrentMessageId;
  window.SillyTavern.ToolManager = window.SillyTavern.ToolManager || _createToolManager();
  window.getVariables = window.getVariables || window.TavernHelper.getVariables;
  window.setVariables = window.setVariables || window.TavernHelper.setVariables;
  window.getVariable = window.getVariable || window.TavernHelper.getVariable;
  window.setVariable = window.setVariable || window.TavernHelper.setVariable;
  window.insertOrAssignVariables = window.insertOrAssignVariables || window.TavernHelper.insertOrAssignVariables;
  window.replaceVariables = window.replaceVariables || window.TavernHelper.replaceVariables;
  window.updateVariablesWith = window.updateVariablesWith || window.TavernHelper.updateVariablesWith;
  window.getChatMessages = window.getChatMessages || window.TavernHelper.getChatMessages;
  window.setChatMessages = window.setChatMessages || window.TavernHelper.setChatMessages;
  window.getLastMessageId = window.getLastMessageId || window.TavernHelper.getLastMessageId;
  window.getCurrentMessageId = window.getCurrentMessageId || window.TavernHelper.getCurrentMessageId;
  window.setChatMessage = window.setChatMessage || window.TavernHelper.setChatMessage;
  window.getCurrentChatId = window.getCurrentChatId || _getCurrentChatId;
  window.getContext = window.getContext || window.TavernHelper.getContext;
  window.registerMacro = window.registerMacro || window.TavernHelper.registerMacro;
  window.unregisterMacro = window.unregisterMacro || window.TavernHelper.unregisterMacro;
  window.saveSettingsDebounced = window.saveSettingsDebounced || _saveSettingsDebounced;
  window.toastr = window.toastr || (function() {
    const calls = [];
    function record(level) {
      return function(message, title, options) {
        const item = { level: level, message: message, title: title, options: options };
        calls.push(item);
        return item;
      };
    }
    return {
      _calls: calls,
      info: record('info'),
      success: record('success'),
      warning: record('warning'),
      warn: record('warning'),
      error: record('error'),
      clear: function() { calls.length = 0; },
      remove: function() { calls.length = 0; },
    };
  })();

  function _call(method, params) {
    return new Promise((resolve, reject) => {
      const id = String(++_reqId);
      _callbacks[id] = { resolve, reject };
      const posted = _postToHost({
        type: '${MSG_REQUEST}',
        pluginId: ${JSON.stringify(pluginId)},
        id: id,
        method: method,
        params: params,
      });
      if (!posted) {
        delete _callbacks[id];
        reject(new Error('Host postMessage unavailable'));
      }
    });
  }

  function _isTrustedHostMessage(e) {
    if (e.source !== parent) return false;
    if (_hostOrigin !== '*' && e.origin && e.origin !== _hostOrigin) return false;
    return true;
  }

  // 监听宿主的响应
  window.addEventListener('message', function(e) {
    if (!_isTrustedHostMessage(e)) return;
    if (e.data && e.data.type === '${MSG_RESPONSE}') {
      const cb = _callbacks[e.data.id];
      if (cb) {
        delete _callbacks[e.data.id];
        if (e.data.error) {
          cb.reject(new Error(e.data.error));
        } else {
          cb.resolve(e.data.result);
        }
      }
    }
    // 事件分发
    if (e.data && e.data.type === '${MSG_EVENT}') {
      _syncChatFromHostEvent(e.data.event, e.data.data);
      _dispatch(e.data.event, e.data.data);
    }
    if (e.data && e.data.type === '${MSG_HOOK_REQUEST}' && e.data.pluginId === ${JSON.stringify(pluginId)}) {
      const hookPayload = e.data.data;
      Promise.resolve()
        .then(function() {
          return _emitAndWait(e.data.event, hookPayload);
        })
        .then(function(result) {
          _postToHost({
            type: '${MSG_HOOK_RESPONSE}',
            pluginId: ${JSON.stringify(pluginId)},
            id: e.data.id,
            result: result === undefined ? hookPayload : result,
          });
        })
        .catch(function(err) {
          _postToHost({
            type: '${MSG_HOOK_RESPONSE}',
            pluginId: ${JSON.stringify(pluginId)},
            id: e.data.id,
            error: String(err && err.message ? err.message : err),
          });
        });
    }
  });

  _loadExtensionSettingsFromHost();
  _postToHost({ type: 'sf:ready', pluginId: ${JSON.stringify(pluginId)} });
})();
<\/script>`
}

// ─── 宿主侧：处理 iframe 请求 ──────────────────────────────────────────────

/**
 * 创建宿主侧的消息处理器
 * @param {Object} plugin - InstalledPluginDto
 * @param {Function} invoke - Tauri invoke 函数
 * @returns {Function} message handler
 */
export function createHostHandler(plugin, invoke, options = {}) {
  const isTrustedSource = options.isTrustedSource || (() => true)
  // 插件本地 storage（宿主侧维护，避免 iframe localStorage 被清除）
  // Persistence adapters: merge injected overrides on top of degraded defaults.
  const defaultAdapters = applyAdapters({
    saveChat: options.saveChatAdapter,
    popup: options.popupAdapter,
    requestHeaders: options.requestHeadersAdapter,
  })
  const saveChatAdapter = defaultAdapters.saveChat
  // The injected adapter can survive a PluginHost remount. Bind its idempotency
  // map to that adapter rather than the short-lived message handler closure.
  const saveChatRequests = saveChatRequestsForAdapter(saveChatAdapter)

  function persistChatIdempotently(params) {
    const key = saveChatSnapshotKey(plugin.id, params?.chat)
    const existing = saveChatRequests.get(key)
    if (existing) return existing

    const pending = Promise.resolve()
      .then(() => saveChatAdapter.saveChat({ pluginId: plugin.id, ...(params || {}) }))
    saveChatRequests.set(key, pending)
    while (saveChatRequests.size > MAX_DEDUPED_SAVE_CHAT_REQUESTS) {
      const oldest = saveChatRequests.keys().next().value
      if (oldest === undefined) break
      saveChatRequests.delete(oldest)
    }
    pending.then(
      (result) => {
        // Failed/degraded outcomes may be retried. Successful outcomes remain
        // cached so a retry after a lost/late response cannot double-write.
        if (result?.ok !== true || result?.degraded) saveChatRequests.delete(key)
      },
      () => saveChatRequests.delete(key),
    )
    return pending
  }

  return async function handleMessage(event) {
    if (!isTrustedSource(event)) return

    const data = event.data
    if (!data || data.type !== MSG_REQUEST || data.pluginId !== plugin.id) return

    const method = API_METHODS[data.method]
    if (!method) {
      postResponse(event, {
        type: MSG_RESPONSE,
        id: data.id,
        error: `未知方法: ${data.method}`,
      })
      return
    }

    // storage 不走后端
    if (data.method === 'storage.get') {
      const key = data.params?.key
      postResponse(event, {
        type: MSG_RESPONSE,
        id: data.id,
        result: readHostPluginStorage(plugin.id, key),
      })
      return
    }
    if (data.method === 'storage.set') {
      const { key, value } = data.params || {}
      writeHostPluginStorage(plugin.id, key, value)
      postResponse(event, {
        type: MSG_RESPONSE,
        id: data.id,
        result: true,
      })
      return
    }
    if (data.method === 'chat.save') {
      // Every same-snapshot retry joins this host-local promise. This protects
      // the actual durable adapter, not just the iframe's late UI response.
      try {
        const result = await persistChatIdempotently(data.params || {})
        postResponse(event, {
          type: MSG_RESPONSE,
          id: data.id,
          result,
        })
      } catch {
        postResponse(event, {
          type: MSG_RESPONSE,
          id: data.id,
          result: { ok: false, degraded: true, reason: 'persist_failed', persistedAt: null },
        })
      }
      return
    }
    if (data.method === 'ui.popup') {
      const popupAdapter = options.popupAdapter || defaultAdapters.popup
      try {
        const value = await popupAdapter.popup(data.params || {}, plugin)
        postResponse(event, {
          type: MSG_RESPONSE,
          id: data.id,
          result: { value, degraded: value === undefined },
        })
      } catch {
        postResponse(event, {
          type: MSG_RESPONSE,
          id: data.id,
          result: { value: undefined, degraded: true },
        })
      }
      return
    }
    if (data.method === 'ui.requestHeaders') {
      const headersAdapter = options.requestHeadersAdapter || defaultAdapters.requestHeaders
      try {
        const headers = headersAdapter.getHeaders()
        postResponse(event, {
          type: MSG_RESPONSE,
          id: data.id,
          result: headers,
        })
      } catch {
        postResponse(event, {
          type: MSG_RESPONSE,
          id: data.id,
          result: { 'Content-Type': 'application/json' },
        })
      }
      return
    }

    // 权限校验
    const permissions = requiredPermissions(method)
    if (!hasAnyPermission(plugin, permissions)) {
      postResponse(event, {
        type: MSG_RESPONSE,
        id: data.id,
        error: `权限不足: 需要 ${permissions.join(' 或 ')}`,
      })
      return
    }

    // 调用 Tauri 后端（unsupported/null command 方法显式报错，不 invoke）
    if (method.unsupported || !method.command) {
      postResponse(event, {
        type: MSG_RESPONSE,
        id: data.id,
        error: method.unsupported || `方法 ${data.method} 在当前宿主不支持`,
      })
      return
    }
    try {
      const params = method.params ? method.params(data.params || {}, plugin.id) : {}
      const result = await invoke(method.command, params)
      postResponse(event, {
        type: MSG_RESPONSE,
        id: data.id,
        result: result,
      })
    } catch (err) {
      postResponse(event, {
        type: MSG_RESPONSE,
        id: data.id,
        error: String(err),
      })
    }
  }
}

export function createPluginHookBridge(plugin, options = {}) {
  const pluginId = plugin?.id
  const getTarget = typeof options.getTarget === 'function' ? options.getTarget : () => null
  const isTrustedSource = options.isTrustedSource || (() => true)
  const targetOrigin = normalizeTargetOrigin(options.targetOrigin)
  // timeoutMs === null disables the bridge-level timer so an outer runtime
  // (promptHooks.js) owns timeout accounting and audits status=timeout.
  // Finite values enable a bridge-local timer (tests / standalone callers).
  // Default remains DEFAULT_PLUGIN_HOOK_TIMEOUT_MS for backward compatibility
  // when production forgets to pass timeoutMs: null.
  let timeoutMs
  if (options.timeoutMs === null) {
    timeoutMs = null
  } else if (Number.isFinite(options.timeoutMs)) {
    timeoutMs = Math.max(0, options.timeoutMs)
  } else {
    timeoutMs = DEFAULT_PLUGIN_HOOK_TIMEOUT_MS
  }
  const onError = typeof options.onError === 'function' ? options.onError : () => {}
  let nextHookId = 0
  const pending = new Map()

  function settle(id, resolver) {
    const entry = pending.get(id)
    if (!entry) return false
    pending.delete(id)
    if (entry.timer) clearTimeout(entry.timer)
    resolver(entry)
    return true
  }

  function fallback(entry, reason) {
    if (reason) onError(reason)
    if (reason && /timed out/i.test(String(reason?.message || reason || ''))) {
      const error = reason instanceof Error ? reason : new Error(String(reason))
      error.code = error.code || 'PROMPT_HOOK_TIMEOUT'
      // Prefer reject so outer withTimeout / audit can classify timeout instead
      // of silently resolving as ok with the fallback payload.
      if (typeof entry.reject === 'function') {
        entry.reject(error)
        return
      }
    }
    entry.resolve(entry.fallback)
  }

  function handleMessage(event) {
    if (!isTrustedSource(event)) return false

    const data = event.data
    if (!data || data.type !== MSG_HOOK_RESPONSE || data.pluginId !== pluginId) return false

    return settle(data.id, (entry) => {
      if (data.error) {
        fallback(entry, new Error(data.error))
      } else {
        entry.resolve(data.result)
      }
    })
  }

  function emitAndWait(eventName, payload = {}) {
    const target = getTarget()
    if (!pluginId || !target || typeof target.postMessage !== 'function') {
      return Promise.resolve(payload)
    }

    const id = String(++nextHookId)
    return new Promise((resolve, reject) => {
      const timer = timeoutMs === null
        ? null
        : setTimeout(() => {
          settle(id, (entry) => fallback(entry, new Error(`Plugin hook timed out: ${eventName}`)))
        }, timeoutMs)
      pending.set(id, { resolve, reject, fallback: payload, timer })
      target.postMessage({
        type: MSG_HOOK_REQUEST,
        pluginId,
        id,
        event: eventName,
        data: payload,
      }, targetOrigin)
    })
  }

  function dispose() {
    for (const [id] of pending) {
      settle(id, (entry) => fallback(entry, null))
    }
  }

  return {
    emitAndWait,
    handleMessage,
    dispose,
  }
}
