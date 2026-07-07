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
export const MSG_MOUNT = 'sf:ui:mount'

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
  'character.list':   { permission: 'ReadCharacters',  command: 'list_characters' },
  'character.get':    { permission: 'ReadCharacters',  command: 'get_character',     params: (p) => ({ id: p.id }) },
  'worldInfo.search': { permission: 'ReadWorldInfo',   command: 'get_character',     params: (p) => ({ id: p.characterId }) },
  'memory.getRecent': { permission: 'ReadMemory',      command: 'get_conversation',  params: (p) => ({ id: p.conversationId }) },
  'variables.get':    { permission: 'WriteVariables',  command: 'get_character_variables', params: (p) => ({ campaignId: p.campaignId, instanceId: p.instanceId }) },
  'variables.set':    { permission: 'WriteVariables',  command: 'set_character_variable', params: (p) => ({ campaignId: p.campaignId, instanceId: p.instanceId, key: p.key, value: p.value }) },
  'storage.get':      { permission: null,              command: null },  // 本地 localStorage，不走后端
  'storage.set':      { permission: null,              command: null },
  'llm.generate':     { permission: 'CallLlm',         command: 'start_writing',     params: (p) => ({ prompt: p.prompt }) },
}

// ─── PipelineEvent → 插件事件映射 ─────────────────────────────────────────

function createPluginEventPayload(pipelineEvent) {
  const data = pipelineEvent?.data && typeof pipelineEvent.data === 'object'
    ? pipelineEvent.data
    : {}
  const payload = {
    ...data,
    event_type: pipelineEvent.event_type,
    data,
    raw: pipelineEvent,
  }

  if (pipelineEvent.event_type === 'editor_progress') {
    payload.token = data.delta || ''
    payload.text = data.delta || ''
  } else if (pipelineEvent.event_type === 'draft_ready') {
    payload.text = data.text || ''
  } else if (pipelineEvent.event_type === 'error') {
    payload.message = data.message || ''
  }

  return payload
}

/**
 * 将 Tauri WritingEvent 映射为插件可订阅事件。
 *
 * 同时发送 StoryForge 原生事件名（pipeline.xxx / xxx）和少量 ST 常用别名；
 * ST 99 事件全集仍由后续兼容层继续补齐。
 */
export function mapPipelineEventToPluginEvents(pipelineEvent) {
  if (!pipelineEvent?.event_type) return []

  const payload = createPluginEventPayload(pipelineEvent)
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
      return true
    })
    .map((name) => ({ event: name, data: payload }))
}

function mapGenericPluginEvent(eventName, data) {
  if (!eventName) return []
  return [{ event: eventName, data: data && typeof data === 'object' ? data : {} }]
}

/**
 * 将 App.vue 事件 feed 中的一条记录规范化为 PluginHost 可发送的事件数组。
 * 支持历史的 PipelineEvent 记录，也支持 `CHAT_CHANGED` 等宿主通用事件。
 */
export function mapPluginEventRecordToPluginEvents(record) {
  if (!record) return []
  if (record.event_type) return mapPipelineEventToPluginEvents(record)

  const event = record.event
  if (event?.event_type) return mapPipelineEventToPluginEvents(event)
  if (typeof event === 'string') return mapGenericPluginEvent(event, record.data)
  if (typeof record.name === 'string') return mapGenericPluginEvent(record.name, record.data)
  if (typeof event?.name === 'string') return mapGenericPluginEvent(event.name, event.data ?? record.data)
  if (typeof event?.event === 'string') return mapGenericPluginEvent(event.event, event.data ?? record.data)

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
    return listeners.reduce(function(chain, fn) {
      return chain.then(function() {
        return fn.apply(null, args);
      });
    }, Promise.resolve()).then(function() {});
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

  function _triggerSlashCommand(name) {
    const args = Array.prototype.slice.call(arguments, 1);
    const command = _slashCommands.find(function(item) {
      return item.name === name || item.aliases.indexOf(name) >= 0;
    });
    if (!command) return undefined;
    return command.callback.apply(null, args);
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
      generate: (prompt) => _call('llm.generate', { prompt }),
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

  // 监听宿主的响应
  window.addEventListener('message', function(e) {
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
    if (method.permission && !plugin.permissions?.includes(method.permission)) {
      postResponse(event, {
        type: MSG_RESPONSE,
        id: data.id,
        error: `权限不足: 需要 ${method.permission}`,
      })
      return
    }

    // 调用 Tauri 后端
    try {
      const params = method.params ? method.params(data.params) : {}
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
