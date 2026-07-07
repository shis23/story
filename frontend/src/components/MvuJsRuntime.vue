<template>
  <!-- 隐藏容器：JSR/ST JS 执行沙箱（iframe srcdoc）
       sandbox 只开 allow-scripts（不开 allow-same-origin，避免沙箱逃逸反模式）。
       shim 通过 postMessage 与父通信，不依赖 same-origin；卡脚本若越权访问 parent 将被跨 origin 阻止并降级。 -->
  <iframe
    ref="iframeRef"
    :srcdoc="iframeSrc"
    sandbox="allow-scripts"
    style="position: absolute; width: 0; height: 0; border: none; opacity: 0; pointer-events: none;"
    @load="onIframeLoad"
  />
</template>

<script setup>
/**
 * MvuJsRuntime.vue — JS Fallback WebView 容器
 *
 * 隐藏 iframe，加载卡 HTML/CSS/JS + JSR/ST API 常用子集 shim。
 * 通过 postMessage 与 iframe 通信，Tauri event 与 Rust WebViewMvuRuntime 通信。
 *
 * 通信协议：
 *   Rust → 前端（Tauri event）:
 *     mvu:load_card_assets  { html, css, js }
 *     mvu:unload_card        {}
 *     mvu:execute            { request_id, fragment_js, variables, timeout_ms }
 *
 *   前端 → iframe（postMessage）:
 *     mvu:load_assets        { html, css, js }
 *     mvu:unload             {}
 *     mvu:inject_variables   { variables }
 *     mvu:execute            { request_id, fragment_js, variables }
 *
 *   iframe → 前端（postMessage）:
 *     mvu:ready              {}
 *     mvu:assets_loaded      { error? }
 *     mvu:execute_result     { request_id, variable_updates, side_effects, error? }
 */

import { ref, onMounted, onUnmounted } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { getTrustedMvuRuntimeMessage } from '../mvu-runtime-bridge.js'

const iframeRef = ref(null)
const iframeReady = ref(false)
let assetsLoaded = false
let pendingAssets = null
let unlistenFns = []

// ─── shim（JSR/ST API 常用子集）─────────────────────────────────────────

