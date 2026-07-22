<template>
  <!--
    隐藏 TH 运行时：顺序执行 tavern_helper.scripts（远程 ES module + inline JS）。
    宿主代持网络；iframe 仅 allow-scripts，与主应用不同源。
  -->
  <div class="tavern-helper-runtime" :class="{ 'sr-only-runtime': !showStatus }">
    <div
      v-if="showStatus"
      class="text-[11px] px-2 py-1 border border-line rounded-md bg-surface-2/40 text-ink-soft space-y-0.5"
    >
      <div class="flex items-center gap-2">
        <span class="font-medium text-ink">TavernHelper</span>
        <span>{{ summary }}</span>
        <button
          type="button"
          class="ml-auto underline text-accent"
          :disabled="running"
          @click="runAll"
        >
          {{ running ? '执行中…' : '重新执行' }}
        </button>
      </div>
      <div v-for="(s, i) in statuses" :key="i" class="truncate" :class="statusClass(s.state)">
        {{ i + 1 }}. {{ s.label }} — {{ s.state }}{{ s.detail ? ` · ${s.detail}` : '' }}
      </div>
      <div v-if="visibleButtons.length" class="flex flex-wrap gap-1 pt-1">
        <button
          v-for="b in visibleButtons"
          :key="b.name + ':' + b.scriptIndex"
          type="button"
          class="min-h-6 px-2 rounded border border-line bg-surface text-[11px] text-ink hover:border-accent-border hover:text-accent disabled:opacity-40"
          :disabled="running || !iframeReady"
          :title="b.scriptLabel"
          @click="invokeButton(b)"
        >{{ b.name }}</button>
      </div>
      <div v-if="lastWriteHint" class="text-[10px] text-ink-faint truncate">变量出站：{{ lastWriteHint }}</div>
      <div v-if="lastError" class="text-err break-words">{{ lastError }}</div>
    </div>
    <iframe
      ref="iframeRef"
      class="th-iframe"
      sandbox="allow-scripts"
      :srcdoc="srcdoc"
      @load="onIframeLoad"
    />
  </div>
</template>

<script setup>
/**
 * TavernHelperRuntime — Phase 6: ordered remote module + inline script execution.
 * No silent success: failures surface in status line.
 */
import { ref, watch, computed, onMounted, onUnmounted } from 'vue'
import { cardShellFetchUrl } from '../tauri-api.js'
import { orderedTavernHelperFromShells, collectVisibleThButtons } from '../utils/tavernHelperScripts.js'

const props = defineProps({
  /** CardShellManifest.shells */
  shells: { type: Array, default: () => [] },
  /** Show status strip (default true for debug visibility) */
  showStatus: { type: Boolean, default: true },
  /** Auto-run when shells change */
  autoRun: { type: Boolean, default: true },
})

const emit = defineEmits(['done', 'error', 'var-write', 'status'])

const iframeRef = ref(null)
const srcdoc = ref(bootstrapSrcdoc())
const running = ref(false)
const statuses = ref([])
const lastError = ref(null)
const iframeReady = ref(false)
const lastWriteHint = ref('')
let runSeq = 0
let bridgeHandler = null
const blobUrls = []

const scripts = computed(() => orderedTavernHelperFromShells(props.shells))
const visibleButtons = computed(() => collectVisibleThButtons(scripts.value))
const summary = computed(() => {
  const n = scripts.value.length
  if (!n) return '无脚本'
  const ok = statuses.value.filter((s) => s.state === 'ok').length
  const fail = statuses.value.filter((s) => s.state === 'error').length
  if (running.value) return `${ok}/${n}…`
  if (fail) return `${ok}/${n} 成功，${fail} 失败`
  if (ok === n && n) return `${n} 全部完成`
  return `${n} 待执行`
})

function statusClass(state) {
  if (state === 'ok') return 'text-ok'
  if (state === 'error') return 'text-err'
  if (state === 'running') return 'text-running'
  return 'text-ink-soft'
}

