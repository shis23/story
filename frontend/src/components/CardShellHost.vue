<template>
  <!--
    可见 Card Shell：独立 iframe 源（srcdoc），allow-scripts only。
    远程 HTML/JS 由宿主代持拉取后注入；jQuery $.load 桥到父窗口 fetch。
  -->
  <div class="card-shell-host w-full min-h-0 flex flex-col" :class="rootClass">
    <div
      v-if="statusLine"
      class="shrink-0 px-2 py-1 text-[11px] border-b border-line"
      :class="error ? 'text-err bg-err/10' : 'text-ink-soft bg-surface-2/50'"
    >
      {{ statusLine }}
      <button
        v-if="error"
        type="button"
        class="ml-2 underline text-err"
        @click="retry"
      >
        重试
      </button>
    </div>
    <iframe
      ref="iframeRef"
      class="w-full flex-1 min-h-[120px] border-0 bg-surface"
      :style="iframeStyle"
      sandbox="allow-scripts"
      :src="frameSrc"
      @load="onIframeLoad"
    />
  </div>
</template>

<script setup>
/**
 * CardShellHost — 可见卡前端壳（开场 home/custom_start / 状态栏 / 消息 HTML）。
 * 与隐藏 MvuJsRuntime 职责分离：本组件负责呈现 + 宿主代持网络。
 */
import { ref, watch, computed, onMounted, onUnmounted } from 'vue'
import { cardShellFetchUrl } from '../tauri-api.js'

const props = defineProps({
  /** Remote shell entry URL (preferred) */
  url: { type: String, default: null },
  /** Inline HTML when no remote entry */
  html: { type: String, default: null },
  /** Optional title for status line */
  label: { type: String, default: '' },
  /** CSS height, e.g. 280px or 60vh */
  height: { type: String, default: '280px' },
  /** compact status bar mode */
  compact: { type: Boolean, default: false },
  /** extra class on root */
  rootClass: { type: String, default: '' },
})

const emit = defineEmits(['loaded', 'error', 'message', 'var-write'])

const iframeRef = ref(null)
const srcdoc = ref(blankSrcdoc('准备加载…'))
const frameSrc = ref('about:blank')
let frameBlobUrl = null
const loading = ref(false)
const error = ref(null)
const loadedUrl = ref(null)
let loadSeq = 0
let bridgeHandler = null
/** @type {Map<string, string>} host-side store for large inline modules */
const inlineModuleSources = new Map()
let inlineModuleSeq = 0

const iframeStyle = computed(() => ({
  height: props.compact ? props.height || '96px' : props.height,
  minHeight: props.compact ? '72px' : '120px',
}))

const statusLine = computed(() => {
  if (error.value) return `壳加载失败：${error.value}`
  if (loading.value) return `宿主代持加载中… ${props.label || props.url || ''}`.trim()
  if (loadedUrl.value) return `已加载 ${props.label || loadedUrl.value}`
  return ''
})

function blankSrcdoc(msg) {
  return `<!doctype html><html><head><meta charset="utf-8"/>
<style>html,body{margin:0;padding:12px;font:12px/1.5 system-ui,sans-serif;background:#0f0f10;color:#c8c8c8}</style>
</head><body>${escapeHtml(msg)}</body></html>`
}

function revokeFrameBlob() {
  if (frameBlobUrl) {
    try { URL.revokeObjectURL(frameBlobUrl) } catch (_) {}
    frameBlobUrl = null
  }
}

function setFrameHtml(html) {
  srcdoc.value = html
  revokeFrameBlob()
  // blob: URL so WebView2 executes scripts (srcdoc often does not in Tauri).
  const blob = new Blob([html], { type: 'text/html' })
  frameBlobUrl = URL.createObjectURL(blob)
  frameSrc.value = frameBlobUrl
}