const SHIM_SCRIPT = `(function() {
'use strict';

// === 容器隔离 ===
var C = document.createElement('div');
C.id = 'mvu-card-root';
document.body.appendChild(C);

// === 变量层 ===
var V = {};       // 变量存储
var RES = [];     // _.set 收集
var TAGS = [];    // triggerSlashTag 收集
var LISTEN = {};  // eventOn 回调

// getChatVariable / setChatVariable
window.getChatVariable = function(k, def) {
  return V[k] !== undefined ? V[k] : (def !== undefined ? def : undefined);
};
window.setChatVariable = function(k, val) {
  V[k] = val;
  RES.push({ key: k, value: val });
};
window.getvar = window.getChatVariable;
window.setvar = window.setChatVariable;

// JSR _ 别名
window._ = window._ || {};
window._.get = function(k) { return window.getChatVariable(k); };
window._.set = function(k, val) { window.setChatVariable(k, val); return val; };

// === DOM 子集（容器内）===
var _create = document.createElement.bind(document);
document.querySelector = function(s) { return C.querySelector(s); };
document.querySelectorAll = function(s) { return C.querySelectorAll(s); };
document.createElement = function(t) { return _create(t); };

// jQuery $ 子集
function W(els) {
  var a = Array.isArray(els) ? els : (els ? [els] : []);
  a.find = function(s) { return W(a.flatMap(function(e) { return Array.from(e.querySelectorAll(s)); })); };
  a.text = function(v) {
    if (v === undefined) return a.map(function(e) { return e.textContent; }).join('');
    a.forEach(function(e) { e.textContent = v; }); return a;
  };
  a.html = function(v) {
    if (v === undefined) return a.length ? a[0].innerHTML : '';
    a.forEach(function(e) { e.innerHTML = v; }); return a;
  };
  a.load = function(url, data, complete) {
    var callback = typeof data === 'function' ? data : complete;
    console.warn('[MVU] blocked jquery load:', String(url || ''));
    if (typeof callback === 'function') {
      try { callback.call(a[0] || null, '', 'error', null); }
      catch (err) { console.error('[MVU] jquery load callback error:', err); }
    }
    return a;
  };
  a.css = function(p, v) {
    if (typeof p === 'string' && v !== undefined) { a.forEach(function(e) { e.style[p] = v; }); return a; }
    if (typeof p === 'object') { a.forEach(function(e) { for (var k in p) e.style[k] = p[k]; }); return a; }
    return a.length ? getComputedStyle(a[0])[p] : '';
  };
  a.attr = function(n, v) {
    if (v === undefined) return a.length ? a[0].getAttribute(n) : '';
    a.forEach(function(e) { e.setAttribute(n, v); }); return a;
  };
  a.ready = function(fn) {
    if (typeof fn === 'function') fn.call(document);
    return a;
  };
  return a;
}
function blockedJqRequest(kind, url) {
  console.warn('[MVU] blocked jquery ' + kind + ':', String(url || ''));
  var request = {
    done: function() { return request; },
    fail: function(fn) {
      if (typeof fn === 'function') {
        try { fn.call(null, null, 'error', null); }
        catch (err) { console.error('[MVU] jquery ' + kind + ' fail callback error:', err); }
      }
      return request;
    },
    always: function(fn) {
      if (typeof fn === 'function') {
        try { fn.call(null, '', 'error', null); }
        catch (err) { console.error('[MVU] jquery ' + kind + ' always callback error:', err); }
      }
      return request;
    },
    then: function(_resolve, reject) {
      if (typeof reject === 'function') {
        try { reject.call(null, null, 'error', null); }
        catch (err) { console.error('[MVU] jquery ' + kind + ' reject callback error:', err); }
      }
      return request;
    },
    catch: function(fn) {
      if (typeof fn === 'function') {
        try { fn.call(null, null, 'error', null); }
        catch (err) { console.error('[MVU] jquery ' + kind + ' catch callback error:', err); }
      }
      return request;
    }
  };
  return request;
}
window.$ = function(s) {
  if (typeof s === 'function') { s.call(document); return W([C]); }
  if (s === document || s === window) return W([s]);
  if (typeof s === 'string') return W(C.querySelectorAll(s));
  if (s && s.nodeType) return W([s]);
  return W([]);
};
window.jQuery = window.$;
window.$.find = function(s) { return window.$(s); };
window.$.getScript = function(url) { return blockedJqRequest('getScript', url); };
window.jQuery.getScript = window.$.getScript;

// === 定时器（受控）===
var TM = [];
var MAX_TM = 20;
var _setTimeout = window.setTimeout;
var _setInterval = window.setInterval;

window.setTimeout = function(fn, ms) {
  if (TM.length >= MAX_TM) throw new Error('MVU: timer limit ' + MAX_TM);
  var id = _setTimeout(fn, Math.max(ms || 0, 10));
  TM.push(id); return id;
};
window.setInterval = function(fn, ms) {
  if (TM.length >= MAX_TM) throw new Error('MVU: timer limit ' + MAX_TM);
  var id = _setInterval(fn, Math.max(ms || 0, 50));
  TM.push(id); return id;
};
function clearTimers() {
  TM.forEach(function(id) { _setTimeout(function() { clearTimeout(id); clearInterval(id); }, 0); });
  TM = [];
}

// === ST/JSR 事件 ===
window.triggerSlashTag = function(tag) { TAGS.push(String(tag)); };
window.triggerSlash = window.triggerSlashTag;
window.eventOn = function(name, fn) {
  if (!LISTEN[name]) LISTEN[name] = [];
  LISTEN[name].push(fn);
};
window.eventSource = { on: window.eventOn, emit: function(){} };
window.registerSlashCommand = function(name) { console.log('[MVU] slash cmd: ' + name); };

// === 网络拦截 ===
window.fetch = function() { return Promise.reject(new Error('MVU: fetch disabled')); };
window.XMLHttpRequest = function() { throw new Error('MVU: XHR disabled'); };
window.WebSocket = function() { throw new Error('MVU: WebSocket disabled'); };

function runUserScript(source, vars, includeOnSlashTag) {
  var body = [
    'var variables = vars;',
    'var __st_vars = vars;',
    'var _ = api._;',
    'var $ = api.$;',
    'var triggerSlashTag = api.triggerSlashTag;',
    'var triggerSlash = api.triggerSlash;',
    'var getChatVariable = api.getChatVariable;',
    'var setChatVariable = api.setChatVariable;',
    'var getvar = api.getvar;',
    'var setvar = api.setvar;',
    includeOnSlashTag ? 'var onSlashTag = function(t){ triggerSlashTag(t); };' : '',
    String(source || '')
  ].join(String.fromCharCode(10));
  var fn = new Function('vars', 'api', body);
  return fn(vars, {
    _: window._,
    $: window.$,
    triggerSlashTag: window.triggerSlashTag,
    triggerSlash: window.triggerSlash,
    getChatVariable: window.getChatVariable,
    setChatVariable: window.setChatVariable,
    getvar: window.getvar,
    setvar: window.setvar
  });
}

function snapshotVariables() {
  var snap = {};
  for (var k in V) {
    try { snap[k] = JSON.stringify(V[k]); }
    catch { snap[k] = String(V[k]); }
  }
  return snap;
}

function collectVariableUpdates(before) {
  var merged = {};
  for (var mk in V) {
    var afterValue;
    try { afterValue = JSON.stringify(V[mk]); }
    catch { afterValue = String(V[mk]); }
    if (before[mk] !== afterValue) merged[mk] = V[mk];
  }
  RES.forEach(function(r) {
    if (!(r.key in merged) && before[r.key] === undefined) {
      merged[r.key] = V[r.key];
    }
  });
  return merged;
}

// === 消息通道 ===
window.addEventListener('message', function(e) {
  var d = e.data;
  if (!d || !d.type) return;

  if (d.type === 'mvu:inject_variables' && d.variables) {
    for (var k in d.variables) V[k] = d.variables[k];
    (LISTEN['st_chat_changed'] || []).forEach(function(fn) { try { fn(); } catch(ex) { console.error('[MVU]', ex); } });
  }

  if (d.type === 'mvu:load_assets') {
    try {
      if (d.css) { var s = document.createElement('style'); s.textContent = d.css; document.head.appendChild(s); }
      C.innerHTML = d.html || '';
      if (d.js) {
        runUserScript(d.js, V, true);
      }
      parent.postMessage({ type: 'mvu:assets_loaded' }, '*');
    } catch(err) {
      parent.postMessage({ type: 'mvu:assets_loaded', error: String(err) }, '*');
    }
  }

  if (d.type === 'mvu:unload') {
    C.innerHTML = '';
    V = {}; RES = []; TAGS = []; LISTEN = {};
    clearTimers();
    parent.postMessage({ type: 'mvu:unloaded' }, '*');
  }

  if (d.type === 'mvu:execute') {
    var rid = d.request_id;
    var vars = d.variables || {};
    for (var vk in vars) V[vk] = vars[vk];
    RES = []; TAGS = [];
    var before = snapshotVariables();

    try {
      runUserScript(d.fragment_js, V, false);
    } catch (execErr) {
      console.error('[MVU] execute error:', execErr);
    }
    clearTimers();

    var merged = collectVariableUpdates(before);
    parent.postMessage({
      type: 'mvu:execute_result',
      request_id: rid,
      variable_updates: merged,
      side_effects: TAGS.slice()
    }, '*');
  }
});

// 通知宿主 shim 已就绪
parent.postMessage({ type: 'mvu:ready' }, '*');
})();`;