function bootstrapSrcdoc() {
  // Avoid raw script tags as contiguous text in this SFC (Vue parser would close early).
  const sOpen = '<' + 'script>'
  const sClose = '</' + 'script>'
  const body = `
(function(){
  'use strict';
  var V = {};
  function ask(type, payload){
    return new Promise(function(resolve, reject){
      var id = 'th_' + Math.random().toString(36).slice(2);
      function onMsg(ev){
        var d = ev.data || {};
        if (!d || d.__sf_th_bridge_res !== id) return;
        window.removeEventListener('message', onMsg);
        if (d.error) reject(new Error(d.error));
        else resolve(d.result);
      }
      window.addEventListener('message', onMsg);
      parent.postMessage({ __sf_th_bridge: true, id: id, type: type, payload: payload || {} }, '*');
      setTimeout(function(){
        window.removeEventListener('message', onMsg);
        reject(new Error('TH bridge timeout: ' + type));
      }, 60000);
    });
  }
  window.__sfThHostFetchText = function(url){ return ask('fetch_text', { url: url }); };
  window.__sfThHostFetchModuleBlob = function(url){ return ask('fetch_module_blob', { url: url }); };
  window.__sfThReport = function(payload){ return ask('report', payload); };
  window.getvar = function(k, d){ return V[k] !== undefined ? V[k] : d; };
  window.setvar = function(k, v){
    V[k] = v;
    parent.postMessage({ __sf_th_bridge: true, type: 'var_write', payload: { key: k, value: v } }, '*');
    return v;
  };
  window.getChatVariable = window.getvar;
  window.setChatVariable = window.setvar;
  window.eventOn = window.eventOn || function(){ return function(){}; };
  window.eventEmit = window.eventEmit || function(){};
  window.triggerSlash = window.triggerSlash || function(){ return Promise.resolve(''); };
  window.TavernHelper = window.TavernHelper || {
    getVariable: window.getvar,
    setVariable: window.setvar,
    getVariables: function(){ return Object.assign({}, V); },
    eventOn: window.eventOn,
    eventEmit: window.eventEmit,
    triggerSlash: window.triggerSlash,
  };
  window.tavernHelper = window.TavernHelper;

  window.__sfThRunScripts = async function(items){
    var results = [];
    for (var i = 0; i < items.length; i++){
      var item = items[i];
      try {
        await window.__sfThReport({ index: i, label: item.label, state: 'running' });
        if (item.kind === 'remote_url'){
          var blobUrl = await window.__sfThHostFetchModuleBlob(item.url);
          await import(blobUrl);
        } else if (item.kind === 'inline_js'){
          await new Promise(function(resolve, reject){
            try {
              var s = document.createElement('script');
              s.text = item.js;
              s.onload = function(){ resolve(); };
              s.onerror = function(e){ reject(e || new Error('inline script error')); };
              document.head.appendChild(s);
              setTimeout(resolve, 0);
            } catch (e) { reject(e); }
          });
        } else {
          throw new Error('unknown th kind: ' + item.kind);
        }
        await window.__sfThReport({ index: i, label: item.label, state: 'ok' });
        results.push({ index: i, ok: true });
      } catch (e) {
        var msg = String((e && e.message) || e);
        await window.__sfThReport({ index: i, label: item.label, state: 'error', detail: msg });
        results.push({ index: i, ok: false, error: msg });
      }
    }
    return results;
  };

  parent.postMessage({ __sf_th_bridge: true, type: 'ready' }, '*');
})();
`
  return (
    '<!doctype html><html><head><meta charset="utf-8"/>' +
    '<style>html,body{margin:0;padding:0;background:transparent}</style>' +
    sOpen +
    body +
    sClose +
    '</head><body></body></html>'
  )
}

function revokeBlobs() {
  while (blobUrls.length) {
    try {
      URL.revokeObjectURL(blobUrls.pop())
    } catch (_) {
      /* ignore */
    }
  }
}

async function hostFetchText(url) {
  const res = await cardShellFetchUrl(url)
  if (res.body_text == null) {
    throw new Error('not text: ' + url + ' (' + (res.content_type || '') + ')')
  }
  return res.body_text
}

