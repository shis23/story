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
  committed: ['MESSAGE_RECEIVED'],
  error: ['GENERATION_STOPPED'],
}

// ─── 方法 → 权限 + Tauri 命令映射 ──────────────────────────────────────────

export const API_METHODS = {
  'character.list':   { permission: 'ReadCharacters',  command: 'plugin_list_characters', params: (_p, pluginId) => ({ pluginId }) },
  'character.get':    { permission: 'ReadCharacters',  command: 'plugin_read_character',  params: (p, pluginId) => ({ pluginId, characterId: p.id }) },
  'worldInfo.search': { permission: 'ReadWorldInfo',   command: 'plugin_read_world_info', params: (p, pluginId) => ({ pluginId, characterId: p.characterId }) },
  'memory.getRecent': { permission: 'ReadMemory',      command: 'get_conversation',  params: (p) => ({ id: p.conversationId }) },
  'variables.get':    { permissions: ['ReadVariables', 'WriteVariables'], command: 'plugin_get_variable', params: (p, pluginId) => ({ pluginId, campaignId: p.campaignId, instanceId: p.instanceId }) },
  'variables.set':    { permission: 'WriteVariables',  command: 'plugin_set_variable', params: (p, pluginId) => ({ pluginId, campaignId: p.campaignId, instanceId: p.instanceId, key: p.key, value: p.value }) },
  'storage.get':      { permission: null,              command: null },  // 本地 localStorage，不走后端
  'storage.set':      { permission: null,              command: null },
  'llm.generate':     { permission: 'CallLlm',         command: 'start_writing',     params: (p) => ({ intent: p.intent ?? p.prompt ?? '' }) },
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
  'displayContent',
  'display_content',
  'text',
  'token',
  'delta',
  'messages',
  'prompt',
  'intent',
  'raw',
])

function sanitizePluginEventData(value) {
  if (Array.isArray(value)) {
    return value.map((item) => sanitizePluginEventData(item))
  }
  if (!value || typeof value !== 'object') {
    return value
  }

  const sanitized = {}
  for (const [key, child] of Object.entries(value)) {
    if (SENSITIVE_EVENT_FIELDS.has(key)) continue
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
  } else if (pipelineEvent.event_type === 'error') {
    payload.message = eventData.message || ''
  }

  return payload
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
    ...(ST_EVENT_ALIASES[pipelineEvent.event_type] || []),
  ]
  const seen = new Set()

  return names
    .filter((name) => {
      if (seen.has(name)) return false
      seen.add(name)
      return isSubscribedToPluginEvent(plugin, name)
    })
    .map((name) => ({ event: name, data: payload }))
}