const iframeSrc = '<!DOCTYPE html><html><head><meta charset="utf-8"></head><body><script>' + SHIM_SCRIPT + '<\/script></body></html>'

// ─── iframe 通信 ────────────────────────────────────────────────────────

function onIframeLoad() {
  // shim 加载后会 postMessage mvu:ready
}

function sendToIframe(msg) {
  const win = iframeRef.value?.contentWindow
  if (win) win.postMessage(msg, '*')
}

// ─── 操作方法 ───────────────────────────────────────────────────────────

function handleLoadAssets(payload) {
  if (!iframeReady.value) {
    pendingAssets = payload
    return
  }
  assetsLoaded = false
  sendToIframe({ type: 'mvu:load_assets', html: payload.html || '', css: payload.css || '', js: payload.js || '' })
}

function handleUnload() {
  assetsLoaded = false
  sendToIframe({ type: 'mvu:unload' })
  invoke('mvu_unload_ack').catch(() => {})
}

// 正在执行的 request_id → 超时定时器（兜底：卡脚本卡死时回传 error，避免 Rust 侧永久等待）
const executeTimers = new Map()

function handleExecute(payload) {
  if (!iframeReady.value) {
    invoke('mvu_execute_result', {
      requestId: payload.request_id,
      variableUpdates: {},
      sideEffects: [],
      error: 'iframe 未就绪',
    }).catch(() => {})
    return
  }
  sendToIframe({ type: 'mvu:inject_variables', variables: payload.variables || {} })
  sendToIframe({
    type: 'mvu:execute',
    request_id: payload.request_id,
    fragment_js: payload.fragment_js,
    variables: payload.variables || {},
  })
  // 超时兜底（默认 3000ms）
  const timeoutMs = payload.timeout_ms || 3000
  const timer = setTimeout(() => {
    if (executeTimers.has(payload.request_id)) {
      executeTimers.delete(payload.request_id)
      invoke('mvu_execute_result', {
        requestId: payload.request_id,
        variableUpdates: {},
        sideEffects: [],
        error: `JS 执行超时（${timeoutMs}ms）`,
      }).catch(err => console.error('[MVU] timeout result invoke failed:', err))
    }
  }, timeoutMs)
  executeTimers.set(payload.request_id, timer)
}