function escapeHtml(s) {
  return String(s || '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

async function hostFetch(url) {
  const res = await cardShellFetchUrl(url)
  if (res.body_text != null) return { kind: 'text', ...res }
  if (res.body_base64) {
    // binary — return as data URL for images etc.
    const ct = res.content_type || 'application/octet-stream'
    return {
      kind: 'binary',
      dataUrl: `data:${ct};base64,${res.body_base64}`,
      ...res,
    }
  }
  throw new Error('empty shell body')
}


/**
 * Replace inline type=module scripts with iframe-local importer calls.
 * Parent-origin blob: URLs are opaque to the shell iframe.
 */
async function rewriteModuleScriptsInHtml(html, pageUrl) {
  const sc = 'script'
  const re = new RegExp(
    '<' + sc + '\\b([^>]*?\\btype\\s*=\\s*["\']module["\'][^>]*)>([\\s\\S]*?)</' + sc + '>',
    'gi',
  )
  const parts = []
  let last = 0
  let match
  let n = 0
  // Clear previous sources for this load path
  // (caller may invoke prepare multiple times; keys are unique via seq)
  while ((match = re.exec(html)) !== null) {
    const attrs = match[1] || ''
    if (/\bsrc\s*=/i.test(attrs)) continue
    const code = match[2] || ''
    if (!code.trim()) continue
    parts.push(html.slice(last, match.index))
    const id = 'mod_' + (++inlineModuleSeq)
    inlineModuleSources.set(id, code)
    const entry = pageUrl || 'https://shell.local/inline-module.js'
    // Tiny module stub: fetch source from host by id, then run graph in iframe origin.
    const tag =
      '<' + sc + ' type="module">' +
      '(async()=>{' +
      'try{' +
      'await (window.__sfShellPreloadPromise || Promise.resolve());' +
      'if (typeof window.__sfShellRunInlineModuleFromHost !== "function") throw new Error("shell runner missing");' +
      'await window.__sfShellRunInlineModuleFromHost(' +
      JSON.stringify(id) +
      ', ' +
      JSON.stringify(entry + '#' + id) +
      ');' +
      '}catch(err){' +
      'try{parent.postMessage({__sf_shell_bridge:true,type:"shell_runtime",payload:{kind:"module_error",detail:String((err&&err.message)||err)}},"*");}catch(_e){}' +
      'console.error("[CardShell] module boot", err);' +
      '}' +
      '})();' +
      '</' + sc + '>'
    parts.push(tag)
    last = re.lastIndex
    n += 1
  }
  if (!n) return html
  parts.push(html.slice(last))
  return parts.join('')
}

async function prepareShellDocument(html, pageUrl) {
  const wrapped = wrapRemoteHtml(html, pageUrl)
  try {
    return await rewriteModuleScriptsInHtml(wrapped, pageUrl)
  } catch (e) {
    console.warn('[CardShell] module rewrite failed, falling back to wrapped HTML', e)
    return wrapped
  }
}

function wrapRemoteHtml(html, pageUrl) {
  // Inject bridge + ST-like globals. Shells may be full docs or head+body fragments (no <html>).
  const sOpen = '<' + 'script>'
  const sClose = '</' + 'script>'
  let baseHref = ''
  try {
    if (pageUrl) baseHref = new URL('.', pageUrl).href
  } catch (_) {
    baseHref = pageUrl || ''
  }
  const baseTag = baseHref ? `<base href="${baseHref}">` : ''
  const pageJson = JSON.stringify(pageUrl || '')

  // Bridge as string joins so regex escapes are not corrupted by template literals.
  const bridgeLines = [
    "(function(){",
    "  'use strict';",
    "  var PAGE = " + pageJson + ";",
    "  function ask(type, payload){",
    "    return new Promise(function(resolve, reject){",
    "      var id = 'sf_' + Math.random().toString(36).slice(2);",
    "      function onMsg(ev){",
    "        var d = ev.data || {};",
    "        if (!d || d.__sf_shell_bridge_res !== id) return;",
    "        window.removeEventListener('message', onMsg);",
    "        if (d.error) reject(new Error(d.error));",
    "        else resolve(d.result);",
    "      }",
    "      window.addEventListener('message', onMsg);",
    "      parent.postMessage({ __sf_shell_bridge: true, id: id, type: type, payload: payload || {} }, '*');",
    "      setTimeout(function(){ window.removeEventListener('message', onMsg); reject(new Error('shell bridge timeout')); }, 120000);",
    "    });",
    "  }",
    "  window.__sfHostFetchText = function(url){ return ask('fetch_text', { url: url }); };",
    "  window.__sfHostFetchDataUrl = function(url){ return ask('fetch_data_url', { url: url }); };",
    "  window.__sfShellLocalBlobs = [];",
    "  window.__sfShellModuleCache = Object.create(null);",
    "  window.__sfShellSourceToBlob = function(code){",
    "    var blob = new Blob([code], { type: 'text/javascript' });",
    "    var u = URL.createObjectURL(blob);",
    "    window.__sfShellLocalBlobs.push(u);",
    "    return u;",
    "  };",
    "  window.__sfShellImportSpecRe = function(){",
    "    var bs = String.fromCharCode(92);",
    "    var sq = String.fromCharCode(39);",
    "    var dq = String.fromCharCode(34);",
    "    var qcls = sq + dq;",
    "    var pat = '(?:' + bs + 'bfrom' + bs + 's+|' + bs + 'bimport' + bs + 's*' + bs + '(?|' + bs + 'bimport' + bs + 's+)[' + qcls + ']([^' + qcls + ']+)[' + qcls + ']';",
    "    return new RegExp(pat, 'g');",
    "  };",
    "  window.__sfShellResolveModuleBlob = async function(entryUrl){",
    "    async function load(url){",
    "      if (window.__sfShellModuleCache[url]) return window.__sfShellModuleCache[url];",
    "      window.__sfShellModuleCache[url] = (async function(){",
    "        var code = await window.__sfHostFetchText(url);",
    "        var re = window.__sfShellImportSpecRe();",
    "        var specs = [];",
    "        var m;",
    "        while ((m = re.exec(code)) !== null) {",
    "          var spec = m[1];",
    "          if (!spec) continue;",
    "          if (spec.indexOf('http://') === 0 || spec.indexOf('https://') === 0 || spec.charAt(0) === '.' || spec.charAt(0) === '/') specs.push(spec);",
    "        }",
    "        var map = Object.create(null);",
    "        for (var i = 0; i < specs.length; i++) {",
    "          var sp = specs[i];",
    "          var abs = sp;",
    "          if (sp.charAt(0) === '.' || sp.charAt(0) === '/') { try { abs = new URL(sp, url).href; } catch (e) { continue; } }",
    "          map[sp] = await load(abs);",
    "        }",
    "        if (Object.keys(map).length) {",
    "          re = window.__sfShellImportSpecRe();",
    "          code = code.replace(re, function(full, spec){ return map[spec] ? full.replace(spec, map[spec]) : full; });",
    "        }",
    "        return window.__sfShellSourceToBlob(code);",
    "      })();",
    "      return window.__sfShellModuleCache[url];",
    "    }",
    "    return load(entryUrl);",
    "  };",
    "  window.__sfShellImportUrl = async function(entryUrl){",
    "    var blobUrl = await window.__sfShellResolveModuleBlob(entryUrl);",
    "    return import(blobUrl);",
    "  };",
    "  window.__sfShellRunInlineModuleFromHost = async function(id, entryUrl){",
"    var code = await ask('fetch_inline_module', { id: id });",
"    return window.__sfShellRunInlineModule(code, entryUrl);",
"  };",
"  window.__sfShellBindGlobalsPreamble = function(){",
    "    var nl = String.fromCharCode(10);",
    "    return [",
    "      'const __sfG = globalThis;',",
    "      'const Vue = __sfG.Vue;',",
    "      'const $ = __sfG.$ || __sfG.jQuery;',",
    "      'const jQuery = __sfG.jQuery || __sfG.$;',",
    "      'const _ = __sfG._;',",
    "      'const getvar = __sfG.getvar;',",
    "      'const setvar = __sfG.setvar;',",
    "      'const TavernHelper = __sfG.TavernHelper || __sfG.tavernHelper;',",
    "      'const tavernHelper = __sfG.tavernHelper || __sfG.TavernHelper;',",
    "      'const SillyTavern = __sfG.SillyTavern;',",
    "      'const z = __sfG.z;',",
    "      'const Zod = __sfG.Zod || __sfG.z;'",
    "    ].join(nl) + nl;",
    "  };",
    "  window.__sfShellReport = function(kind, detail){",
    "    try { parent.postMessage({ __sf_shell_bridge: true, type: 'shell_runtime', payload: { kind: kind, detail: String(detail || '') } }, '*'); } catch (e) {}",
    "  };",
    "  window.__sfShellRunInlineModule = async function(code, entryUrl){",
    "    try {",
    "      window.__sfShellReport('module_start', entryUrl || '');",
    "      var re = window.__sfShellImportSpecRe();",
    "      var specs = [];",
    "      var m;",
    "      while ((m = re.exec(code)) !== null) {",
    "        var spec = m[1];",
    "        if (!spec) continue;",
    "        if (spec.indexOf('http://') === 0 || spec.indexOf('https://') === 0 || spec.charAt(0) === '.' || spec.charAt(0) === '/') specs.push(spec);",
    "      }",
    "      var map = Object.create(null);",
    "      for (var i = 0; i < specs.length; i++) {",
    "        var sp = specs[i];",
    "        var abs = sp;",
    "        if (sp.charAt(0) === '.' || sp.charAt(0) === '/') { try { abs = new URL(sp, entryUrl || 'https://shell.local/inline.js').href; } catch (e) { throw new Error('bad relative import ' + sp); } }",
    "        map[sp] = await window.__sfShellResolveModuleBlob(abs);",
    "        if (!map[sp]) throw new Error('dep blob empty: ' + abs);",
    "      }",
    "      if (Object.keys(map).length) {",
    "        re = window.__sfShellImportSpecRe();",
    "        code = code.replace(re, function(full, spec){ return map[spec] ? full.replace(spec, map[spec]) : full; });",
    "      }",
    "      code = window.__sfShellBindGlobalsPreamble() + code;",
    "      var blobUrl = window.__sfShellSourceToBlob(code);",
    "      var mod = await import(blobUrl);",
    "      window.__sfShellReport('module_ok', entryUrl || '');",
    "      return mod;",
    "    } catch (err) {",
    "      window.__sfShellReport('module_error', (err && err.message) || err);",
    "      throw err;",
    "    }",
    "  };",
"  function patch$(){",
    "    if (!window.jQuery || window.jQuery.__sfLoadPatched) return !!window.jQuery;",
    "    var $ = window.jQuery;",
    "    $.fn.load = function(url, data, complete){",
    "      var self = this;",
    "      var cb = complete;",
    "      if (typeof data === 'function') { cb = data; data = undefined; }",
    "      var href = url;",
    "      if (typeof url === 'string' && url.indexOf(' ') >= 0) href = url.split(' ')[0];",
    "      window.__sfHostFetchText(href).then(function(html){",
    "        try { self.html(html); } catch (e) {}",
    "        if (typeof cb === 'function') cb.call(self, html, 'success', null);",
    "      }).catch(function(err){",
    "        if (typeof cb === 'function') cb.call(self, null, 'error', err);",
    "        console.error('[CardShell] $.load failed', href, err);",
    "      });",
    "      return self;",
    "    };",
    "    $.getScript = function(url, success){",
    "      return window.__sfHostFetchText(url).then(function(code){",
    "        var s = document.createElement('script');",
    "        s.text = code;",
    "        document.head.appendChild(s);",
    "        if (typeof success === 'function') success();",
    "      });",
    "    };",
    "    $.__sfLoadPatched = true;",
    "    return true;",
    "  }",
    "  patch$();",
    "  var obs = new MutationObserver(function(){ patch$(); });",
    "  obs.observe(document.documentElement, { childList: true, subtree: true });",
    "  var n = 0; var t = setInterval(function(){ if (patch$() || ++n > 40) clearInterval(t); }, 100);",
    "  var V = window.__sfShellVars || (window.__sfShellVars = {});",
    "  window.getvar = function(k, d){ return V[k] !== undefined ? V[k] : d; };",
    "  window.setvar = function(k, v){",
    "    V[k] = v;",
    "    try { parent.postMessage({ __sf_shell_bridge: true, type: 'var_write', payload: { key: k, value: v } }, '*'); } catch (e) {}",
    "    return v;",
    "  };",
    "  window.getChatVariable = window.getvar;",
    "  window.setChatVariable = window.setvar;",
    "  window.TavernHelper = window.TavernHelper || {",
    "    getVariable: window.getvar,",
    "    setVariable: window.setvar,",
    "    getVariables: function(){ return Object.assign({}, V); },",
    "    getCharWorldbookNames: function(){ return { primary: null }; },",
    "    getWorldbook: async function(){ return []; },",
    "    updateWorldbookWith: async function(){ return []; },",
    "  };",
    "  window.tavernHelper = window.TavernHelper;",
    "  if (!window.SillyTavern) {",
    "    window.SillyTavern = {",
    "      chat: [], extensionSettings: {}, characters: [], characterId: 0,",
    "      name1: 'User', name2: 'Assistant',",
    "      POPUP_TYPE: { TEXT: 1, CONFIRM: 2, INPUT: 3 },",
    "      POPUP_RESULT: { AFFIRMATIVE: 1, NEGATIVE: 0, CANCELLED: -1 },",
    "      ToolManager: { isToolCallingSupported: function(){ return false; } },",
    "      getRequestHeaders: function(){ return { 'Content-Type': 'application/json' }; },",
    "      getCurrentChatId: function(){ return 'storyforge-shell-chat'; },",
    "      saveChat: async function(){ return true; },",
    "      saveSettingsDebounced: function(){},",
    "      callGenericPopup: async function(){ return -1; },",
    "      registerMacro: function(){}, unregisterMacro: function(){},",
    "    };",
    "  }",
    "})();",
  ]
  const preloadLines = [
    "(function(){",
    "  window.__sfShellPreloadPromise = (async function(){",
    "    async function classic(url, pred){",
    "      var code = await window.__sfHostFetchText(url);",
    "      var s = document.createElement('script');",
    "      s.text = code;",
    "      document.head.appendChild(s);",
    "      if (pred && !pred()) throw new Error('preload failed: ' + url);",
    "    }",
    "    if (!window.jQuery) {",
    "      await classic('https://cdnjs.cloudflare.com/ajax/libs/jquery/3.7.1/jquery.min.js', function(){ return !!window.jQuery; });",
    "      window.$ = window.jQuery;",
    "    }",
    "    if (!window.Vue) {",
    "      await classic('https://cdn.jsdelivr.net/npm/vue@3.5.13/dist/vue.global.prod.js', function(){ return !!window.Vue; });",
    "    }",
    "    if (!window.z || typeof window.z.object !== 'function') {",
    "      var zodUrls = [",
    "        'https://testingcf.jsdelivr.net/npm/zod@4.4.3/+esm',",
    "        'https://cdn.jsdelivr.net/npm/zod@4.4.3/+esm'",
    "      ];",
    "      var lastErr = null; var api = null;",
    "      for (var zi = 0; zi < zodUrls.length && !api; zi++) {",
    "        try {",
    "          var zcode = await window.__sfHostFetchText(zodUrls[zi]);",
    "          if (!zcode || zcode.indexOf('prefault') < 0) throw new Error('not zod v4');",
    "          var zblob = new Blob([zcode], { type: 'text/javascript' });",
    "          var zurl = URL.createObjectURL(zblob);",
    "          var zmod = await import(zurl);",
    "          var cand = null;",
    "          if (zmod && typeof zmod.object === 'function') cand = zmod;",
    "          else if (zmod && zmod.z && typeof zmod.z.object === 'function') cand = zmod.z;",
    "          else if (zmod && zmod.default && typeof zmod.default.object === 'function') cand = zmod.default;",
    "          else if (zmod && zmod.default && zmod.default.z && typeof zmod.default.z.object === 'function') cand = zmod.default.z;",
    "          if (!cand) throw new Error('zod shape unexpected');",
    "          var probe = cand.object({ a: cand.string() });",
    "          if (typeof probe.loose !== 'function') throw new Error('no loose');",
    "          if (typeof cand.string().prefault !== 'function') throw new Error('no prefault');",
    "          api = cand;",
    "        } catch (e) { lastErr = e; console.warn('[CardShell] zod load fail', zodUrls[zi], e); }",
    "      }",
    "      if (!api) throw new Error('Zod v4 load failed: ' + String((lastErr && lastErr.message) || lastErr));",
    "      var root = { z: api };",
    "      ['object','string','number','boolean','array','enum','record','union','optional','nullable','coerce','literal','any','unknown','void','null','undefined','date','bigint','symbol','tuple','map','set','lazy','pipe','transform','refine','superRefine','preprocess','custom','instanceof','promise','function','file','success','NEVER'].forEach(function(k){",
    "        try { if (typeof api[k] !== 'undefined') root[k] = api[k]; } catch (e3) {}",
    "      });",
    "      if (typeof root.object !== 'function' && typeof api.object === 'function') root.object = api.object.bind(api);",
    "      if (typeof root.string !== 'function' && typeof api.string === 'function') root.string = api.string.bind(api);",
    "      root.z = api;",
    "      window.z = root; window.Zod = root;",
    "      try { globalThis.z = root; globalThis.Zod = root; } catch (e0) {}",
    "    }",
    "    if (!window._) {",
    "      try { await classic('https://cdnjs.cloudflare.com/ajax/libs/lodash.js/4.17.21/lodash.min.js', function(){ return !!window._; }); } catch (e) { console.warn(e); }",
    "    }",
    "    if (!window.Vue) throw new Error('Vue missing after shell preload');",
    "    if (!window.z || typeof window.z.object !== 'function') throw new Error('Zod missing after shell preload');",
    "    try { parent.postMessage({ __sf_shell_bridge: true, type: 'shell_runtime', payload: { kind: 'preload_ok', detail: 'vue+jquery' } }, '*'); } catch (e0) {}",
    "  })().catch(function(err){",
    "    try { parent.postMessage({ __sf_shell_bridge: true, type: 'shell_runtime', payload: { kind: 'preload_error', detail: String((err && err.message) || err) } }, '*'); } catch (e1) {}",
    "  });",
    "})();",
  ]
  const fetchPatchLines = [
    "(function(){",
    "  var nativeFetch = window.fetch ? window.fetch.bind(window) : null;",
    "  window.fetch = function(input, init){",
    "    try {",
    "      var url = (typeof input === 'string') ? input : (input && input.url);",
    "      if (url && (String(url).indexOf('http://') === 0 || String(url).indexOf('https://') === 0)) {",
    "        return window.__sfHostFetchText(String(url)).then(function(text){",
    "          return new Response(text, { status: 200, headers: { 'Content-Type': 'text/plain;charset=utf-8' } });",
    "        });",
    "      }",
    "    } catch (e) {}",
    "    if (nativeFetch) return nativeFetch(input, init);",
    "    return Promise.reject(new Error('fetch unavailable'));",
    "  };",
    "})();",
  ]

  const headInject =
    baseTag +
    sOpen + bridgeLines.join('\n') + sClose +
    sOpen + preloadLines.join('\n') + sClose +
    sOpen + fetchPatchLines.join('\n') + sClose

  let doc = html || ''
  const hasHtml = /<html[\s>]/i.test(doc)
  const hasHead = /<head[\s>]/i.test(doc)
  const hasBody = /<body[\s>]/i.test(doc)

  if (hasHtml) {
    if (hasHead) {
      doc = doc.replace(/<head([^>]*)>/i, (m) => `${m}${headInject}`)
    } else {
      doc = doc.replace(/<html([^>]*)>/i, (m) => `${m}<head>${headInject}</head>`)
    }
    return doc
  }

  // Fragment: <head>...</head><body>...</body> (status / custom_start)
  if (hasHead || hasBody) {
    if (hasHead) {
      doc = doc.replace(/<head([^>]*)>/i, (m) => `${m}${headInject}`)
    } else {
      doc = `<head>${headInject}</head>` + doc
    }
    return `<!doctype html><html>${doc}</html>`
  }

  return `<!doctype html><html><head><meta charset="utf-8"/>${headInject}
<style>html,body{margin:0;padding:0;background:transparent;}</style>
</head><body>${doc}</body></html>`
}

async function loadShell() {
  const seq = ++loadSeq
  error.value = null
  inlineModuleSources.clear()
  loadedUrl.value = null
  loading.value = true
  try {
    if (props.url) {
      const res = await hostFetch(props.url)
      if (seq !== loadSeq) return
      if (res.kind !== 'text' || res.body_text == null) {
        throw new Error('远程壳不是文本 HTML: ' + (res.content_type || ''))
      }
      const wrapped = await prepareShellDocument(res.body_text, props.url)
      setFrameHtml(wrapped)
      loadedUrl.value = props.url
      emit('loaded', { url: props.url, fromCache: res.from_cache })
    } else if (props.html) {
      const wrapped = await prepareShellDocument(props.html, null)
      setFrameHtml(wrapped)
      loadedUrl.value = '(inline)'
      emit('loaded', { url: null, inline: true })
    } else {
      setFrameHtml(blankSrcdoc('未指定壳 URL / HTML'))
    }
  } catch (e) {
    if (seq !== loadSeq) return
    const msg = String(e?.message || e)
    error.value = msg
    setFrameHtml(blankSrcdoc('加载失败：' + msg))
    emit('error', msg)
  } finally {
    if (seq === loadSeq) loading.value = false
  }
}

function retry() {
  loadShell()
}

function onIframeLoad() {
  // no-op; bridge listens on window
}

async function onBridgeMessage(ev) {
  const d = ev.data
  if (!d || !d.__sf_shell_bridge) return
  // blob/sandbox: source identity can be flaky; accept bridge messages without hard contentWindow match
  if (d.type === 'var_write') {
    emit('message', d)
    emit('var-write', d.payload || {})
    return
  }
  if (d.type === 'shell_runtime') {
    const p = d.payload || {}
    const kind = p.kind || ''
    const detail = p.detail || ''
    if (kind === 'module_error' || kind === 'preload_error') {
      error.value = detail || kind
      emit('error', error.value)
    } else if (kind === 'module_ok' || kind === 'preload_ok' || kind === 'module_start') {
      console.info('[CardShell]', kind, detail)
    }
    emit('message', d)
    return
  }
  if (!d.id) return
  const type = d.type
  const payload = d.payload || {}
  const reply = (result, err) => {
    try {
      ev.source.postMessage(
        {
          __sf_shell_bridge_res: d.id,
          result,
          error: err || null,
        },
        '*',
      )
    } catch (_) {
      /* ignore */
    }
  }
  try {
    if (type === 'fetch_text') {
      const res = await hostFetch(payload.url)
      if (res.kind !== 'text' || res.body_text == null) {
        throw new Error('not text: ' + payload.url)
      }
      reply(res.body_text)
      return
    }
    if (type === 'fetch_data_url') {
      const res = await hostFetch(payload.url)
      if (res.kind === 'binary') reply(res.dataUrl)
      else if (res.body_text != null) {
        // text as data url
        reply(
          `data:${res.content_type || 'text/plain'};base64,` +
            btoa(unescape(encodeURIComponent(res.body_text))),
        )
      } else throw new Error('empty')
      return
    }
    if (type === 'fetch_inline_module') {
      const id = payload.id
      if (!id || !inlineModuleSources.has(id)) {
        throw new Error('unknown inline module id: ' + id)
      }
      reply(inlineModuleSources.get(id))
      return
    }
    emit('message', d)
    reply(null, 'unknown bridge type: ' + type)
  } catch (e) {
    reply(null, String(e?.message || e))
  }
}

watch(
  () => [props.url, props.html],
  () => {
    loadShell()
  },
)

onMounted(() => {
  bridgeHandler = onBridgeMessage
  window.addEventListener('message', bridgeHandler)
  loadShell()
})

onUnmounted(() => {
  if (bridgeHandler) window.removeEventListener('message', bridgeHandler)
  loadSeq++
  revokeFrameBlob()
})

defineExpose({ reload: loadShell, retry })
</script>
