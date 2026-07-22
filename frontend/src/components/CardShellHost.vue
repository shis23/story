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

function wrapRemoteHtml(html, pageUrl) {
  // Inject bridge BEFORE content so jQuery.load can be patched after jquery arrives.
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
      setTimeout(function(){ window.removeEventListener('message', onMsg); reject(new Error('shell bridge timeout')); }, 30000);
    });
  }
  window.__sfHostFetchText = function(url){ return ask('fetch_text', { url: url }); };
  window.__sfHostFetchDataUrl = function(url){ return ask('fetch_data_url', { url: url }); };

  // Patch jQuery.load when jQuery appears
  function patch$(){
    if (!window.jQuery || window.jQuery.__sfLoadPatched) return !!window.jQuery;
    var $ = window.jQuery;
    $.fn.load = function(url, data, complete){
      var self = this;
      var cb = complete;
      if (typeof data === 'function') { cb = data; data = undefined; }
      var href = url;
      if (typeof url === 'string' && url.indexOf(' ') >= 0) {
        // jQuery classic: "url selector"
        href = url.split(' ')[0];
      }
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
  // Also poll briefly for late jQuery
  var n = 0; var t = setInterval(function(){ if (patch$() || ++n > 40) clearInterval(t); }, 100);

  // getvar/setvar: in-shell store + host outbox
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
})();
`
  const bridge = sOpen + bridgeBody + sClose

  // If the response is a full document, inject bridge + base into head; else wrap.
  const hasHtml = /<html[\s>]/i.test(html)
  if (hasHtml) {
    if (/<head[\s>]/i.test(html)) {
      return html.replace(/<head([^>]*)>/i, (m) => `${m}${baseTag}${bridge}`)
    }
    return html.replace(/<html([^>]*)>/i, (m) => `${m}<head>${baseTag}${bridge}</head>`)
  }
  return `<!doctype html><html><head><meta charset="utf-8"/>${baseTag}${bridge}
<style>html,body{margin:0;padding:0;background:transparent;}</style>
</head><body>${html}</body></html>`
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
      setFrameHtml(wrapRemoteHtml(res.body_text, props.url))
      loadedUrl.value = props.url
      emit('loaded', { url: props.url, fromCache: res.from_cache })
    } else if (props.html) {
      setFrameHtml(wrapRemoteHtml(props.html, null))
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
  // only accept from our iframe
  if (!iframeRef.value || ev.source !== iframeRef.value.contentWindow) return
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