// ─── iframe postMessage 监听 ────────────────────────────────────────────

function onWindowMessage(event) {
  const d = getTrustedMvuRuntimeMessage(event, iframeRef.value?.contentWindow)
  if (!d) return

  if (d.type === 'mvu:ready') {
    iframeReady.value = true
    if (pendingAssets) {
      handleLoadAssets(pendingAssets)
      pendingAssets = null
    }
  }

  if (d.type === 'mvu:assets_loaded') {
    assetsLoaded = true
    invoke('mvu_load_ack', { error: d.error || null }).catch(() => {})
  }

  if (d.type === 'mvu:unloaded') {
    assetsLoaded = false
  }

  if (d.type === 'mvu:execute_result') {
    // 收到结果，清掉超时定时器
    const timer = executeTimers.get(d.request_id)
    if (timer) {
      clearTimeout(timer)
      executeTimers.delete(d.request_id)
    }
    invoke('mvu_execute_result', {
      requestId: d.request_id,
      variableUpdates: d.variable_updates || {},
      sideEffects: d.side_effects || [],
      error: d.error || null,
    }).catch(err => console.error('[MVU] execute_result invoke failed:', err))
  }
}

// ─── Tauri event 监听 ──────────────────────────────────────────────────

onMounted(async () => {
  window.addEventListener('message', onWindowMessage)

  const [u1, u2, u3] = await Promise.all([
    listen('mvu:load_card_assets', (ev) => handleLoadAssets(ev.payload)),
    listen('mvu:unload_card', () => handleUnload()),
    listen('mvu:execute', (ev) => handleExecute(ev.payload)),
  ])
  unlistenFns = [u1, u2, u3]
})

onUnmounted(() => {
  window.removeEventListener('message', onWindowMessage)
  unlistenFns.forEach(fn => fn())
  unlistenFns = []
  executeTimers.forEach(t => clearTimeout(t))
  executeTimers.clear()
})

defineExpose({
  isReady: () => iframeReady.value,
  isAssetsLoaded: () => assetsLoaded,
})
</script>
