// 生成文件，不要手改：node integrations/dsh-storyforge/scripts/build-vendor.mjs
// 来源：integrations/dsh-storyforge/shell/entry.mjs 列出的桌面前端模块（esbuild 0.25.12）。

// frontend/src/utils/pluginPersistence.js
var PERSISTENCE_DEGRADED_REASON = "local_mirror_only_no_host_persist";
var HEADERS_DEGRADED_REASON = "static_json_content_type_only";
var POPUP_TYPES = Object.freeze({
  TEXT: 1,
  CONFIRM: 2,
  INPUT: 3,
  DISPLAY: 4,
  CROP: 5
});
var FORBIDDEN_HEADER_KEY_FRAGMENTS = [
  "authorization",
  "apikey",
  "proxyauthorization",
  "proxyauthenticate",
  "wwwauthenticate",
  "cookie",
  "setcookie",
  "authtoken",
  "accessToken".toLowerCase(),
  "refreshtoken",
  "secret",
  "password",
  "credential",
  "bearer"
];
function isCancelled(signal) {
  return Boolean(signal?.aborted);
}
function makeCancelledResult(reason = "cancelled") {
  return { ok: false, cancelled: true, degraded: false, reason, persistedAt: null };
}
function makeDegradedFallback(reason, detail) {
  return { ok: false, cancelled: false, degraded: true, reason, detail: detail || null, persistedAt: null };
}
function createSaveChatAdapter(options = {}) {
  const persist = typeof options.persist === "function" ? options.persist : null;
  const chatSnapshot = typeof options.chatSnapshot === "function" ? options.chatSnapshot : (snapshot) => snapshot;
  async function saveChat(payload = {}, adapterOptions = {}) {
    if (isCancelled(adapterOptions.signal)) return makeCancelledResult();
    const snapshot = chatSnapshot(payload);
    if (!persist) {
      return { ok: true, degraded: true, reason: PERSISTENCE_DEGRADED_REASON, cancelled: false, persistedAt: null };
    }
    try {
      const result = await persist(snapshot);
      if (isCancelled(adapterOptions.signal)) return makeCancelledResult();
      if (result && result.ok) {
        return {
          ok: true,
          degraded: false,
          cancelled: false,
          reason: null,
          persistedAt: result.persistedAt ?? null
        };
      }
      return makeDegradedFallback(
        result?.reason || "persist_rejected",
        result?.detail || null
      );
    } catch {
      return makeDegradedFallback("persist_failed", null);
    }
  }
  return { saveChat };
}
function createDefaultSaveChatAdapter() {
  return createSaveChatAdapter({});
}
function createPopupAdapter(options = {}) {
  const isAuthorized = typeof options.isAuthorized === "function" ? options.isAuthorized : null;
  const render = typeof options.render === "function" ? options.render : null;
  async function popup(request = {}, pluginContext = null) {
    if (isAuthorized && !isAuthorized(pluginContext)) {
      return void 0;
    }
    if (render) {
      try {
        return await render(request);
      } catch {
        return void 0;
      }
    }
    return degradedPopupDefault(request);
  }
  return { popup };
}
function createDefaultPopupAdapter() {
  return createPopupAdapter({});
}
function degradedPopupDefault(request = {}) {
  const type = request.type;
  if (request.defaultValue !== void 0) return Promise.resolve(String(request.defaultValue));
  if (type === POPUP_TYPES.CONFIRM || String(type || "").toLowerCase() === "confirm") {
    return Promise.resolve(null);
  }
  return Promise.resolve("");
}
function createRequestHeadersAdapter(options = {}) {
  const getHeaders = typeof options.getHeaders === "function" ? options.getHeaders : null;
  function resolve() {
    let raw = {};
    if (getHeaders) {
      try {
        raw = getHeaders() || {};
      } catch {
        raw = {};
      }
    } else {
      raw = { "Content-Type": "application/json" };
    }
    return redactHeaders(raw);
  }
  return { getHeaders: resolve, degradedReason: getHeaders ? null : HEADERS_DEGRADED_REASON };
}
function createDefaultRequestHeadersAdapter() {
  return createRequestHeadersAdapter({});
}
function isForbiddenHeaderKey(key) {
  const normalized = String(key || "").toLowerCase().replace(/[^a-z0-9]/g, "");
  if (!normalized) return true;
  return FORBIDDEN_HEADER_KEY_FRAGMENTS.some((fragment) => normalized.includes(fragment));
}
function redactHeaders(headers) {
  const out = {};
  for (const [key, value] of Object.entries(headers || {})) {
    if (isForbiddenHeaderKey(key)) continue;
    if (typeof value === "string" && /bearer\s|sk-|SF_SECRET_|api[_-]?key|authorization|proxy-?authorization/i.test(value)) {
      continue;
    }
    out[key] = value;
  }
  if (!out["Content-Type"]) out["Content-Type"] = "application/json";
  return out;
}
function applyPersistenceAdapters(overrides = {}) {
  return {
    saveChat: overrides.saveChat || createDefaultSaveChatAdapter(),
    popup: overrides.popup || createDefaultPopupAdapter(),
    requestHeaders: overrides.requestHeaders || createDefaultRequestHeadersAdapter()
  };
}