function resolveUrl(base, rel) {
  try {
    return new URL(rel, base).href
  } catch {
    return null
  }
}

/**
 * Host-mediated ES module graph:
 * fetch → rewrite absolute/relative imports to blob: URLs → object URL.
 */
async function fetchModuleBlob(entryUrl, cache = new Map(), depth = 0) {
  if (depth > 12) throw new Error('module graph too deep: ' + entryUrl)
  if (cache.has(entryUrl)) return cache.get(entryUrl)

  // placeholder to break cycles
  cache.set(entryUrl, null)
  let code = await hostFetchText(entryUrl)

  // collect import specifiers: from 'x' | import 'x' | import('x')
  const specs = new Set()
  const re =
    /(?:\bfrom\s+|\bimport\s*\(?|\bimport\s+)['"`]([^'"`]+)['"`]/g
  let m
  while ((m = re.exec(code)) !== null) {
    const spec = m[1]
    if (!spec) continue
    if (spec.startsWith('http://') || spec.startsWith('https://') || spec.startsWith('.')) {
      specs.add(spec)
    }
  }

  const rewriteMap = new Map()
  for (const spec of specs) {
    const abs =
      spec.startsWith('http://') || spec.startsWith('https://')
        ? spec
        : resolveUrl(entryUrl, spec)
    if (!abs) continue
    try {
      const childBlob = await fetchModuleBlob(abs, cache, depth + 1)
      if (childBlob) rewriteMap.set(spec, childBlob)
    } catch (e) {
      // leave original; import will fail visibly
      console.warn('[TH] dep fetch failed', abs, e)
    }
  }

  if (rewriteMap.size) {
    code = code.replace(re, (full, spec) => {
      if (!rewriteMap.has(spec)) return full
      const blob = rewriteMap.get(spec)
      return full.replace(spec, blob)
    })
  }

  const blob = new Blob([code], { type: 'text/javascript' })
  const blobUrl = URL.createObjectURL(blob)
  blobUrls.push(blobUrl)
  cache.set(entryUrl, blobUrl)
  return blobUrl
}

async function onBridgeMessage(ev) {
  const d = ev.data
  if (!d || !d.__sf_th_bridge) return
  if (!iframeRef.value || (ev.source && ev.source !== iframeRef.value.contentWindow)) {
    // allow ready without strict source on first paint in some webviews
    if (d.type !== 'ready' && d.type !== 'var_write' && !d.id) return
  }

  if (d.type === 'ready') {
    iframeReady.value = true
    return
  }
  if (d.type === 'var_write') {
    noteVarWrite(d.payload || {})
    return
  }

  if (!d.id) return
  const reply = (result, err) => {
    try {
      ev.source.postMessage(
        { __sf_th_bridge_res: d.id, result, error: err || null },
        '*',
      )
    } catch (_) {
      /* ignore */
    }
  }

  try {
    if (d.type === 'fetch_text') {
      const text = await hostFetchText(d.payload.url)
      reply(text)
      return
    }
    if (d.type === 'fetch_module_blob') {
      const blob = await fetchModuleBlob(d.payload.url)
      reply(blob)
      return
    }
    if (d.type === 'report') {
      const p = d.payload || {}
      const idx = p.index
      if (typeof idx === 'number' && statuses.value[idx]) {
        statuses.value[idx] = {
          label: p.label || statuses.value[idx].label,
          state: p.state || 'running',
          detail: p.detail || '',
        }
        statuses.value = statuses.value.slice()
      }
      emit('status', p)
      reply(true)
      return
    }
    reply(null, 'unknown th bridge type: ' + d.type)
  } catch (e) {
    reply(null, String(e?.message || e))
  }
}

function ensureStatuses() {
  statuses.value = scripts.value.map((s) => ({
    label: s.label,
    state: 'pending',
    detail: '',
  }))
}

async function waitReady(timeoutMs = 5000) {
  const start = Date.now()
  while (!iframeReady.value) {
    if (Date.now() - start > timeoutMs) throw new Error('TH iframe not ready')
    await new Promise((r) => setTimeout(r, 30))
  }
}

async function runAll() {
  const seq = ++runSeq
  lastError.value = null
  if (!scripts.value.length) {
    ensureStatuses()
    emit('done', { ok: true, count: 0 })
    return
  }
  running.value = true
  ensureStatuses()
  revokeBlobs()
  try {
    // reload iframe clean slate
    iframeReady.value = false
    srcdoc.value = bootstrapSrcdoc()
    await waitReady()
    if (seq !== runSeq) return
    const win = iframeRef.value?.contentWindow
    if (!win || typeof win.__sfThRunScripts !== 'function') {
      throw new Error('TH runner missing in iframe')
    }
    const payload = scripts.value.map((s) => ({
      kind: s.kind,
      label: s.label,
      url: s.url,
      js: s.js,
    }))
    const results = await win.__sfThRunScripts(payload)
    if (seq !== runSeq) return
    const failed = (results || []).filter((r) => !r.ok)
    if (failed.length) {
      lastError.value = failed.map((f) => `#${f.index + 1}: ${f.error}`).join(' | ')
      emit('error', lastError.value)
      emit('done', { ok: false, results })
    } else {
      emit('done', { ok: true, results })
    }
  } catch (e) {
    if (seq !== runSeq) return
    lastError.value = String(e?.message || e)
    emit('error', lastError.value)
    emit('done', { ok: false, error: lastError.value })
  } finally {
    if (seq === runSeq) running.value = false
  }
}

