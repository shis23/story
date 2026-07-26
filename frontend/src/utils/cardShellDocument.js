/**
 * Rebind the small, explicit ST host surface used by trusted card shells.
 *
 * CardShellHost deliberately runs fetched card UI in a sandboxed, opaque-origin
 * iframe. In that context `window.top` is not readable, while the compatible
 * ST APIs are installed on the iframe's own `window` by plugin-bridge. Only
 * rewrite these known API roots; unrelated top-window access must retain its
 * original browser semantics.
 *
 * @param {string} html
 * @returns {string}
 */
export function rewriteCardShellTopBridgeAccess(html) {
  return String(html || '').replace(
    /window\s*\.\s*top\s*(?:\?\.\s*|\.\s*)(TavernHelper|tavernHelper|SillyTavern)\b/g,
    (_match, apiRoot) => `window.${apiRoot}`,
  )
}

/**
 * Supply the two legacy globals that the Destiny home shell probes before it
 * renders an environment result. StoryForge does not claim unsupported ST
 * features: the helpers report the bridge's actual capability (or reject),
 * letting the card show a concrete unsupported state instead of retaining its
 * initial loading placeholder forever.
 *
 * @param {{worldbookName?: string, selectorVariables?: Record<string, object>, mvuStatData?: Record<string, unknown>}} options
 * @returns {string}
 */