function mapGenericPluginEvent(eventName, data, plugin = null) {
  if (!eventName) return []
  if (!isSubscribedToPluginEvent(plugin, eventName)) return []
  const payload = data && typeof data === 'object' ? data : {}
  return [{
    event: eventName,
    data: canReadMemory(plugin) ? payload : sanitizePluginEventData(payload),
  }]
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
  event.source?.postMessage(payload, responseTargetOrigin(event))
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
    parent.postMessage({
      type: '${MSG_MOUNT}',
      pluginId: ${JSON.stringify(pluginId)},
      slot: _normalizeUiSlot(slotName),
      html: html,
    }, _hostOrigin);
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

  function _invokeSlashCommand(name, args) {
    const command = _slashCommands.find(function(item) {
      return item.name === name || item.aliases.indexOf(name) >= 0;
    });
    if (!command) return _invokeBuiltinSlashCommand(name, args);
    return command.callback.apply(null, args);
  }

  function _invokeBuiltinSlashCommand(name, args) {
    if (name === 'genraw') {
      return window.storyforge.llm.generate(args && args.length ? args[0] : '');
    }
    return undefined;
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
    let previousResult;
    let chain = null;
    function runSegment(index, resolvedPipe) {
      const parsed = _parseSlashInvocation(segments[index]);
      if (!parsed) return resolvedPipe;
      if (index > 0) {
        parsed.pipe = resolvedPipe;
        parsed.previousResult = resolvedPipe;
      }
      return _invokeSlashCommand(parsed.name, [parsed.args, parsed]);
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
    return _invokeSlashCommand(trimmedName, args);
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
    if (['local', 'chat', 'global'].includes(value.type)) {
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

  function _readSelectorVariables(selector) {
    try {
      const parsed = JSON.parse(localStorage.getItem(_variableSelectorKey(selector)) || '{}');
      return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed : {};
    } catch {
      return {};
    }
  }

  function _writeSelectorVariables(selector, variables) {
    const next = variables && typeof variables === 'object' && !Array.isArray(variables) ? variables : {};
    localStorage.setItem(_variableSelectorKey(selector), JSON.stringify(next));
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

  function _getLastMessageId() {
    return _chat.length - 1;
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

  function _saveChat() {
    return Promise.resolve(true);
  }

  function _callGenericPopup(html, type, defaultValue) {
    if (defaultValue !== undefined) return Promise.resolve(String(defaultValue));
    return Promise.resolve('');
  }

  function _getRequestHeaders() {
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

  function _getContext() {
    return {
      chat: _chat,
      name1: window.SillyTavern?.name1 || 'User',
      name2: window.SillyTavern?.name2 || 'Assistant',
      characters: [],
      groups: [],
      extensionSettings: window.extension_settings,
      extension_settings: window.extension_settings,
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
      POPUP_TYPE: {
        INPUT: 'input',
        CONFIRM: 'confirm',
        TEXT: 'text',
        DISPLAY: 'display',
      },
      getContext: _getContext,
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
      get: (key) => {
        try { return JSON.parse(localStorage.getItem('sf_plugin_' + ${JSON.stringify(pluginId)} + '_' + key)); }
        catch { return null; }
      },
      set: (key, value) => {
        localStorage.setItem('sf_plugin_' + ${JSON.stringify(pluginId)} + '_' + key, JSON.stringify(value));
      },
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
      trigger: _triggerSlashCommand,
    },

    statusBar: {
      set: (html) => _mountToSlot('statusbar', html),
      clear: () => _mountToSlot('statusbar', ''),
    },
  };

  window.extension_settings = window.extension_settings || {};
  window.extension_settings.storyforge = window.extension_settings.storyforge || {};
  window.extension_settings.storyforge.statusBar = window.extension_settings.storyforge.statusBar || {};
  window.extension_settings.storyforge.status_bar = window.extension_settings.storyforge.statusBar;

  window.event_types = _eventTypes;
  window.eventTypes = _eventTypes;
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
  window.triggerSlashCommand = _triggerSlashCommand;
  window.triggerSlash = _triggerSlashCommand;
  window.triggerSlashTag = _triggerSlashCommand;
  window.SlashCommand = window.SlashCommand || {
    fromProps: function(props) { return props || {}; },
  };
  window.SlashCommandParser = window.SlashCommandParser || {};
  window.SlashCommandParser.commands = _slashCommands;
  window.SlashCommandParser.addCommandObject = function(command) {
    return _registerSlashCommand(command);
  };
  window.TavernHelper = window.TavernHelper || _createTavernHelper();
  window.tavernHelper = window.TavernHelper;
  window.SillyTavern = window.SillyTavern || _createSillyTavern();
  if (!Array.isArray(window.SillyTavern.chat)) window.SillyTavern.chat = _chat;
  window.SillyTavern.chat = window.SillyTavern.chat || _chat;
  window.SillyTavern.saveChat = window.SillyTavern.saveChat || _saveChat;
  window.SillyTavern.callGenericPopup = window.SillyTavern.callGenericPopup || _callGenericPopup;
  window.SillyTavern.getRequestHeaders = window.SillyTavern.getRequestHeaders || _getRequestHeaders;
  window.SillyTavern.getContext = window.SillyTavern.getContext || _getContext;
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
  window.setChatMessage = window.setChatMessage || window.TavernHelper.setChatMessage;
  window.getContext = window.getContext || window.TavernHelper.getContext;
  window.registerMacro = window.registerMacro || window.TavernHelper.registerMacro;
  window.unregisterMacro = window.unregisterMacro || window.TavernHelper.unregisterMacro;
  window.saveSettingsDebounced = window.saveSettingsDebounced || function() {};

  function _call(method, params) {
    return new Promise((resolve, reject) => {
      const id = String(++_reqId);
      _callbacks[id] = { resolve, reject };
      parent.postMessage({
        type: '${MSG_REQUEST}',
        pluginId: ${JSON.stringify(pluginId)},
        id: id,
        method: method,
        params: params,
      }, _hostOrigin);
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
      _dispatch(e.data.event, e.data.data);
    }
    if (e.data && e.data.type === '${MSG_HOOK_REQUEST}' && e.data.pluginId === ${JSON.stringify(pluginId)}) {
      const hookPayload = e.data.data;
      Promise.resolve()
        .then(function() {
          return _emitAndWait(e.data.event, hookPayload);
        })
        .then(function(result) {
          parent.postMessage({
            type: '${MSG_HOOK_RESPONSE}',
            pluginId: ${JSON.stringify(pluginId)},
            id: e.data.id,
            result: result === undefined ? hookPayload : result,
          }, _hostOrigin);
        })
        .catch(function(err) {
          parent.postMessage({
            type: '${MSG_HOOK_RESPONSE}',
            pluginId: ${JSON.stringify(pluginId)},
            id: e.data.id,
            error: String(err && err.message ? err.message : err),
          }, _hostOrigin);
        });
    }
  });

  // 通知宿主 iframe 已加载
  parent.postMessage({ type: 'sf:ready', pluginId: ${JSON.stringify(pluginId)} }, _hostOrigin);
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
  const pluginStorage = {}

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
        result: pluginStorage[key] ?? null,
      })
      return
    }
    if (data.method === 'storage.set') {
      const { key, value } = data.params || {}
      pluginStorage[key] = value
      postResponse(event, {
        type: MSG_RESPONSE,
        id: data.id,
        result: true,
      })
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

    // 调用 Tauri 后端
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
  const timeoutMs = Number.isFinite(options.timeoutMs)
    ? Math.max(0, options.timeoutMs)
    : DEFAULT_PLUGIN_HOOK_TIMEOUT_MS
  const onError = typeof options.onError === 'function' ? options.onError : () => {}
  let nextHookId = 0
  const pending = new Map()

  function settle(id, resolver) {
    const entry = pending.get(id)
    if (!entry) return false
    pending.delete(id)
    clearTimeout(entry.timer)
    resolver(entry)
    return true
  }

  function fallback(entry, reason) {
    if (reason) onError(reason)
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
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        settle(id, (entry) => fallback(entry, new Error(`Plugin hook timed out: ${eventName}`)))
      }, timeoutMs)
      pending.set(id, { resolve, fallback: payload, timer })
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