// frontend/src/plugin-bridge.js
var MSG_REQUEST = "sf:api:request";
var MSG_RESPONSE = "sf:api:response";
var MSG_EVENT = "sf:api:event";
var MSG_HOOK_REQUEST = "sf:hook:request";
var MSG_HOOK_RESPONSE = "sf:hook:response";
var MSG_MOUNT = "sf:ui:mount";
var WRITE_CHAT_PERMISSION = "WriteChat";
var ST_EVENT_TYPES = Object.freeze({
  APP_READY: "APP_READY",
  CHAT_CHANGED: "CHAT_CHANGED",
  CHAT_LOADED: "CHAT_LOADED",
  MESSAGE_RECEIVED: "MESSAGE_RECEIVED",
  MESSAGE_SENT: "MESSAGE_SENT",
  MESSAGE_UPDATED: "MESSAGE_UPDATED",
  MESSAGE_DELETED: "MESSAGE_DELETED",
  MESSAGE_SWIPED: "MESSAGE_SWIPED",
  GENERATION_STARTED: "GENERATION_STARTED",
  GENERATION_STOPPED: "GENERATION_STOPPED",
  GENERATION_ENDED: "GENERATION_ENDED",
  STREAM_TOKEN: "STREAM_TOKEN",
  CHARACTER_LOADED: "CHARACTER_LOADED",
  CHARACTER_MESSAGE_RENDERED: "CHARACTER_MESSAGE_RENDERED",
  USER_MESSAGE_RENDERED: "USER_MESSAGE_RENDERED",
  WORLDINFO_SETTINGS_UPDATED: "WORLDINFO_SETTINGS_UPDATED",
  WORLDINFO_UPDATED: "WORLDINFO_UPDATED",
  WORLDINFO_FORCE_ACTIVATE: "WORLDINFO_FORCE_ACTIVATE",
  GENERATE_BEFORE_COMBINE_PROMPTS: "GENERATE_BEFORE_COMBINE_PROMPTS",
  GENERATE_AFTER_COMBINE_PROMPTS: "GENERATE_AFTER_COMBINE_PROMPTS",
  CHAT_COMPLETION_PROMPT_READY: "CHAT_COMPLETION_PROMPT_READY",
  TOOL_CALLS_PERFORMED: "TOOL_CALLS_PERFORMED",
  TOOL_CALLS_RENDERED: "TOOL_CALLS_RENDERED",
  GROUP_UPDATED: "GROUP_UPDATED",
  GROUP_MEMBER_DRAFTED: "GROUP_MEMBER_DRAFTED",
  GROUP_WRAPPER_FINISHED: "GROUP_WRAPPER_FINISHED",
  SETTINGS_LOADED: "SETTINGS_LOADED",
  SETTINGS_UPDATED: "SETTINGS_UPDATED",
  EXTENSION_SETTINGS_LOADED: "EXTENSION_SETTINGS_LOADED",
  EXTENSIONS_FIRST_LOAD: "EXTENSIONS_FIRST_LOAD"
});
var HOST_PLUGIN_STORAGE_FALLBACK = /* @__PURE__ */ new Map();
var HOST_SAVE_CHAT_REQUESTS_BY_ADAPTER = /* @__PURE__ */ new WeakMap();
var MAX_DEDUPED_SAVE_CHAT_REQUESTS = 128;
var MAX_PLUGIN_STORAGE_VALUE_CHARS = 256 * 1024;
var MAX_PLUGIN_STORAGE_TOTAL_CHARS = 1024 * 1024;
function hostPluginStorageKey(pluginId, key) {
  return [
    "sf_host_plugin_storage",
    encodeURIComponent(String(pluginId)),
    encodeURIComponent(String(key))
  ].join(":");
}
function legacyHostPluginStorageKey(pluginId, key) {
  return `sf_host_plugin_storage_${pluginId}_${key}`;
}
function saveChatRequestsForAdapter(adapter) {
  let requests = HOST_SAVE_CHAT_REQUESTS_BY_ADAPTER.get(adapter);
  if (!requests) {
    requests = /* @__PURE__ */ new Map();
    HOST_SAVE_CHAT_REQUESTS_BY_ADAPTER.set(adapter, requests);
  }
  return requests;
}
function measurePluginStorageValue(value) {
  try {
    const serialized = JSON.stringify(value);
    return serialized === void 0 ? 0 : serialized.length;
  } catch {
    return 0;
  }
}
function pluginStorageUsageChars(pluginId) {
  const keyPrefix = `${hostPluginStorageKey(pluginId, "")}`;
  let total = 0;
  const addValue = (raw) => {
    if (typeof raw === "string") total += raw.length;
  };
  let usedLocalStorage = false;
  try {
    const storage = typeof globalThis !== "undefined" ? globalThis.localStorage : null;
    if (storage && typeof storage.length === "number" && typeof storage.key === "function") {
      usedLocalStorage = true;
      for (let index = 0; index < storage.length; index += 1) {
        const storageKey = storage.key(index);
        if (typeof storageKey === "string" && storageKey.startsWith(keyPrefix)) {
          addValue(storage.getItem(storageKey));
        }
      }
    }
  } catch {
    usedLocalStorage = false;
  }
  if (!usedLocalStorage) {
    for (const [storageKey, value] of HOST_PLUGIN_STORAGE_FALLBACK) {
      if (storageKey.startsWith(keyPrefix)) addValue(JSON.stringify(value) ?? "");
    }
  }
  return total;
}
function checkPluginStorageQuota(pluginId, key, value, previousValue = null) {
  const currentChars = measurePluginStorageValue(value);
  if (currentChars > MAX_PLUGIN_STORAGE_VALUE_CHARS) {
    return `插件存储单键超限: ${key} 需要 ${currentChars} 字符，上限 ${MAX_PLUGIN_STORAGE_VALUE_CHARS}`;
  }
  const replacedChars = measurePluginStorageValue(previousValue);
  const totalChars = pluginStorageUsageChars(pluginId) - replacedChars + currentChars;
  if (totalChars > MAX_PLUGIN_STORAGE_TOTAL_CHARS) {
    return `插件存储总量超限: 需要 ${totalChars} 字符，上限 ${MAX_PLUGIN_STORAGE_TOTAL_CHARS}`;
  }
  return null;
}
function readHostPluginStorageByKey(storageKey) {
  try {
    if (typeof globalThis !== "undefined" && globalThis.localStorage) {
      const raw = globalThis.localStorage.getItem(storageKey);
      return raw === null ? null : JSON.parse(raw);
    }
  } catch {
  }
  return HOST_PLUGIN_STORAGE_FALLBACK.has(storageKey) ? HOST_PLUGIN_STORAGE_FALLBACK.get(storageKey) : null;
}
function readHostPluginStorage(pluginId, key) {
  const storageKey = hostPluginStorageKey(pluginId, key);
  const value = readHostPluginStorageByKey(storageKey);
  if (value !== null) return value;
  const legacyValue = readHostPluginStorageByKey(legacyHostPluginStorageKey(pluginId, key));
  if (legacyValue !== null) {
    writeHostPluginStorage(pluginId, key, legacyValue);
  }
  return legacyValue;
}
function writeHostPluginStorage(pluginId, key, value) {
  const storageKey = hostPluginStorageKey(pluginId, key);
  try {
    if (typeof globalThis !== "undefined" && globalThis.localStorage) {
      globalThis.localStorage.setItem(storageKey, JSON.stringify(value));
    }
  } catch {
  }
  HOST_PLUGIN_STORAGE_FALLBACK.set(storageKey, value);
}
function saveChatSnapshotKey(pluginId, chat) {
  let snapshot = "";
  try {
    snapshot = JSON.stringify(chat ?? []);
  } catch {
    snapshot = "[unserializable-chat]";
  }
  let hash = 2166136261;
  for (let index = 0; index < snapshot.length; index += 1) {
    hash ^= snapshot.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return `${String(pluginId)}:${(hash >>> 0).toString(16)}:${snapshot.length}`;
}
var API_METHODS = {
  "character.list": { permission: "ReadCharacters", command: "plugin_list_characters", params: (_p, pluginId) => ({ pluginId }) },
  "character.get": { permission: "ReadCharacters", command: "plugin_read_character", params: (p, pluginId) => ({ pluginId, characterId: p.id }) },
  "worldInfo.search": { permission: "ReadWorldInfo", command: "plugin_read_world_info", params: (p, pluginId) => ({ pluginId, characterId: p.characterId }) },
  // L4 严格门禁：走插件通道命令，后端 PluginRegistry 二次校验 ReadMemory。
  // 未注册的插件（含卡壳虚拟插件）后端直接拒绝，前端权限数组不再是唯一边界。
  "memory.getRecent": { permission: "ReadMemory", command: "plugin_get_conversation", params: (p, pluginId) => ({ pluginId, id: p.conversationId }) },
  // Gate 8 审查 P2-C1: 读=读、写=写——仅持 WriteVariables 的插件不得读变量。
  "variables.get": { permission: "ReadVariables", command: "plugin_get_variable", params: (p, pluginId) => ({ pluginId, campaignId: p.campaignId, instanceId: p.instanceId }) },
  "variables.set": { permission: "WriteVariables", command: "plugin_set_variable", params: (p, pluginId) => ({ pluginId, campaignId: p.campaignId, instanceId: p.instanceId, key: p.key, value: p.value }) },
  "storage.get": { permission: null, command: null },
  // 本地 localStorage，不走后端
  "storage.set": { permission: null, command: null },
  // 本地 localStorage，不走后端（配额门见 createHostHandler）
  // Host-side adapter routes (no backend command). Defaults are degraded shims;
  // production can inject real adapters without editing tauri-app storage code.
  // M-31c：chat.save 无权限门控时是「无 ReadMemory 也能用的写通道」——adapter
  // 换成真实持久化后即成为插件直接改写宿主聊天的通道，故补独立授权位。
  "chat.save": { permission: WRITE_CHAT_PERMISSION, command: null },
  "ui.popup": { permission: null, command: null },
  "ui.requestHeaders": { permission: null, command: null },
  // Gate 8 复评：后端 start_writing 的 on_event 是必填 Channel，插件沙箱
  // 无法提供事件通道——该 API 在生产必然失败（Tauri 参数反序列化缺必填
  // Channel 直接报错）。显式标记 unsupported，返回清晰错误而非 invoke 一个
  // 注定失败的调用（旧测试曾固化「只传 intent」的错误契约）。
  "llm.generate": { permission: "CallLlm", command: null, unsupported: "llm.generate 不支持：start_writing 需要宿主注入 onEvent 事件通道，插件通道无法提供；请使用宿主写作流程" }
};
function requiredPermissions(method) {
  if (Array.isArray(method.permissions)) return method.permissions;
  return method.permission ? [method.permission] : [];
}
function hasAnyPermission(plugin, permissions) {
  if (!permissions.length) return true;
  return permissions.some((permission) => plugin.permissions?.includes(permission));
}
function defaultHostOrigin() {
  if (typeof window !== "undefined" && window.location?.origin) {
    return window.location.origin;
  }
  return "*";
}
function normalizeTargetOrigin(origin) {
  return typeof origin === "string" && origin.trim() ? origin.trim() : "*";
}
function responseTargetOrigin(event) {
  const origin = event?.origin;
  return origin && origin !== "null" ? origin : "*";
}
function postResponse(event, payload) {
  if (event?.source && typeof event.source.postMessage === "function") {
    event.source.postMessage(payload, responseTargetOrigin(event));
  }
}
function generateBridgeScript(pluginId, hostOrigin = defaultHostOrigin(), nonce = "") {
  const targetOrigin = normalizeTargetOrigin(hostOrigin);
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
  _postToHost({ type: 'sf:ready', pluginId: ${JSON.stringify(pluginId)}, handshake: ${JSON.stringify(String(nonce || ""))} });
})();
</script>`;
}
function createHostHandler(plugin, invoke, options = {}) {
  const isTrustedSource = options.isTrustedSource || (() => true);
  const defaultAdapters = applyPersistenceAdapters({
    saveChat: options.saveChatAdapter,
    popup: options.popupAdapter,
    requestHeaders: options.requestHeadersAdapter
  });
  const saveChatAdapter = defaultAdapters.saveChat;
  const saveChatRequests = saveChatRequestsForAdapter(saveChatAdapter);
  function persistChatIdempotently(params) {
    const key = saveChatSnapshotKey(plugin.id, params?.chat);
    const existing = saveChatRequests.get(key);
    if (existing) return existing;
    const pending = Promise.resolve().then(() => saveChatAdapter.saveChat({ pluginId: plugin.id, ...params || {} }));
    saveChatRequests.set(key, pending);
    while (saveChatRequests.size > MAX_DEDUPED_SAVE_CHAT_REQUESTS) {
      const oldest = saveChatRequests.keys().next().value;
      if (oldest === void 0) break;
      saveChatRequests.delete(oldest);
    }
    pending.then(
      (result) => {
        if (result?.ok !== true || result?.degraded) saveChatRequests.delete(key);
      },
      () => saveChatRequests.delete(key)
    );
    return pending;
  }
  return async function handleMessage(event) {
    if (!isTrustedSource(event)) return;
    const data = event.data;
    if (!data || data.type !== MSG_REQUEST || data.pluginId !== plugin.id) return;
    const method = API_METHODS[data.method];
    if (!method) {
      postResponse(event, {
        type: MSG_RESPONSE,
        id: data.id,
        error: `未知方法: ${data.method}`
      });
      return;
    }
    if (data.method === "storage.get") {
      const key = data.params?.key;
      postResponse(event, {
        type: MSG_RESPONSE,
        id: data.id,
        result: readHostPluginStorage(plugin.id, key)
      });
      return;
    }
    if (data.method === "storage.set") {
      const { key, value } = data.params || {};
      const quotaError = checkPluginStorageQuota(
        plugin.id,
        key,
        value,
        readHostPluginStorage(plugin.id, key)
      );
      if (quotaError) {
        postResponse(event, {
          type: MSG_RESPONSE,
          id: data.id,
          error: quotaError
        });
        return;
      }
      writeHostPluginStorage(plugin.id, key, value);
      postResponse(event, {
        type: MSG_RESPONSE,
        id: data.id,
        result: true
      });
      return;
    }
    if (data.method === "ui.popup") {
      const popupAdapter = options.popupAdapter || defaultAdapters.popup;
      try {
        const value = await popupAdapter.popup(data.params || {}, plugin);
        postResponse(event, {
          type: MSG_RESPONSE,
          id: data.id,
          result: { value, degraded: value === void 0 }
        });
      } catch {
        postResponse(event, {
          type: MSG_RESPONSE,
          id: data.id,
          result: { value: void 0, degraded: true }
        });
      }
      return;
    }
    if (data.method === "ui.requestHeaders") {
      const headersAdapter = options.requestHeadersAdapter || defaultAdapters.requestHeaders;
      try {
        const headers = headersAdapter.getHeaders();
        postResponse(event, {
          type: MSG_RESPONSE,
          id: data.id,
          result: headers
        });
      } catch {
        postResponse(event, {
          type: MSG_RESPONSE,
          id: data.id,
          result: { "Content-Type": "application/json" }
        });
      }
      return;
    }
    const permissions = requiredPermissions(method);
    if (!hasAnyPermission(plugin, permissions)) {
      postResponse(event, {
        type: MSG_RESPONSE,
        id: data.id,
        error: `权限不足: 需要 ${permissions.join(" 或 ")}`
      });
      return;
    }
    if (data.method === "chat.save") {
      try {
        const result = await persistChatIdempotently(data.params || {});
        postResponse(event, {
          type: MSG_RESPONSE,
          id: data.id,
          result
        });
      } catch {
        postResponse(event, {
          type: MSG_RESPONSE,
          id: data.id,
          result: { ok: false, degraded: true, reason: "persist_failed", persistedAt: null }
        });
      }
      return;
    }
    if (method.unsupported || !method.command) {
      postResponse(event, {
        type: MSG_RESPONSE,
        id: data.id,
        error: method.unsupported || `方法 ${data.method} 在当前宿主不支持`
      });
      return;
    }
    try {
      const params = method.params ? method.params(data.params || {}, plugin.id) : {};
      const result = await invoke(method.command, params);
      postResponse(event, {
        type: MSG_RESPONSE,
        id: data.id,
        result
      });
    } catch (err) {
      postResponse(event, {
        type: MSG_RESPONSE,
        id: data.id,
        error: String(err)
      });
    }
  };
}

// frontend/src/utils/cardShellDocument.js
function rewriteCardShellTopBridgeAccess(html) {
  return String(html || "").replace(
    /window\s*\.\s*top\s*(?:\?\.\s*|\.\s*)(TavernHelper|tavernHelper|SillyTavern)\b/g,
    (_match, apiRoot) => `window.${apiRoot}`
  );
}
function createCardShellRuntimeCompatibilityScript(options = {}) {
  const scriptJson = (value) => JSON.stringify(value).replace(/</g, "\\u003c").replace(/>/g, "\\u003e").replace(/&/g, "\\u0026").replace(/\u2028/g, "\\u2028").replace(/\u2029/g, "\\u2029");
  const worldbookName = scriptJson(String(options.worldbookName || ""));
  const selectorVariables = scriptJson(options.selectorVariables && typeof options.selectorVariables === "object" ? options.selectorVariables : {});
  const mvuStatData = scriptJson(options.mvuStatData && typeof options.mvuStatData === "object" ? options.mvuStatData : {});
  return [
    "(function(){",
    `  var WORLD_BOOK_NAME = ${worldbookName};`,
    `  var SELECTOR_VARIABLES = ${selectorVariables};`,
    // M5：真实 Campaign 变量树（stat_data 底座，只读）。构建时注入快照，
    // mvu_data_get 桥可刷新；壳自写的选择器桶按键覆盖其上。
    `  var MVU_STAT_DATA = ${mvuStatData};`,
    "  function bridge(type, payload){",
    '    if (typeof window.__sfShellAsk !== "function") {',
    '      return Promise.reject(new Error("StoryForge card shell bridge is unavailable"));',
    "    }",
    "    return window.__sfShellAsk(type, payload || {});",
    "  }",
    // The opaque sandbox origin makes reading `localStorage` / `sessionStorage`
    // throw a SecurityError, which aborts card modules that cache UI drafts there
    // (Destiny's custom-start shell). Swap in a per-document in-memory Storage so
    // the card runs; nothing is shared with the host or kept across reloads.
    "  function memoryStorage(){",
    "    var data = new Map();",
    "    var api = {",
    "      key: function(index){ var keys = Array.from(data.keys()); return index >= 0 && index < keys.length ? keys[index] : null; },",
    "      getItem: function(key){ key = String(key); return data.has(key) ? data.get(key) : null; },",
    "      setItem: function(key, value){ data.set(String(key), String(value)); },",
    "      removeItem: function(key){ data.delete(String(key)); },",
    "      clear: function(){ data.clear(); }",
    "    };",
    '    Object.defineProperty(api, "length", { get: function(){ return data.size; } });',
    "    return api;",
    "  }",
    '  ["localStorage", "sessionStorage"].forEach(function(name){',
    "    var usable = false;",
    "    try { usable = !!window[name]; } catch (_) { usable = false; }",
    "    if (usable) return;",
    "    try { Object.defineProperty(window, name, { value: memoryStorage(), configurable: true, writable: true }); } catch (_) {}",
    "  });",
    "  var helper = window.TavernHelper || window.tavernHelper || {};",
    "  window.TavernHelper = helper;",
    "  window.tavernHelper = helper;",
    '  helper.version = helper.version || "4.3.17";',
    "  helper.getCharWorldbookNames = helper.getCharWorldbookNames || function(selector){",
    '    if (selector !== "current" || !WORLD_BOOK_NAME) return null;',
    "    return { primary: WORLD_BOOK_NAME, all: [WORLD_BOOK_NAME] };",
    "  };",
    "  helper.getWorldbook = helper.getWorldbook || function(name){",
    "    if (!WORLD_BOOK_NAME || name !== WORLD_BOOK_NAME) return Promise.resolve([]);",
    '    return bridge("campaign_worldbook_get", { name: name });',
    "  };",
    "  helper.updateWorldbookWith = helper.updateWorldbookWith || async function(name, updater){",
    "    if (!WORLD_BOOK_NAME || name !== WORLD_BOOK_NAME) {",
    '      throw new Error("Worldbook is not bound to the active StoryForge campaign");',
    "    }",
    "    var entries = await helper.getWorldbook(name);",
    '    var next = typeof updater === "function" ? await updater(entries) : entries;',
    '    return bridge("campaign_worldbook_update", { name: name, entries: next || entries });',
    "  };",
    "  window.getTavernHelperVersion = window.getTavernHelperVersion || function(){",
    '    return helper && typeof helper.version === "string" ? helper.version : null;',
    "  };",
    '  function isRecord(value){ return !!value && typeof value === "object" && !Array.isArray(value); }',
    "  function clone(value){",
    "    if (!isRecord(value)) return {};",
    "    try { return JSON.parse(JSON.stringify(value)); } catch (_) { return {}; }",
    "  }",
    "  function selectorKey(selector){",
    '    var type = String((selector && selector.type) || "local").toLowerCase();',
    '    if (["message", "character", "local", "chat", "global", "script", "preset"].indexOf(type) < 0) type = "local";',
    '    var candidates = type === "message" ? [selector && selector.message_id, selector && selector.messageId] :',
    '      type === "preset" ? [selector && selector.preset_id, selector && selector.presetId, selector && selector.name] :',
    '      type === "character" ? [selector && selector.character_id, selector && selector.characterId] :',
    '      type === "script" ? [selector && selector.script_id, selector && selector.scriptId] : [];',
    '    var identity = "current";',
    "    for (var i = 0; i < candidates.length; i++) { if (candidates[i] !== null && candidates[i] !== undefined && String(candidates[i]).trim()) { identity = encodeURIComponent(String(candidates[i]).trim()).slice(0, 256); break; } }",
    '    return type + ":" + identity;',
    "  }",
    '  function isSelector(value){ return isRecord(value) && typeof value.type === "string"; }',
    "  function getVariables(selector){ return clone(SELECTOR_VARIABLES[selectorKey(selector)]); }",
    "  function diffVariables(prev, next){",
    "    var sets = {}; var deletes = []; var k; var same;",
    "    for (k in next) { if (Object.prototype.hasOwnProperty.call(next, k)) {",
    "      try { same = JSON.stringify(prev[k]) === JSON.stringify(next[k]); } catch (_) { same = false; }",
    "      if (!same) sets[k] = next[k];",
    "    } }",
    "    for (k in prev) { if (Object.prototype.hasOwnProperty.call(prev, k) && !Object.prototype.hasOwnProperty.call(next, k)) deletes.push(k); }",
    "    return { sets: sets, deletes: deletes };",
    "  }",
    "  function persistVariables(selector, variables, mode){",
    "    var next = clone(variables);",
    '    var writeMode = mode === "merge" ? "merge" : "replace";',
    "    var key = selectorKey(selector);",
    "    var payloadMode = writeMode; var payload = next;",
    '    if (writeMode === "replace") {',
    // M2：整桶 replace 携带的是本壳过期快照，会抹掉另一壳对同桶的写入。
    // 改发相对本壳快照的键级补丁（sets/deletes），宿主按键应用，
    // 本壳未触碰的键在宿主侧存活。
    '      payloadMode = "patch"; payload = diffVariables(getVariables(selector), next);',
    "      SELECTOR_VARIABLES[key] = next;",
    "    } else {",
    "      SELECTOR_VARIABLES[key] = Object.assign(getVariables(selector), next);",
    "    }",
    '    var task = bridge("shell_variables_set", { selector: selector || { type: String(key).split(":")[0] }, variables: payload, mode: payloadMode });',
    '    task.catch(function(error){ try { console.error("[CardShell] variable persistence failed", error); } catch (_) {} });',
    "    return task;",
    "  }",
    "  function setVariables(first, second){",
    "    var selector = isSelector(first) ? first : second;",
    "    var variables = isSelector(first) ? second : first;",
    '    return persistVariables(selector, variables, "replace");',
    "  }",
    "  function getVariable(first, second){",
    "    var selector = isSelector(first) ? first : null;",
    "    var key = isSelector(first) ? second : first;",
    "    return getVariables(selector)[key];",
    "  }",
    "  function setVariable(first, second, third){",
    "    var selector = isSelector(first) ? first : third;",
    "    var key = isSelector(first) ? second : first;",
    "    var value = isSelector(first) ? third : second;",
    '    var patch = {}; patch[key] = value; return persistVariables(selector, patch, "merge");',
    "  }",
    "  function insertOrAssignVariables(first, second){",
    "    var selector = isSelector(first) ? first : second;",
    "    var patch = isSelector(first) ? second : first;",
    '    return persistVariables(selector, isRecord(patch) ? patch : {}, "merge");',
    "  }",
    "  function replaceVariables(first, second){ return setVariables(first, second); }",
    "  function updateVariablesWith(first, second){",
    "    var selector = isSelector(first) ? first : second;",
    "    var updater = isSelector(first) ? second : first;",
    "    var current = getVariables(selector);",
    '    var next = typeof updater === "function" ? updater(current) : updater;',
    '    return persistVariables(selector, next, "replace");',
    "  }",
    "  function deleteVariable(first, second){",
    "    var selector = isSelector(first) ? first : second;",
    "    var key = isSelector(first) ? second : first;",
    '    var next = getVariables(selector); delete next[key]; return persistVariables(selector, next, "replace");',
    "  }",
    "  window.getVariables = getVariables;",
    "  window.setVariables = setVariables;",
    "  window.getVariable = getVariable;",
    "  window.setVariable = setVariable;",
    "  window.insertOrAssignVariables = insertOrAssignVariables;",
    "  window.replaceVariables = replaceVariables;",
    "  window.updateVariablesWith = updateVariablesWith;",
    "  window.deleteVariable = deleteVariable;",
    "  helper.getVariables = getVariables;",
    "  helper.setVariables = setVariables;",
    "  helper.getVariable = getVariable;",
    "  helper.setVariable = setVariable;",
    "  helper.insertOrAssignVariables = insertOrAssignVariables;",
    "  helper.replaceVariables = replaceVariables;",
    "  helper.updateVariablesWith = updateVariablesWith;",
    "  helper.deleteVariable = deleteVariable;",
    "  function cloneValue(v){ try { return v === undefined ? null : JSON.parse(JSON.stringify(v)); } catch (_) { return null; } }",
    "  function deepMergeRecords(base, extra){",
    "    var out = isRecord(base) ? clone(base) : {};",
    "    if (!isRecord(extra)) return out;",
    "    for (var k in extra) { if (Object.prototype.hasOwnProperty.call(extra, k)) {",
    "      out[k] = isRecord(out[k]) && isRecord(extra[k]) ? deepMergeRecords(out[k], extra[k]) : cloneValue(extra[k]);",
    "    } }",
    "    return out;",
    "  }",
    "  var mvu = window.Mvu || {};",
    '  mvu.runtime = mvu.runtime || "StoryForge WebViewMvuRuntime";',
    // M5：events 常量表——缺失时 eventOn(Mvu.events.…) 直接 TypeError 中止整段
    // 内联脚本。宿主不发这些事件（监听是空转），但脚本余下部分得以执行。
    "  mvu.events = mvu.events || {",
    '    SINGLE_VARIABLE_UPDATED: "mag_variable_updated",',
    '    VARIABLE_UPDATE_STARTED: "mag_variable_update_started",',
    '    VARIABLE_UPDATE_ENDED: "mag_variable_update_ended"',
    "  };",
    "  function refreshMvuStatData(){",
    '    return bridge("mvu_data_get", {}).then(function(data){',
    "      if (isRecord(data)) MVU_STAT_DATA = data;",
    "      return MVU_STAT_DATA;",
    "    }).catch(function(){ return MVU_STAT_DATA; });",
    "  }",
    '  mvu.isReady = function(){ return bridge("mvu_status", {}); };',
    // M5：读侧桥接真实 Campaign 变量树（只读底座），壳自写桶键级覆盖其上；
    // 写侧保持沙箱桶（不直写一等变量，与 M4 一致）。
    "  mvu.getMvuData = function(selector){",
    "    var local = getVariables(selector);",
    "    local.stat_data = deepMergeRecords(MVU_STAT_DATA, isRecord(local.stat_data) ? local.stat_data : {});",
    "    return local;",
    "  };",
    "  mvu.refreshMvuData = refreshMvuStatData;",
    '  mvu.replaceMvuData = function(value, selector){ return persistVariables(selector, value, "replace"); };',
    "  window.Mvu = mvu;",
    "  refreshMvuStatData();",
    "  window.waitGlobalInitialized = window.waitGlobalInitialized || function(name){",
    '    var globalName = String(name || "");',
    '    if (globalName !== "Mvu") {',
    "      return globalName && window[globalName]",
    "        ? Promise.resolve(window[globalName])",
    '        : Promise.reject(new Error(globalName + " unavailable in the StoryForge card shell"));',
    "    }",
    "    var deadline = Date.now() + 2500;",
    "    function check(){",
    '      return bridge("mvu_status", {}).then(function(status){',
    "        if (status && status.ready) return refreshMvuStatData().then(function(){ return window.Mvu; });",
    '        if (Date.now() >= deadline) throw new Error("Mvu unavailable in the StoryForge card shell");',
    "        return new Promise(function(resolve){ setTimeout(resolve, 50); }).then(check);",
    "      });",
    "    }",
    "    return check();",
    "  };",
    "})();"
  ].join("\n");
}
function isCardShellBridgeMessageForSession(event, expectedSession) {
  const data = event?.data;
  return Boolean(
    expectedSession && data?.__sf_shell_bridge === true && data.shellSession === expectedSession
  );
}

// frontend/src/utils/cardShellFetchProxy.js
function createCardShellFetchProxyScript() {
  return [
    "(function(){",
    "  var nativeFetch = window.fetch ? window.fetch.bind(window) : null;",
    "  var shellCacheEntries = new Map();",
    "  var shellCache = {",
    "    open: function(){ return Promise.resolve({",
    "      match: function(key){ var value = shellCacheEntries.get(String(key)); return Promise.resolve(value ? value.clone() : undefined); },",
    "      put: function(key, response){ shellCacheEntries.set(String(key), response.clone()); return Promise.resolve(); },",
    "    }); },",
    "  };",
    '  try { Object.defineProperty(window, "caches", { value: shellCache, configurable: true }); } catch (_) { window.caches = shellCache; }',
    "  function dataUrlResponse(dataUrl){",
    '    var raw = String(dataUrl || "");',
    '    var comma = raw.indexOf(",");',
    '    if (comma < 0 || raw.slice(0, 5).toLowerCase() !== "data:") throw new Error("host returned an invalid data URL");',
    "    var meta = raw.slice(5, comma);",
    "    var body = raw.slice(comma + 1);",
    '    var contentType = (meta.split(";")[0] || "application/octet-stream").trim();',
    "    var bytes;",
    "    if (/(?:^|;)base64(?:;|$)/i.test(meta)) {",
    "      var binary = atob(body);",
    "      bytes = new Uint8Array(binary.length);",
    "      for (var i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);",
    "    } else {",
    "      var decoded = decodeURIComponent(body);",
    "      bytes = new TextEncoder().encode(decoded);",
    "    }",
    '    return new Response(bytes, { status: 200, headers: { "Content-Type": contentType } });',
    "  }",
    "  function announceHostFallback(message){",
    '    if (!message || !document || !document.body || document.querySelector("[data-sf-shell-map-fallback]")) return;',
    '    var notice = document.createElement("div");',
    '    notice.setAttribute("data-sf-shell-map-fallback", "true");',
    "    notice.textContent = String(message);",
    '    notice.style.cssText = "position:fixed;right:12px;bottom:12px;z-index:2147483647;max-width:min(320px,calc(100vw - 24px));padding:8px 10px;border:1px solid rgba(255,204,102,.7);border-radius:8px;background:rgba(20,26,40,.94);color:#ffe2a6;font:12px/1.45 sans-serif;box-shadow:0 8px 22px rgba(0,0,0,.35);pointer-events:none;";',
    "    document.body.appendChild(notice);",
    "  }",
    "  function hostResourceResponse(resource, init){",
    '    if (resource && typeof resource === "object" && typeof resource.cacheUrl === "string") {',
    '      if (!nativeFetch) throw new Error("native fetch unavailable for host cache resource");',
    "      announceHostFallback(resource.fallbackMessage);",
    "      return nativeFetch(resource.cacheUrl, init);",
    "    }",
    "    return dataUrlResponse(resource);",
    "  }",
    "  window.fetch = function(input, init){",
    "    try {",
    '      var url = (typeof input === "string") ? input : (input && input.url);',
    "      if (url && (/^https?:\\/\\//i).test(String(url))) {",
    "        return window.__sfHostFetchDataUrl(String(url)).then(function(resource){ return hostResourceResponse(resource, init); });",
    "      }",
    "    } catch (e) {}",
    "    if (nativeFetch) return nativeFetch(input, init);",
    '    return Promise.reject(new Error("fetch unavailable"));',
    "  };",
    "})();"
  ].join("\n");
}
function createCardShellMapReadyFallbackScript() {
  return [
    "(function(){",
    "  function hasReadyMapImage(){",
    `    var mapPage = document.querySelector('[data-page="map"]');`,
    "    if (!mapPage) return false;",
    '    var images = mapPage.querySelectorAll("img");',
    "    for (var i = 0; i < images.length; i++) {",
    "      var image = images[i];",
    '      var src = String(image.currentSrc || image.src || "");',
    '      if (src.indexOf("blob:") === 0 && image.complete && image.naturalWidth > 0 && image.naturalHeight > 0) return true;',
    "    }",
    "    return false;",
    "  }",
    "  function settleMapLoader(){",
    "    if (!hasReadyMapImage()) return;",
    '    var nodes = document.querySelectorAll("span, div, p");',
    "    for (var i = 0; i < nodes.length; i++) {",
    "      var node = nodes[i];",
    "      if (node.children.length !== 0) continue;",
    '      if (String(node.textContent || "").trim() !== "地图加载中，请稍候…") continue;',
    '      node.style.display = "none";',
    '      node.setAttribute("aria-hidden", "true");',
    "    }",
    "  }",
    '  if (typeof MutationObserver === "function") {',
    "    new MutationObserver(settleMapLoader).observe(document.documentElement, { childList: true, subtree: true, attributes: true });",
    "  }",
    "  window.setInterval(settleMapLoader, 250);",
    "  window.setTimeout(settleMapLoader, 0);",
    "})();"
  ].join("\n");
}

// frontend/src/utils/cardShellDisplay.js
var LOAD_RE = /\$\(\s*['"]body['"]\s*\)\s*\.load\(\s*['"`](https?:\/\/[^'"`]+)['"`]\s*\)/gi;
function classifyShellUrl(url) {
  const u = String(url || "").toLowerCase();
  if (u.includes("/status/")) return "status";
  if (u.includes("/home/")) return "opening_home";
  if (u.includes("custom_start") || u.includes("/intro/")) return "opening_custom";
  return "message_html";
}
function segmentShellContent(displayContent) {
  const text = String(displayContent || "");
  if (!text) return [{ type: "text", content: text }];
  const FENCE_GLUE_RE = /```[\s\S]*?\$\(\s*['"]body['"]\s*\)\s*\.load\([\s\S]*?```/gi;
  const BODY_GLUE_RE = /<body[^>]*>\s*<script[^>]*>\s*\$\(\s*['"]body['"]\s*\)\s*\.load\([\s\S]*?<\/script>\s*<\/body>/gi;
  const BARE_LOAD_RE = /\$\(\s*['"]body['"]\s*\)\s*\.load\(\s*['"`]https?:\/\/[^'"`]+['"`]\s*\)/gi;
  const INLINE_DOC_RE = /<!DOCTYPE\s+html[\s\S]*?<\/html\s*>|<html[\s>][\s\S]*?<\/html\s*>|<body[\s>][\s\S]*?<\/body\s*>/gi;
  const regions = [];
  const overlapsClaimed = (start, end) => regions.some((r) => start < r.end && end > r.start);
  const passes = [
    [FENCE_GLUE_RE, "load"],
    [BODY_GLUE_RE, "load"],
    [BARE_LOAD_RE, "load"],
    [INLINE_DOC_RE, "inline"]
  ];
  for (const [re, mode] of passes) {
    let m;
    while ((m = re.exec(text)) !== null) {
      const start = m.index;
      const end = start + m[0].length;
      if (overlapsClaimed(start, end)) continue;
      if (mode === "inline") {
        if (!/<script[\s>]/i.test(m[0])) continue;
        regions.push({ start, end, mode, html: m[0] });
      } else {
        const urls = extractLoadUrlsFrom(m[0]);
        if (!urls.length) continue;
        regions.push({ start, end, mode, urls });
      }
    }
  }
  if (!regions.length) return [{ type: "text", content: text }];
  regions.sort((a, b) => a.start - b.start);
  const segments = [];
  const seenUrls = /* @__PURE__ */ new Set();
  const pushText = (raw) => {
    const cleaned = raw.replace(/\n{3,}/g, "\n\n").trim();
    if (cleaned) segments.push({ type: "text", content: cleaned });
  };
  let cursor = 0;
  for (const region of regions) {
    if (region.start > cursor) pushText(text.slice(cursor, region.start));
    if (region.mode === "inline") {
      segments.push({
        type: "shell",
        mode: "inline",
        html: region.html,
        kind: "message_html",
        start: region.start
      });
    } else {
      for (const url of region.urls) {
        if (seenUrls.has(url)) continue;
        seenUrls.add(url);
        segments.push({
          type: "shell",
          mode: "load",
          url,
          kind: classifyShellUrl(url),
          start: region.start
        });
      }
    }
    cursor = region.end;
  }
  if (cursor < text.length) pushText(text.slice(cursor));
  return segments;
}
function extractLoadUrlsFrom(regionText) {
  const urls = [];
  const seen = /* @__PURE__ */ new Set();
  const re = new RegExp(LOAD_RE.source, LOAD_RE.flags);
  let m;
  while ((m = re.exec(regionText)) !== null) {
    if (seen.has(m[1])) continue;
    seen.add(m[1]);
    urls.push(m[1]);
  }
  return urls;
}
var MAX_TRIGGER_PATTERN_LEN = 4096;
var MAX_TRIGGER_TEXT_LEN = 2e5;
function parseStFindRegex(trigger) {
  const t = String(trigger || "").trim();
  if (!t) return null;
  if (t.length > MAX_TRIGGER_PATTERN_LEN) return null;
  const wrapped = t.match(/^\/([\s\S]+)\/([a-z]*)$/i);
  try {
    if (wrapped) {
      return new RegExp(wrapped[1], wrapped[2].replace(/[^gimsuy]/g, ""));
    }
    return new RegExp(t);
  } catch {
    return null;
  }
}
function trustedInlineShellDocStarts(displayContent, sourceContent, triggers) {
  const trusted = /* @__PURE__ */ new Set();
  const displayText = String(displayContent || "");
  const sourceText = String(sourceContent || "");
  if (!displayText || !sourceText) return trusted;
  const triggerList = Array.isArray(triggers) ? triggers : [];
  if (!triggerList.length) return trusted;
  const cappedSource = sourceText.length > MAX_TRIGGER_TEXT_LEN ? sourceText.slice(0, MAX_TRIGGER_TEXT_LEN) : sourceText;
  const matchedStarts = [];
  for (const item of triggerList) {
    const trigger = typeof item === "string" ? item : item?.trigger;
    const re = parseStFindRegex(trigger);
    if (!re) continue;
    const globalRe = re.global ? re : new RegExp(re.source, `${re.flags}g`);
    for (const match of cappedSource.matchAll(globalRe)) {
      matchedStarts.push(match.index);
    }
  }
  if (!matchedStarts.length) return trusted;
  matchedStarts.sort((a, b) => a - b);
  let index = 0;
  for (const segment of segmentShellContent(displayText)) {
    if (segment.type !== "shell" || segment.mode !== "inline") continue;
    if (index >= matchedStarts.length) break;
    trusted.add(segment.start);
    index += 1;
  }
  return trusted;
}

// frontend/src/utils/cardShellVariableStore.js
var CARD_SHELL_VARIABLES_KEY = "__storyforge_card_shell_variables";
var SUPPORTED_BUCKETS = /* @__PURE__ */ new Set(["message", "character", "local", "chat", "global", "script", "preset"]);
function isRecord(value) {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}
function cloneRecord(value) {
  if (!isRecord(value)) return {};
  try {
    return JSON.parse(JSON.stringify(value));
  } catch {
    return {};
  }
}
function selectorKey(selector) {
  const type = String(selector?.type || "local").toLowerCase();
  const safeType = SUPPORTED_BUCKETS.has(type) ? type : "local";
  const identity = safeSelectorIdentity(selector, safeType);
  return `${safeType}:${identity}`;
}
function safeSelectorIdentity(selector, type) {
  const candidates = type === "message" ? [selector?.message_id, selector?.messageId] : type === "preset" ? [selector?.preset_id, selector?.presetId, selector?.name] : type === "character" ? [selector?.character_id, selector?.characterId] : type === "script" ? [selector?.script_id, selector?.scriptId] : [];
  const raw = candidates.find((value) => value !== null && value !== void 0 && String(value).trim());
  return raw === void 0 ? "current" : encodeURIComponent(String(raw).trim()).slice(0, 256);
}
function isStoredSelectorKey(key) {
  const [type, identity] = String(key || "").split(":", 2);
  return SUPPORTED_BUCKETS.has(type) && Boolean(identity) && identity.length <= 256;
}
function normalizeCardShellVariables(value) {
  if (!isRecord(value)) return {};
  return Object.fromEntries(
    Object.entries(value).flatMap(([key, variables]) => {
      if (!isRecord(variables)) return [];
      const normalizedKey = SUPPORTED_BUCKETS.has(key) ? `${key}:current` : key;
      return isStoredSelectorKey(normalizedKey) ? [[normalizedKey, cloneRecord(variables)]] : [];
    })
  );
}
function updateCardShellVariables(snapshot, selector, variables) {
  const next = normalizeCardShellVariables(snapshot);
  next[selectorKey(selector)] = cloneRecord(variables);
  return next;
}
function mergeCardShellVariables(snapshot, selector, variables) {
  const next = normalizeCardShellVariables(snapshot);
  const key = selectorKey(selector);
  next[key] = { ...cloneRecord(next[key]), ...cloneRecord(variables) };
  return next;
}
function patchCardShellVariables(snapshot, selector, patch) {
  const next = normalizeCardShellVariables(snapshot);
  const key = selectorKey(selector);
  const bucket = cloneRecord(next[key]);
  Object.assign(bucket, isRecord(patch?.sets) ? cloneRecord(patch.sets) : {});
  for (const name of Array.isArray(patch?.deletes) ? patch.deletes : []) {
    if (typeof name === "string") delete bucket[name];
  }
  next[key] = bucket;
  return next;
}

// frontend/src/utils/mvuKey.js
function normalizeMvuKey(raw) {
  let s = String(raw ?? "").trim();
  if (s.startsWith("/") || s.startsWith("stat_data/")) {
    s = s.replace(/^\/+/, "").replaceAll("/", ".");
  }
  while (s.startsWith("stat_data.")) s = s.slice("stat_data.".length);
  return s.split(".").map((seg) => seg.trim()).filter(Boolean).map(
    (seg) => seg.length >= 2 && seg.startsWith("<") && seg.endsWith(">") ? `{${seg.slice(1, -1)}}` : seg
  ).join(".");
}

// frontend/src/utils/mvuStatTree.js
function isRecord2(value) {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}
function assignLeaf(node, leaf, value) {
  const next = value === void 0 ? null : value;
  const existing = node[leaf];
  if (!isRecord2(existing)) {
    node[leaf] = next;
    return;
  }
  if (isRecord2(next)) {
    for (const [key, child] of Object.entries(next)) {
      assignLeaf(existing, key, child);
    }
  }
}
function buildMvuStatDataTree(campaignVariables) {
  const tree = {};
  for (const item of Array.isArray(campaignVariables) ? campaignVariables : []) {
    const rawKey = typeof item?.key === "string" ? item.key : "";
    if (!rawKey || rawKey.startsWith("__storyforge")) continue;
    const segments = normalizeMvuKey(rawKey).split(".").filter(Boolean);
    if (!segments.length) continue;
    let node = tree;
    for (const segment of segments.slice(0, -1)) {
      if (!isRecord2(node[segment])) {
        node[segment] = {};
      }
      node = node[segment];
    }
    assignLeaf(node, segments[segments.length - 1], item?.value);
  }
  return tree;
}

// frontend/src/utils/tavernHelperScripts.js
function analyzeEsmModuleSource(source) {
  if (typeof source !== "string") {
    throw new Error("module source must be a string");
  }
  const imports = [];
  let isModule = false;
  let statementStart = true;
  let braceDepth = 0;
  let index = 0;
  const isIdentifierStart = (ch) => /[A-Za-z_$]/.test(ch || "");
  const isIdentifierPart = (ch) => /[A-Za-z0-9_$]/.test(ch || "");
  const skipQuoted = (start, quote) => {
    let cursor = start + 1;
    while (cursor < source.length) {
      if (source[cursor] === "\\") {
        cursor += 2;
        continue;
      }
      if (source[cursor] === quote) return cursor + 1;
      cursor += 1;
    }
    return source.length;
  };
  const skipLineComment = (start) => {
    const end = source.indexOf("\n", start + 2);
    return end < 0 ? source.length : end;
  };
  const skipBlockComment = (start) => {
    const end = source.indexOf("*/", start + 2);
    return end < 0 ? source.length : end + 2;
  };
  const skipSpaceAndComments = (start) => {
    let cursor = start;
    while (cursor < source.length) {
      if (/\s/.test(source[cursor])) {
        cursor += 1;
        continue;
      }
      if (source.startsWith("//", cursor)) {
        cursor = skipLineComment(cursor);
        continue;
      }
      if (source.startsWith("/*", cursor)) {
        cursor = skipBlockComment(cursor);
        continue;
      }
      break;
    }
    return cursor;
  };
  const statementEnd = (start) => {
    let cursor = start;
    let localDepth = 0;
    while (cursor < source.length) {
      const ch = source[cursor];
      if (ch === "'" || ch === '"' || ch === "`") {
        cursor = skipQuoted(cursor, ch);
        continue;
      }
      if (source.startsWith("//", cursor)) {
        cursor = skipLineComment(cursor);
        continue;
      }
      if (source.startsWith("/*", cursor)) {
        cursor = skipBlockComment(cursor);
        continue;
      }
      if (ch === "(" || ch === "[" || ch === "{") localDepth += 1;
      if (ch === ")" || ch === "]" || ch === "}") {
        localDepth = Math.max(0, localDepth - 1);
      }
      if (ch === ";" && localDepth === 0) return cursor;
      cursor += 1;
    }
    return source.length;
  };
  const pushSpecifier = (start, end, dynamic = false) => {
    imports.push({
      specifier: source.slice(start, end),
      start,
      end,
      dynamic
    });
  };
  while (index < source.length) {
    const ch = source[index];
    if (/\s/.test(ch)) {
      if ((ch === "\n" || ch === "\r") && braceDepth === 0) {
        statementStart = true;
      }
      index += 1;
      continue;
    }
    if (source.startsWith("//", index)) {
      index = skipLineComment(index);
      continue;
    }
    if (source.startsWith("/*", index)) {
      index = skipBlockComment(index);
      continue;
    }
    if (ch === "'" || ch === '"' || ch === "`") {
      index = skipQuoted(index, ch);
      statementStart = false;
      continue;
    }
    if (ch === "{") {
      braceDepth += 1;
      statementStart = false;
      index += 1;
      continue;
    }
    if (ch === "}") {
      braceDepth = Math.max(0, braceDepth - 1);
      statementStart = braceDepth === 0;
      index += 1;
      continue;
    }
    if (ch === ";") {
      statementStart = braceDepth === 0;
      index += 1;
      continue;
    }
    if (!isIdentifierStart(ch)) {
      statementStart = false;
      index += 1;
      continue;
    }
    const tokenStart = index;
    index += 1;
    while (isIdentifierPart(source[index])) index += 1;
    const token = source.slice(tokenStart, index);
    const next = skipSpaceAndComments(index);
    if (token === "import" && source[next] === "(") {
      isModule = true;
      const argument = skipSpaceAndComments(next + 1);
      if (source[argument] === "'" || source[argument] === '"') {
        const quoteEnd = skipQuoted(argument, source[argument]) - 1;
        pushSpecifier(argument + 1, quoteEnd, true);
      }
      statementStart = false;
      continue;
    }
    if (braceDepth !== 0 || !statementStart || token !== "import" && token !== "export") {
      statementStart = false;
      continue;
    }
    isModule = true;
    const end = statementEnd(index);
    if (token === "import" && (source[next] === "'" || source[next] === '"')) {
      const quoteEnd = skipQuoted(next, source[next]) - 1;
      pushSpecifier(next + 1, quoteEnd);
    } else {
      const statement = source.slice(index, end);
      const from = /\bfrom\s*(['"])([^'"\r\n]+)\1/.exec(statement);
      if (from) {
        const relativeStart = from.index + from[0].indexOf(from[2]);
        pushSpecifier(index + relativeStart, index + relativeStart + from[2].length);
      }
    }
    index = end < source.length ? end + 1 : end;
    statementStart = true;
  }
  return {
    isModule,
    imports
  };
}

// frontend/src/utils/cardShellOpeningChat.js
function buildOpeningChatSwipes(greetingOptions = []) {
  if (!Array.isArray(greetingOptions)) return [];
  const swipes = [];
  const seen = /* @__PURE__ */ new Set();
  for (const option of greetingOptions) {
    const content = typeof option === "string" ? option : option?.content;
    const text = String(content || "").trim();
    if (!text || seen.has(text)) continue;
    seen.add(text);
    swipes.push(String(content));
  }
  return swipes;
}
function buildOpeningChatSeed(greetingOptions = [], options = {}) {
  const swipes = buildOpeningChatSwipes(greetingOptions);
  const name = String(options.name || "Assistant");
  const selected = Number(options.selectedIndex);
  const fallbackMes = swipes[Number.isFinite(selected) && selected >= 0 ? selected : 0] || "";
  return {
    name,
    is_user: false,
    is_name: true,
    mes: fallbackMes,
    message: fallbackMes,
    // Force the first「开始旅程」click to take the swipe-switch branch.
    swipe_id: -1,
    swipes: swipes.length ? swipes : [fallbackMes || ""]
  };
}
function resolveOpeningChatSelection(payload = {}, greetingOptions = []) {
  const swipes = buildOpeningChatSwipes(greetingOptions);
  const rawSwipe = payload?.swipe_id;
  const swipeId = rawSwipe === null || rawSwipe === void 0 || rawSwipe === "" ? null : Number(rawSwipe);
  const mes = typeof payload?.mes === "string" ? payload.mes : "";
  if (Number.isFinite(swipeId) && swipeId >= 0 && swipeId < swipes.length) {
    return {
      greetingIndex: swipeId,
      content: swipes[swipeId]
    };
  }
  if (mes) {
    const byContent = swipes.findIndex((swipe) => swipe === mes);
    if (byContent >= 0) {
      return { greetingIndex: byContent, content: swipes[byContent] };
    }
    return { greetingIndex: null, content: mes };
  }
  return { greetingIndex: null, content: swipes[0] || "" };
}

// frontend/src/utils/campaignGreetingOptions.js
function buildGreetingOptionsFromDetail(detail) {
  if (!detail) return [];
  const options = [];
  const seen = /* @__PURE__ */ new Set();
  const addOption = (label, content) => {
    if (!content || !content.trim() || seen.has(content)) return;
    seen.add(content);
    options.push({ label, content });
  };
  addOption("默认", detail.first_mes);
  const alternates = Array.isArray(detail.alternate_greetings) ? detail.alternate_greetings : [];
  alternates.forEach((content, index) => {
    addOption(`备选 ${index + 1}`, content);
  });
  return options;
}

// frontend/src/utils/cardShellWorldbook.js
function entryName(entry) {
  const name = String(entry?.name || "").trim();
  if (name) return name;
  const firstKey = Array.isArray(entry?.keys) ? entry.keys.find((key) => String(key || "").trim()) : null;
  if (firstKey) return String(firstKey).trim();
  return `world-info-${Number(entry?.index) || 0}`;
}
function mapCampaignWorldbookForTavernHelper(worldbook) {
  return Array.isArray(worldbook?.entries) ? worldbook.entries.map((entry) => ({
    index: Number(entry?.index) || 0,
    name: entryName(entry),
    enabled: !entry?.disabled
  })) : [];
}
function resolveCampaignWorldbookEnabledUpdates(currentEntries, requestedEntries) {
  const requestedByName = new Map(
    (Array.isArray(requestedEntries) ? requestedEntries : []).filter((entry) => entry?.name != null).map((entry) => [String(entry.name), Boolean(entry.enabled)])
  );
  return (Array.isArray(currentEntries) ? currentEntries : []).flatMap((entry) => {
    const requestedEnabled = requestedByName.get(entryName(entry));
    if (requestedEnabled === void 0 || requestedEnabled === !entry?.disabled) return [];
    return [{ entryIndex: Number(entry?.index) || 0, enabled: requestedEnabled }];
  });
}

// frontend/src/utils/cardShellPresentation.js
function getOpeningShellPresentation(viewportHeight = defaultViewportHeight()) {
  const raw = Number(viewportHeight);
  const vh = Number.isFinite(raw) && raw > 0 ? raw : 900;
  const heightPx = Math.max(520, Math.min(900, Math.round(vh - 160)));
  return {
    height: `${heightPx}px`,
    autoHeight: false
  };
}
function defaultViewportHeight() {
  try {
    if (typeof window !== "undefined" && window.innerHeight) return window.innerHeight;
  } catch (_) {
  }
  return 900;
}
export {
  CARD_SHELL_VARIABLES_KEY,
  MSG_REQUEST,
  MSG_RESPONSE,
  analyzeEsmModuleSource,
  buildGreetingOptionsFromDetail,
  buildMvuStatDataTree,
  buildOpeningChatSeed,
  classifyShellUrl,
  createCardShellFetchProxyScript,
  createCardShellMapReadyFallbackScript,
  createCardShellRuntimeCompatibilityScript,
  createHostHandler,
  generateBridgeScript,
  getOpeningShellPresentation,
  isCardShellBridgeMessageForSession,
  mapCampaignWorldbookForTavernHelper,
  mergeCardShellVariables,
  normalizeCardShellVariables,
  patchCardShellVariables,
  resolveCampaignWorldbookEnabledUpdates,
  resolveOpeningChatSelection,
  rewriteCardShellTopBridgeAccess,
  segmentShellContent,
  trustedInlineShellDocStarts,
  updateCardShellVariables
};