export function createCardShellRuntimeCompatibilityScript(options = {}) {
  // This text is embedded inside a `<script>` tag in the isolated document.
  // Escape HTML-significant characters after JSON encoding so a persisted card
  // value can never close that tag and append arbitrary document markup.
  const scriptJson = (value) => JSON.stringify(value)
    .replace(/</g, '\\u003c')
    .replace(/>/g, '\\u003e')
    .replace(/&/g, '\\u0026')
    .replace(/\u2028/g, '\\u2028')
    .replace(/\u2029/g, '\\u2029')
  const worldbookName = scriptJson(String(options.worldbookName || ''))
  const selectorVariables = scriptJson(options.selectorVariables && typeof options.selectorVariables === 'object'
    ? options.selectorVariables
    : {})
  const mvuStatData = scriptJson(options.mvuStatData && typeof options.mvuStatData === 'object'
    ? options.mvuStatData
    : {})
  return [
    '(function(){',
    `  var WORLD_BOOK_NAME = ${worldbookName};`,
    `  var SELECTOR_VARIABLES = ${selectorVariables};`,
    // M5：真实 Campaign 变量树（stat_data 底座，只读）。构建时注入快照，
    // mvu_data_get 桥可刷新；壳自写的选择器桶按键覆盖其上。
    `  var MVU_STAT_DATA = ${mvuStatData};`,
    '  function bridge(type, payload){',
    '    if (typeof window.__sfShellAsk !== "function") {',
    '      return Promise.reject(new Error("StoryForge card shell bridge is unavailable"));',
    '    }',
    '    return window.__sfShellAsk(type, payload || {});',
    '  }',
    '  var helper = window.TavernHelper || window.tavernHelper || {};',
    '  window.TavernHelper = helper;',
    '  window.tavernHelper = helper;',
    '  helper.version = helper.version || "4.3.17";',
    '  helper.getCharWorldbookNames = helper.getCharWorldbookNames || function(selector){',
    '    if (selector !== "current" || !WORLD_BOOK_NAME) return null;',
    '    return { primary: WORLD_BOOK_NAME, all: [WORLD_BOOK_NAME] };',
    '  };',
    '  helper.getWorldbook = helper.getWorldbook || function(name){',
    '    if (!WORLD_BOOK_NAME || name !== WORLD_BOOK_NAME) return Promise.resolve([]);',
    '    return bridge("campaign_worldbook_get", { name: name });',
    '  };',
    '  helper.updateWorldbookWith = helper.updateWorldbookWith || async function(name, updater){',
    '    if (!WORLD_BOOK_NAME || name !== WORLD_BOOK_NAME) {',
    '      throw new Error("Worldbook is not bound to the active StoryForge campaign");',
    '    }',
    '    var entries = await helper.getWorldbook(name);',
    '    var next = typeof updater === "function" ? await updater(entries) : entries;',
    '    return bridge("campaign_worldbook_update", { name: name, entries: next || entries });',
    '  };',
    '  window.getTavernHelperVersion = window.getTavernHelperVersion || function(){',
    '    return helper && typeof helper.version === "string" ? helper.version : null;',
    '  };',
    '  function isRecord(value){ return !!value && typeof value === "object" && !Array.isArray(value); }',
    '  function clone(value){',
    '    if (!isRecord(value)) return {};',
    '    try { return JSON.parse(JSON.stringify(value)); } catch (_) { return {}; }',
    '  }',
    '  function selectorKey(selector){',
    '    var type = String((selector && selector.type) || "local").toLowerCase();',
    '    if (["message", "character", "local", "chat", "global", "script", "preset"].indexOf(type) < 0) type = "local";',
    '    var candidates = type === "message" ? [selector && selector.message_id, selector && selector.messageId] :',
    '      type === "preset" ? [selector && selector.preset_id, selector && selector.presetId, selector && selector.name] :',
    '      type === "character" ? [selector && selector.character_id, selector && selector.characterId] :',
    '      type === "script" ? [selector && selector.script_id, selector && selector.scriptId] : [];',
    '    var identity = "current";',
    '    for (var i = 0; i < candidates.length; i++) { if (candidates[i] !== null && candidates[i] !== undefined && String(candidates[i]).trim()) { identity = encodeURIComponent(String(candidates[i]).trim()).slice(0, 256); break; } }',
    '    return type + ":" + identity;',
    '  }',
    '  function isSelector(value){ return isRecord(value) && typeof value.type === "string"; }',
    '  function getVariables(selector){ return clone(SELECTOR_VARIABLES[selectorKey(selector)]); }',
    '  function diffVariables(prev, next){',
    '    var sets = {}; var deletes = []; var k; var same;',
    '    for (k in next) { if (Object.prototype.hasOwnProperty.call(next, k)) {',
    '      try { same = JSON.stringify(prev[k]) === JSON.stringify(next[k]); } catch (_) { same = false; }',
    '      if (!same) sets[k] = next[k];',
    '    } }',
    '    for (k in prev) { if (Object.prototype.hasOwnProperty.call(prev, k) && !Object.prototype.hasOwnProperty.call(next, k)) deletes.push(k); }',
    '    return { sets: sets, deletes: deletes };',
    '  }',
    '  function persistVariables(selector, variables, mode){',
    '    var next = clone(variables);',
    '    var writeMode = mode === "merge" ? "merge" : "replace";',
    '    var key = selectorKey(selector);',
    '    var payloadMode = writeMode; var payload = next;',
    '    if (writeMode === "replace") {',
    // M2：整桶 replace 携带的是本壳过期快照，会抹掉另一壳对同桶的写入。
    // 改发相对本壳快照的键级补丁（sets/deletes），宿主按键应用，
    // 本壳未触碰的键在宿主侧存活。
    '      payloadMode = "patch"; payload = diffVariables(getVariables(selector), next);',
    '      SELECTOR_VARIABLES[key] = next;',
    '    } else {',
    '      SELECTOR_VARIABLES[key] = Object.assign(getVariables(selector), next);',
    '    }',
    '    var task = bridge("shell_variables_set", { selector: selector || { type: String(key).split(":")[0] }, variables: payload, mode: payloadMode });',
    '    task.catch(function(error){ try { console.error("[CardShell] variable persistence failed", error); } catch (_) {} });',
    '    return task;',
    '  }',
    '  function setVariables(first, second){',
    '    var selector = isSelector(first) ? first : second;',
    '    var variables = isSelector(first) ? second : first;',
    '    return persistVariables(selector, variables, "replace");',
    '  }',
    '  function getVariable(first, second){',
    '    var selector = isSelector(first) ? first : null;',
    '    var key = isSelector(first) ? second : first;',
    '    return getVariables(selector)[key];',
    '  }',
    '  function setVariable(first, second, third){',
    '    var selector = isSelector(first) ? first : third;',
    '    var key = isSelector(first) ? second : first;',
    '    var value = isSelector(first) ? third : second;',
    '    var patch = {}; patch[key] = value; return persistVariables(selector, patch, "merge");',
    '  }',
    '  function insertOrAssignVariables(first, second){',
    '    var selector = isSelector(first) ? first : second;',
    '    var patch = isSelector(first) ? second : first;',
    '    return persistVariables(selector, isRecord(patch) ? patch : {}, "merge");',
    '  }',
    '  function replaceVariables(first, second){ return setVariables(first, second); }',
    '  function updateVariablesWith(first, second){',
    '    var selector = isSelector(first) ? first : second;',
    '    var updater = isSelector(first) ? second : first;',
    '    var current = getVariables(selector);',
    '    var next = typeof updater === "function" ? updater(current) : updater;',
    '    return persistVariables(selector, next, "replace");',
    '  }',
    '  function deleteVariable(first, second){',
    '    var selector = isSelector(first) ? first : second;',
    '    var key = isSelector(first) ? second : first;',
    '    var next = getVariables(selector); delete next[key]; return persistVariables(selector, next, "replace");',
    '  }',
    '  window.getVariables = getVariables;',
    '  window.setVariables = setVariables;',
    '  window.getVariable = getVariable;',
    '  window.setVariable = setVariable;',
    '  window.insertOrAssignVariables = insertOrAssignVariables;',
    '  window.replaceVariables = replaceVariables;',
    '  window.updateVariablesWith = updateVariablesWith;',
    '  window.deleteVariable = deleteVariable;',
    '  helper.getVariables = getVariables;',
    '  helper.setVariables = setVariables;',
    '  helper.getVariable = getVariable;',
    '  helper.setVariable = setVariable;',
    '  helper.insertOrAssignVariables = insertOrAssignVariables;',
    '  helper.replaceVariables = replaceVariables;',
    '  helper.updateVariablesWith = updateVariablesWith;',
    '  helper.deleteVariable = deleteVariable;',
    '  function cloneValue(v){ try { return v === undefined ? null : JSON.parse(JSON.stringify(v)); } catch (_) { return null; } }',
    '  function deepMergeRecords(base, extra){',
    '    var out = isRecord(base) ? clone(base) : {};',
    '    if (!isRecord(extra)) return out;',
    '    for (var k in extra) { if (Object.prototype.hasOwnProperty.call(extra, k)) {',
    '      out[k] = isRecord(out[k]) && isRecord(extra[k]) ? deepMergeRecords(out[k], extra[k]) : cloneValue(extra[k]);',
    '    } }',
    '    return out;',
    '  }',
    '  var mvu = window.Mvu || {};',
    '  mvu.runtime = mvu.runtime || "StoryForge WebViewMvuRuntime";',
    // M5：events 常量表——缺失时 eventOn(Mvu.events.…) 直接 TypeError 中止整段
    // 内联脚本。宿主不发这些事件（监听是空转），但脚本余下部分得以执行。
    '  mvu.events = mvu.events || {',
    '    SINGLE_VARIABLE_UPDATED: "mag_variable_updated",',
    '    VARIABLE_UPDATE_STARTED: "mag_variable_update_started",',
    '    VARIABLE_UPDATE_ENDED: "mag_variable_update_ended"',
    '  };',
    '  function refreshMvuStatData(){',
    '    return bridge("mvu_data_get", {}).then(function(data){',
    '      if (isRecord(data)) MVU_STAT_DATA = data;',
    '      return MVU_STAT_DATA;',
    '    }).catch(function(){ return MVU_STAT_DATA; });',
    '  }',
    '  mvu.isReady = function(){ return bridge("mvu_status", {}); };',
    // M5：读侧桥接真实 Campaign 变量树（只读底座），壳自写桶键级覆盖其上；
    // 写侧保持沙箱桶（不直写一等变量，与 M4 一致）。
    '  mvu.getMvuData = function(selector){',
    '    var local = getVariables(selector);',
    '    local.stat_data = deepMergeRecords(MVU_STAT_DATA, isRecord(local.stat_data) ? local.stat_data : {});',
    '    return local;',
    '  };',
    '  mvu.refreshMvuData = refreshMvuStatData;',
    '  mvu.replaceMvuData = function(value, selector){ return persistVariables(selector, value, "replace"); };',
    '  window.Mvu = mvu;',
    '  refreshMvuStatData();',
    '  window.waitGlobalInitialized = window.waitGlobalInitialized || function(name){',
    '    var globalName = String(name || "");',
    '    if (globalName !== "Mvu") {',
    '      return globalName && window[globalName]',
    '        ? Promise.resolve(window[globalName])',
    '        : Promise.reject(new Error(globalName + " unavailable in the StoryForge card shell"));',
    '    }',
    '    var deadline = Date.now() + 2500;',
    '    function check(){',
    '      return bridge("mvu_status", {}).then(function(status){',
    '        if (status && status.ready) return refreshMvuStatData().then(function(){ return window.Mvu; });',
    '        if (Date.now() >= deadline) throw new Error("Mvu unavailable in the StoryForge card shell");',
    '        return new Promise(function(resolve){ setTimeout(resolve, 50); }).then(check);',
    '      });',
    '    }',
    '    return check();',
    '  };',
    '})();',
  ].join('\n')
}

