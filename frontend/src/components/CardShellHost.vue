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
 * Host-mediate inline type=module scripts: fetch their import graph as same-origin blobs
 * and rewrite absolute/relative import specs so sandbox can execute without free network.
 */
async function hostModuleBlobGraph(entryUrl, code, cache = new Map(), depth = 0) {
  if (depth > 14) throw new Error('shell module graph too deep: ' + entryUrl)
  if (cache.has(entryUrl)) return cache.get(entryUrl)
  cache.set(entryUrl, null)
  if (code == null) {
    const res = await hostFetch(entryUrl)
    if (res.kind !== 'text' || res.body_text == null) throw new Error('module not text: ' + entryUrl)
    code = res.body_text
  }
  const re = /(?:\bfrom\s+|\bimport\s*\(?|\bimport\s+)['"`]([^'"`]+)['"`]/g
  const specs = new Set()
  let m
  while ((m = re.exec(code)) !== null) {
    const spec = m[1]
    if (!spec) continue
    if (spec.startsWith('http://') || spec.startsWith('https://') || spec.startsWith('.') || spec.startsWith('/')) {
      specs.add(spec)
    }
  }
  const rewriteMap = new Map()
  for (const spec of specs) {
    let abs = spec
    if (spec.startsWith('.') || spec.startsWith('/')) {
      try { abs = new URL(spec, entryUrl).href } catch { continue }
    }
    try {
      const child = await hostModuleBlobGraph(abs, null, cache, depth + 1)
      if (child) rewriteMap.set(spec, child)
    } catch (e) {
      console.warn('[CardShell] dep fetch failed', abs, e)
    }
  }
  if (rewriteMap.size) {
    code = code.replace(re, (full, spec) => {
      if (!rewriteMap.has(spec)) return full
      return full.replace(spec, rewriteMap.get(spec))
    })
  }
  const blob = new Blob([code], { type: 'text/javascript' })
  const blobUrl = URL.createObjectURL(blob)
  cache.set(entryUrl, blobUrl)
  return blobUrl
}

async function rewriteModuleScriptsInHtml(html, pageUrl) {
  // Match inline type=module scripts (avoid raw "</" + "script>" text in this SFC).
  const sc = 'script'
  const re = new RegExp(
    '<' + sc + '\\b([^>]*?\\btype\\s*=\\s*["\']module["\'][^>]*)>([\\s\\S]*?)</' + sc + '>',
    'gi',
  )
  const parts = []
  let last = 0
  let match
  const tasks = []
  while ((match = re.exec(html)) !== null) {
    const attrs = match[1] || ''
    if (/\bsrc\s*=/i.test(attrs)) continue
    const code = match[2] || ''
    if (!code.trim()) continue
    const start = match.index
    const end = re.lastIndex
    parts.push(html.slice(last, start))
    const placeholder = `@@SF_MOD_${tasks.length}@@`
    parts.push(placeholder)
    last = end
    const entry = pageUrl || 'https://shell.local/inline-module.js'
    tasks.push(
      hostModuleBlobGraph(entry + '#mod' + tasks.length, code).then((blobUrl) => ({
        placeholder,
        // Await classic globals (Vue/jQuery) then import host-built blob graph.
        tag:
          '<' + sc + ' type="module">' +
          'await (window.__sfShellPreloadPromise || Promise.resolve());' +
          'await import(' + JSON.stringify(blobUrl) + ');' +
          '</' + sc + '>',
      })),
    )
  }
  if (!tasks.length) return html
  parts.push(html.slice(last))
  const joined = parts.join('')
  const resolved = await Promise.all(tasks)
  let out = joined
  for (const r of resolved) {
    out = out.replace(r.placeholder, r.tag)
  }
  return out
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
  const bridgeBody = `
(function(){
  'use strict';
  var PAGE = ${JSON.stringify(pageUrl || '')};
  function ask(type, payload){
    return new Promise(function(resolve, reject){
      var id = 'sf_' + Math.random().toString(36).slice(2);
      function onMsg(ev){
        var d = ev.data || {};
        if (!d || d.__sf_shell_bridge_res !== id) return;
        window.removeEventListener('message', onMsg);
        if (d.error) reject(new Error(d.error));
        else resolve(d.result);
      }
      window.addEventListener('message', onMsg);
      parent.postMessage({ __sf_shell_bridge: true, id: id, type: type, payload: payload || {} }, '*');
      setTimeout(function(){ window.removeEventListener('message', onMsg); reject(new Error('shell bridge timeout')); }, 60000);
    });
  }
  window.__sfHostFetchText = function(url){ return ask('fetch_text', { url: url }); };
  window.__sfHostFetchDataUrl = function(url){ return ask('fetch_data_url', { url: url }); };

  function patch$(){
    if (!window.jQuery || window.jQuery.__sfLoadPatched) return !!window.jQuery;
    var $ = window.jQuery;
    $.fn.load = function(url, data, complete){
      var self = this;
      var cb = complete;
      if (typeof data === 'function') { cb = data; data = undefined; }
      var href = url;
      if (typeof url === 'string' && url.indexOf(' ') >= 0) href = url.split(' ')[0];
      window.__sfHostFetchText(href).then(function(html){
        try { self.html(html); } catch (e) {}
        if (typeof cb === 'function') cb.call(self, html, 'success', null);
      }).catch(function(err){
        if (typeof cb === 'function') cb.call(self, null, 'error', err);
        console.error('[CardShell] $.load failed', href, err);
      });
      return self;
    };
    $.getScript = function(url, success){
      return window.__sfHostFetchText(url).then(function(code){
        var s = document.createElement('script');
        s.text = code;
        document.head.appendChild(s);
        if (typeof success === 'function') success();
      });
    };
    $.__sfLoadPatched = true;
    return true;
  }
  patch$();
  var obs = new MutationObserver(function(){ patch$(); });
  obs.observe(document.documentElement, { childList: true, subtree: true });
  var n = 0; var t = setInterval(function(){ if (patch$() || ++n > 40) clearInterval(t); }, 100);

  var V = window.__sfShellVars || (window.__sfShellVars = {});
  window.getvar = function(k, d){ return V[k] !== undefined ? V[k] : d; };
  window.setvar = function(k, v){
    V[k] = v;
    try {
      parent.postMessage({ __sf_shell_bridge: true, type: 'var_write', payload: { key: k, value: v } }, '*');
    } catch (e) {}
    return v;
  };
  window.getChatVariable = window.getvar;
  window.setChatVariable = window.setvar;
  window.TavernHelper = window.TavernHelper || {
    getVariable: window.getvar,
    setVariable: window.setvar,
    getVariables: function(){ return Object.assign({}, V); },
    getCharWorldbookNames: function(){ return { primary: null }; },
    getWorldbook: async function(){ return []; },
    updateWorldbookWith: async function(){ return []; },
  };
  window.tavernHelper = window.TavernHelper;
})();
`
  const preloadBody = `
(function(){
  window.__sfShellPreloadPromise = (async function(){
    async function classic(url, pred){
      var code = await window.__sfHostFetchText(url);
      var s = document.createElement('script');
      s.text = code;
      document.head.appendChild(s);
      if (pred && !pred()) throw new Error('preload failed: ' + url);
    }
    if (!window.jQuery) {
      await classic('https://cdnjs.cloudflare.com/ajax/libs/jquery/3.7.1/jquery.min.js', function(){ return !!window.jQuery; });
      window.$ = window.jQuery;
    }
    if (!window.Vue) {
      await classic('https://cdn.jsdelivr.net/npm/vue@3.5.13/dist/vue.global.prod.js', function(){ return !!window.Vue; });
    }
    if (!window._) {
      try {
        await classic('https://cdnjs.cloudflare.com/ajax/libs/lodash.js/4.17.21/lodash.min.js', function(){ return !!window._; });
      } catch (e) { console.warn(e); }
    }
  })();
})();
`
  const fetchPatchBody = `
(function(){
  var nativeFetch = window.fetch ? window.fetch.bind(window) : null;
  window.fetch = function(input, init){
    try {
      var url = (typeof input === 'string') ? input : (input && input.url);
      if (url && (String(url).indexOf('http://') === 0 || String(url).indexOf('https://') === 0)) {
        return window.__sfHostFetchText(String(url)).then(function(text){
          return new Response(text, { status: 200, headers: { 'Content-Type': 'text/plain;charset=utf-8' } });
        });
      }
    } catch (e) {}
    if (nativeFetch) return nativeFetch(input, init);
    return Promise.reject(new Error('fetch unavailable'));
  };
})();
`
  const headInject = baseTag + sOpen + bridgeBody + sClose + sOpen + preloadBody + sClose + sOpen + fetchPatchBody + sClose

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
