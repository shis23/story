/**
 * Build the iframe-local fetch proxy used by visible card shells.
 *
 * The shell iframe deliberately has an opaque sandbox origin. Fetching remote
 * binary assets directly from it is not reliable, especially for card maps.
 * The Tauri host fetches those assets, and this proxy reconstructs a real
 * Response so consumers such as OpenSeadragon can still call response.blob().
 */
export function createCardShellFetchProxyScript() {
  return [
    '(function(){',
    '  var nativeFetch = window.fetch ? window.fetch.bind(window) : null;',
    '  var shellCacheEntries = new Map();',
    '  var shellCache = {',
    '    open: function(){ return Promise.resolve({',
    '      match: function(key){ var value = shellCacheEntries.get(String(key)); return Promise.resolve(value ? value.clone() : undefined); },',
    '      put: function(key, response){ shellCacheEntries.set(String(key), response.clone()); return Promise.resolve(); },',
    '    }); },',
    '  };',
    '  try { Object.defineProperty(window, "caches", { value: shellCache, configurable: true }); } catch (_) { window.caches = shellCache; }',
    '  function dataUrlResponse(dataUrl){',
    '    var raw = String(dataUrl || "");',
    '    var comma = raw.indexOf(",");',
    '    if (comma < 0 || raw.slice(0, 5).toLowerCase() !== "data:") throw new Error("host returned an invalid data URL");',
    '    var meta = raw.slice(5, comma);',
    '    var body = raw.slice(comma + 1);',
    '    var contentType = (meta.split(";")[0] || "application/octet-stream").trim();',
    '    var bytes;',
    '    if (/(?:^|;)base64(?:;|$)/i.test(meta)) {',
    '      var binary = atob(body);',
    '      bytes = new Uint8Array(binary.length);',
    '      for (var i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);',
    '    } else {',
    '      var decoded = decodeURIComponent(body);',
    '      bytes = new TextEncoder().encode(decoded);',
    '    }',
    '    return new Response(bytes, { status: 200, headers: { "Content-Type": contentType } });',
    '  }',
    '  function announceHostFallback(message){',
    '    if (!message || !document || !document.body || document.querySelector("[data-sf-shell-map-fallback]")) return;',
    '    var notice = document.createElement("div");',
    '    notice.setAttribute("data-sf-shell-map-fallback", "true");',
    '    notice.textContent = String(message);',
    '    notice.style.cssText = "position:fixed;right:12px;bottom:12px;z-index:2147483647;max-width:min(320px,calc(100vw - 24px));padding:8px 10px;border:1px solid rgba(255,204,102,.7);border-radius:8px;background:rgba(20,26,40,.94);color:#ffe2a6;font:12px/1.45 sans-serif;box-shadow:0 8px 22px rgba(0,0,0,.35);pointer-events:none;";',
    '    document.body.appendChild(notice);',
    '  }',
    '  function hostResourceResponse(resource, init){',
    '    if (resource && typeof resource === "object" && typeof resource.cacheUrl === "string") {',
    '      if (!nativeFetch) throw new Error("native fetch unavailable for host cache resource");',
    '      announceHostFallback(resource.fallbackMessage);',
    '      return nativeFetch(resource.cacheUrl, init);',
    '    }',
    '    return dataUrlResponse(resource);',
    '  }',
    '  window.fetch = function(input, init){',
    '    try {',
    '      var url = (typeof input === "string") ? input : (input && input.url);',
    '      if (url && (/^https?:\\/\\//i).test(String(url))) {',
    '        return window.__sfHostFetchDataUrl(String(url)).then(function(resource){ return hostResourceResponse(resource, init); });',
    '      }',
    '    } catch (e) {}',
    '    if (nativeFetch) return nativeFetch(input, init);',
    '    return Promise.reject(new Error("fetch unavailable"));',
    '  };',
    '})();',
  ].join('\n')
}

/**
 * Some older Destiny status builds leave their React loading state active after
 * OpenSeadragon has drawn an already-decoded object URL. Do not hide that
 * state until an actual blob-backed map image reports non-zero dimensions.
 */
export function createCardShellMapReadyFallbackScript() {
  return [
    '(function(){',
    '  function hasReadyMapImage(){',
    '    var mapPage = document.querySelector(\'[data-page="map"]\');',
    '    if (!mapPage) return false;',
    '    var images = mapPage.querySelectorAll("img");',
    '    for (var i = 0; i < images.length; i++) {',
    '      var image = images[i];',
    '      var src = String(image.currentSrc || image.src || "");',
    '      if (src.indexOf("blob:") === 0 && image.complete && image.naturalWidth > 0 && image.naturalHeight > 0) return true;',
    '    }',
    '    return false;',
    '  }',
    '  function settleMapLoader(){',
    '    if (!hasReadyMapImage()) return;',
    '    var nodes = document.querySelectorAll("span, div, p");',
    '    for (var i = 0; i < nodes.length; i++) {',
    '      var node = nodes[i];',
    '      if (node.children.length !== 0) continue;',
    '      if (String(node.textContent || "").trim() !== "地图加载中，请稍候…") continue;',
    '      node.style.display = "none";',
    '      node.setAttribute("aria-hidden", "true");',
    '    }',
    '  }',
    '  if (typeof MutationObserver === "function") {',
    '    new MutationObserver(settleMapLoader).observe(document.documentElement, { childList: true, subtree: true, attributes: true });',
    '  }',
    '  window.setInterval(settleMapLoader, 250);',
    '  window.setTimeout(settleMapLoader, 0);',
    '})();',
  ].join('\n')
}
