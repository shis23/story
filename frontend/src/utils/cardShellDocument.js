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
 * @param {{worldbookName?: string}} options
 * @returns {string}
 */
export function createCardShellRuntimeCompatibilityScript(options = {}) {
  const worldbookName = JSON.stringify(String(options.worldbookName || ''))
  return [
    '(function(){',
    `  var WORLD_BOOK_NAME = ${worldbookName};`,
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
    '  window.Mvu = window.Mvu || {',
    '    runtime: "StoryForge WebViewMvuRuntime",',
    '    isReady: function(){ return bridge("mvu_status", {}); },',
    '  };',
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
    '        if (status && status.ready) return window.Mvu;',
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