function onIframeLoad() {
  // ready comes via postMessage
}

watch(
  () => props.shells,
  async () => {
    ensureStatuses()
    if (props.autoRun && scripts.value.length) {
      await runAll()
    }
  },
  { deep: true },
)

onMounted(() => {
  bridgeHandler = onBridgeMessage
  window.addEventListener('message', bridgeHandler)
  ensureStatuses()
  if (props.autoRun && scripts.value.length) {
    runAll()
  }
})

onUnmounted(() => {
  if (bridgeHandler) window.removeEventListener('message', bridgeHandler)
  runSeq++
  revokeBlobs()
})

async function invokeButton(btn) {
  lastError.value = null
  try {
    await waitReady()
    const win = iframeRef.value?.contentWindow
    if (!win) throw new Error('TH iframe missing')
    // Prefer TH slash/button bridges; MagVarUpdate exposes buttons via TavernHelper / global hooks.
    const th = win.TavernHelper || win.tavernHelper || {}
    if (typeof th.triggerSlash === 'function') {
      // Common ST pattern: /button name
      try {
        await th.triggerSlash(`/button ${btn.name}`)
      } catch (_) {
        /* fall through */
      }
    }
    if (typeof win.triggerSlash === 'function') {
      try {
        await win.triggerSlash(`/button ${btn.name}`)
      } catch (_) {
        /* fall through */
      }
    }
    // Direct event for scripts that listen
    if (typeof win.eventEmit === 'function') {
      win.eventEmit('th_button', { name: btn.name, script: btn.scriptLabel })
    }
    if (typeof th.eventEmit === 'function') {
      th.eventEmit('th_button', { name: btn.name, script: btn.scriptLabel })
    }
    // MagVarUpdate / TH often register window handlers on button click names
    const candidates = [
      btn.name,
      `button:${btn.name}`,
      `th:${btn.name}`,
    ]
    for (const key of candidates) {
      if (typeof win[key] === 'function') {
        await win[key]()
        break
      }
    }
    emit('status', { type: 'button', name: btn.name, script: btn.scriptLabel })
  } catch (e) {
    lastError.value = `按钮「${btn.name}」: ${e?.message || e}`
    emit('error', lastError.value)
  }
}

function noteVarWrite(payload) {
  const key = payload?.key
  if (!key) return
  lastWriteHint.value = `${key}`
  emit('var-write', payload || {})
}

defineExpose({ runAll, scripts, statuses, invokeButton, visibleButtons })

</script>

<style scoped>
.sr-only-runtime {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip: rect(0 0 0 0);
}
.th-iframe {
  position: absolute;
  width: 0;
  height: 0;
  border: 0;
  opacity: 0;
  pointer-events: none;
}
</style>