/**
 * A page can host several visible CardShellHost instances. Every iframe posts
 * to the same parent window, so a host must only service messages stamped with
 * its own fresh bridge session instead of trusting the shared event stream.
 *
 * @param {{data?: {__sf_shell_bridge?: boolean, shellSession?: string}} | null | undefined} event
 * @param {string} expectedSession
 * @returns {boolean}
 */
export function isCardShellBridgeMessageForSession(event, expectedSession) {
  const data = event?.data
  return Boolean(
    expectedSession &&
      data?.__sf_shell_bridge === true &&
      data.shellSession === expectedSession,
  )
}

/**
 * Keep a host-side inline module request unique when several visible shells
 * are mounted at once. Each iframe can otherwise ask every CardShellHost for
 * the same local `mod_1` source and receive the first unrelated reply.
 *
 * @param {string} shellId
 * @param {number} sequence
 * @returns {string}
 */
export function makeCardShellInlineModuleId(shellId, sequence) {
  return `${String(shellId || 'card-shell')}-mod-${Number(sequence) || 0}`
}

/**
 * A browser `message` is delivered to every CardShellHost listener in the
 * parent. Only the host retaining the requested source may answer it.
 *
 * @param {Map<string, string>} moduleSources
 * @param {string} moduleId
 * @returns {boolean}
 */
export function ownsCardShellInlineModule(moduleSources, moduleId) {
  return moduleSources instanceof Map && !!moduleId && moduleSources.has(moduleId)
}
