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
 * @returns {string}
 */
export function createCardShellRuntimeCompatibilityScript() {
  return [
    '(function(){',
    '  window.getTavernHelperVersion = window.getTavernHelperVersion || function(){',
    '    var helper = window.TavernHelper || window.tavernHelper;',
    '    return helper && typeof helper.version === "string" ? helper.version : null;',
    '  };',
    '  window.waitGlobalInitialized = window.waitGlobalInitialized || function(name){',
    '    var globalName = String(name || "");',
    '    if (globalName && window[globalName]) return Promise.resolve(window[globalName]);',
    '    var reason = globalName === "Mvu"',
    '      ? "Mvu unavailable in the StoryForge card shell"',
    '      : globalName + " unavailable in the StoryForge card shell";',
    '    return Promise.reject(new Error(reason));',
    '  };',
    '})();',
  ].join('\n')
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
