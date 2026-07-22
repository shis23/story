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
      :src="frameSrc"
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
import { cardShellFetchUrl, getCardShellInlineJs } from '../tauri-api.js'
import { orderedTavernHelperFromShells, collectVisibleThButtons } from '../utils/tavernHelperScripts.js'

const props = defineProps({
  /** CardShellManifest.shells */
  shells: { type: Array, default: () => [] },
  /** Character id for deferred inline JS fetch */
  characterId: { type: String, default: null },
  /** Show status strip (default true for debug visibility) */
  showStatus: { type: Boolean, default: true },
  /** Auto-run when shells change */
  autoRun: { type: Boolean, default: true },
})

const emit = defineEmits(['done', 'error', 'var-write', 'status'])

const iframeRef = ref(null)
const srcdoc = ref('')
const frameSrc = ref('about:blank')
let frameBlobUrl = null
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
  // Build bootstrap as plain string joins (no nested template literal).
  const lines = [
    "(function(){",
    "  'use strict';",
    "  var V = {};",
    "  function ask(type, payload){",
    "    return new Promise(function(resolve, reject){",
    "      var id = 'th_' + Math.random().toString(36).slice(2);",
    "      function onMsg(ev){",
    "        var d = ev.data || {};",
    "        if (!d || d.__sf_th_bridge_res !== id) return;",
    "        window.removeEventListener('message', onMsg);",
    "        if (d.error) reject(new Error(d.error));",
    "        else resolve(d.result);",
    "      }",
    "      window.addEventListener('message', onMsg);",
    "      parent.postMessage({ __sf_th_bridge: true, id: id, type: type, payload: payload || {} }, '*');",
    "      setTimeout(function(){",
    "        window.removeEventListener('message', onMsg);",
    "        reject(new Error('TH bridge timeout: ' + type));",
    "      }, 120000);",
    "    });",
    "  }",
    "  window.__sfThHostFetchText = function(url){ return ask('fetch_text', { url: url }); };",
    "  window.__sfThHostFetchModuleSource = function(url){ return ask('fetch_module_source', { url: url }); };",
    "  window.__sfThReport = function(payload){ return ask('report', payload); };",
    "  window.__sfThLocalBlobs = [];",
    "  window.__sfThModuleSourceToBlob = function(code){",
    "    var blob = new Blob([code], { type: 'text/javascript' });",
    "    var u = URL.createObjectURL(blob);",
    "    window.__sfThLocalBlobs.push(u);",
    "    return u;",
    "  };",
    "  window.__sfThBindGlobalsPreamble = function(){",
    "    var nl = String.fromCharCode(10);",
    "    return [",
    "      'const __sfG = globalThis;',",
    "      'const z = __sfG.z;',",
    "      'const Zod = __sfG.Zod || __sfG.z;',",
    "      'const Vue = __sfG.Vue;',",
    "      'const _ = __sfG._;',",
    "      'const $ = __sfG.$ || __sfG.jQuery;',",
    "      'const jQuery = __sfG.jQuery || __sfG.$;',",
    "      'const getvar = __sfG.getvar;',",
    "      'const setvar = __sfG.setvar;',",
    "      'const getScriptId = __sfG.getScriptId;',",
    "      'const eventOn = __sfG.eventOn;',",
    "      'const eventEmit = __sfG.eventEmit;',",
    "      'const triggerSlash = __sfG.triggerSlash;',",
    "      'const TavernHelper = __sfG.TavernHelper || __sfG.tavernHelper;',",
    "      'const tavernHelper = __sfG.tavernHelper || __sfG.TavernHelper;',",
    "      'const registerVariableSchema = __sfG.registerVariableSchema;',",
    "      'const getChatVariable = __sfG.getChatVariable || __sfG.getvar;',",
    "      'const setChatVariable = __sfG.setChatVariable || __sfG.setvar;',",
    "      'const SillyTavern = __sfG.SillyTavern;',",
    "      'const toastr = __sfG.toastr;',",
    "      'const tavern_events = __sfG.tavern_events;',",
    "      'const getLastMessageId = __sfG.getLastMessageId;',",
    "      'const updateVariablesWith = __sfG.updateVariablesWith;'",
    "    ].join(nl) + nl;",
    "  };",
    "  window.__sfThModuleSourceToBlobBound = function(code){",
    "    return window.__sfThModuleSourceToBlob(window.__sfThBindGlobalsPreamble() + code);",
    "  };",
    "  window.getvar = function(k, d){ return V[k] !== undefined ? V[k] : d; };",
    "  window.setvar = function(k, v){",
    "    V[k] = v;",
    "    parent.postMessage({ __sf_th_bridge: true, type: 'var_write', payload: { key: k, value: v } }, '*');",
    "    return v;",
    "  };",
    "  window.getChatVariable = window.getvar;",
    "  window.setChatVariable = window.setvar;",
    "  window.eventOn = window.eventOn || function(){ return function(){}; };",
    "  window.eventEmit = window.eventEmit || function(){};",
    "  window.triggerSlash = window.triggerSlash || function(){ return Promise.resolve(''); };",
    "  window.TavernHelper = window.TavernHelper || {",
    "    getVariable: window.getvar,",
    "    setVariable: window.setvar,",
    "    getVariables: function(){ return Object.assign({}, V); },",
    "    eventOn: window.eventOn,",
    "    eventEmit: window.eventEmit,",
    "    triggerSlash: window.triggerSlash,",
    "  };",
    "  window.tavernHelper = window.TavernHelper;",
    "  window.__sfThImportSpecRe = function(){",
    "    var bs = String.fromCharCode(92);",
    "    var sq = String.fromCharCode(39);",
    "    var dq = String.fromCharCode(34);",
    "    var qcls = sq + dq;",
    "    var pat = '(?:' + bs + 'bfrom' + bs + 's+|' + bs + 'bimport' + bs + 's*' + bs + '(?|' + bs + 'bimport' + bs + 's+)[' + qcls + ']([^' + qcls + ']+)[' + qcls + ']';",
    "    return new RegExp(pat, 'g');",
    "  };",
        "  window.getScriptId = window.getScriptId || function(){ return window.__sfCurrentScriptId || 'storyforge-th'; };",
    "  window.registerVariableSchema = window.registerVariableSchema || function(){};",
    "  // Minimal ST host surface for MagVarUpdate / destined-journey scripts.",
    "  window.toastr = window.toastr || {",
    "    info: function(){ try{ console.info.apply(console, arguments); }catch(e){} },",
    "    success: function(){ try{ console.info.apply(console, arguments); }catch(e){} },",
    "    warning: function(){ try{ console.warn.apply(console, arguments); }catch(e){} },",
    "    error: function(){ try{ console.error.apply(console, arguments); }catch(e){} },",
    "  };",
    "  window.tavern_events = window.tavern_events || {",
    "    CHAT_CHANGED: 'chat_changed',",
    "    MESSAGE_RECEIVED: 'message_received',",
    "    MESSAGE_UPDATED: 'message_updated',",
    "    CHARACTER_MESSAGE_RENDERED: 'character_message_rendered',",
    "    USER_MESSAGE_RENDERED: 'user_message_rendered',",
    "    GENERATION_ENDED: 'generation_ended',",
    "    GENERATION_STARTED: 'generation_started',",
    "  };",
    "  window.getLastMessageId = window.getLastMessageId || function(){",
    "    try {",
    "      var c = (window.SillyTavern && window.SillyTavern.chat) || [];",
    "      return c.length ? c.length - 1 : -1;",
    "    } catch (e) { return -1; }",
    "  };",
    "  window.updateVariablesWith = window.updateVariablesWith || async function(fn){",
    "    var store = window.__sfThVars || (window.__sfThVars = {});",
    "    var next = typeof fn === 'function' ? await fn(store) : store;",
    "    window.__sfThVars = next || store;",
    "    return window.__sfThVars;",
    "  };",
    "  if (!window.SillyTavern) {",
    "    var chat = [];",
    "    var extensionSettings = {};",
    "    var macros = Object.create(null);",
    "    var tools = Object.create(null);",
    "    window.SillyTavern = {",
    "      chat: chat,",
    "      extensionSettings: extensionSettings,",
    "      chatCompletionSettings: {},",
    "      characters: [],",
    "      characterId: 0,",
    "      name1: 'User',",
    "      name2: 'Assistant',",
    "      POPUP_TYPE: { TEXT: 1, CONFIRM: 2, INPUT: 3 },",
    "      POPUP_RESULT: { AFFIRMATIVE: 1, NEGATIVE: 0, CANCELLED: -1, CUSTOM1: 2 },",
    "      ToolManager: {",
    "        isToolCallingSupported: function(){ return false; },",
    "        registerFunctionTool: function(){},",
    "        unregisterFunctionTool: function(){},",
    "      },",
    "      getRequestHeaders: function(){ return { 'Content-Type': 'application/json' }; },",
    "      getCurrentChatId: function(){ return 'storyforge-active-chat'; },",
    "      getChatCompletionModel: function(){ return ''; },",
    "      saveChat: async function(){ return true; },",
    "      saveSettingsDebounced: function(){},",
    "      saveSettings: async function(){ return true; },",
    "      callGenericPopup: async function(){ return window.SillyTavern.POPUP_RESULT.CANCELLED; },",
    "      registerMacro: function(name, fn){ macros[name] = fn; },",
    "      unregisterMacro: function(name){ delete macros[name]; },",
    "      registerFunctionTool: function(def){ if (def && def.name) tools[def.name] = def; },",
    "      unregisterFunctionTool: function(name){ delete tools[name]; },",
    "    };",
    "  }",
    "  // parent/top mirrors used by some cards",
    "  try { if (!window.parent) window.parent = window; } catch (eP) {}",
    "  try { if (!window.top) window.top = window; } catch (eT) {}",
    "  try { if (window.parent && !window.parent.SillyTavern) window.parent.SillyTavern = window.SillyTavern; } catch (ePS) {}",
    "  try { if (window.top && !window.top.SillyTavern) window.top.SillyTavern = window.SillyTavern; } catch (eTS) {}",

    "  window.__sfThEnsureGlobals = async function(){",
    "    async function loadClassic(url, check){",
    "      var code = await window.__sfThHostFetchText(url);",
    "      if (!code || code.length < 20) throw new Error('empty global script: ' + url);",
    "      var s = document.createElement('script');",
    "      s.text = code;",
    "      document.head.appendChild(s);",
    "      if (typeof check === 'function' && !check()) {",
    "        throw new Error('global script did not define expected symbol: ' + url);",
    "      }",
    "    }",
    "    if (!window.jQuery) {",
    "      await loadClassic('https://cdnjs.cloudflare.com/ajax/libs/jquery/3.7.1/jquery.min.js', function(){ return !!window.jQuery; });",
    "    }",
    "    window.$ = window.jQuery || window.$;",
    "    if (!window.Vue) {",
    "      await loadClassic('https://cdn.jsdelivr.net/npm/vue@3.5.13/dist/vue.global.prod.js', function(){ return !!window.Vue; });",
    "    }",
    "    // Zod v4 required (prefault/loose). Do NOT fall back to v3 — it breaks MagVarUpdate/data_schema.",
    "    if (!window.__sfZodV4Ready) {",
    "      var zodUrls = [",
    "        'https://testingcf.jsdelivr.net/npm/zod@4.4.3/+esm',",
    "        'https://cdn.jsdelivr.net/npm/zod@4.4.3/+esm'",
    "      ];",
    "      var lastErr = null;",
    "      var api = null;",
    "      for (var zi = 0; zi < zodUrls.length && !api; zi++) {",
    "        try {",
    "          var zodUrl = zodUrls[zi];",
    "          // Direct host fetch + local blob import (skip graph rewrite).",
    "          var code = await window.__sfThHostFetchText(zodUrl);",
    "          if (!code || code.length < 1000) throw new Error('zod source too small: ' + (code ? code.length : 0));",
    "          if (code.indexOf('prefault') < 0) throw new Error('zod source missing prefault (not v4?)');",
    "          var blobUrl = window.__sfThModuleSourceToBlob(code);",
    "          var zodMod = await import(blobUrl);",
    "          var cand = null;",
    "          if (zodMod && typeof zodMod.object === 'function') cand = zodMod;",
    "          else if (zodMod && zodMod.z && typeof zodMod.z.object === 'function') cand = zodMod.z;",
    "          else if (zodMod && zodMod.default && typeof zodMod.default.object === 'function') cand = zodMod.default;",
    "          else if (zodMod && zodMod.default && zodMod.default.z && typeof zodMod.default.z.object === 'function') cand = zodMod.default.z;",
    "          if (!cand) {",
    "            var keys = zodMod ? Object.keys(zodMod).slice(0, 20).join(',') : 'null';",
    "            throw new Error('zod module shape unexpected keys=' + keys);",
    "          }",
    "          // Probe v4 APIs that MagVarUpdate/data_schema need.",
    "          var probe = cand.object({ a: cand.string() });",
    "          if (typeof probe.loose !== 'function') throw new Error('zod.object().loose missing');",
    "          var sp = cand.string();",
    "          if (typeof sp.prefault !== 'function') throw new Error('zod.string().prefault missing');",
    "          api = cand;",
    "        } catch (e) {",
    "          lastErr = e;",
    "          console.warn('[TH] Zod v4 load attempt failed', zodUrls[zi], e);",
    "        }",
    "      }",
    "      if (!api) throw new Error('Zod v4 load failed: ' + String((lastErr && lastErr.message) || lastErr));",
    "      // MagVarUpdate: const ue=z; ue.object(...)",
    "      // data_schema/mvu_zod: const t=z; t.z.string(...)  — need plain object with nested .z",
    "      // Module namespace is frozen; never assign onto api itself.",
    "      var root = { z: api };",
    "      try {",
    "        Object.keys(api).forEach(function(k){",
    "          try { if (!(k in root)) root[k] = api[k]; } catch (e1) {}",
    "        });",
    "      } catch (e2) {}",
    "      // Ensure callable schema builders are on root (in case Object.keys misses)",
    "      ['object','string','number','boolean','array','enum','record','union','optional','nullable','coerce','literal','any','unknown','void','null','undefined','date','bigint','symbol','tuple','map','set','lazy','pipe','transform','refine','superRefine','preprocess','custom','instanceof','promise','function','file','success','NEVER'].forEach(function(k){",
    "        try { if (typeof api[k] !== 'undefined') root[k] = api[k]; } catch (e3) {}",
    "      });",
    "      if (typeof root.object !== 'function' && typeof api.object === 'function') root.object = api.object.bind(api);",
    "      if (typeof root.string !== 'function' && typeof api.string === 'function') root.string = api.string.bind(api);",
    "      root.z = api;",
    "      // Also allow t.z.z paths used by some scripts",
    "      try { if (!api.z) { /* frozen ok */ } } catch (e4) {}",
    "      window.z = root;",
    "      window.Zod = root;",
    "      try { globalThis.z = root; globalThis.Zod = root; } catch (e0) {}",
    "      window.__sfZodV4Ready = true;",
    "      var probe2 = root.object({ a: root.string() });",
    "      var probe3 = root.z.string();",
    "      if (typeof probe2.loose !== 'function') throw new Error('root.object().loose missing after wrap');",
    "      if (typeof probe3.prefault !== 'function') throw new Error('root.z.string().prefault missing after wrap');",
    "      console.info('[TH] Zod v4 ready (wrapped)');",
    "    }",
    "    if (!window.z || typeof window.z.object !== 'function') throw new Error('Zod global missing after load');",
    "    if (window.z && !window.z.z) { try { window.z = Object.assign({ z: window.z }, window.z); } catch (eZ) {} }",
    "    if (!window._) {",
    "      try {",
    "        await loadClassic('https://cdnjs.cloudflare.com/ajax/libs/lodash.js/4.17.21/lodash.min.js', function(){ return !!window._; });",
    "      } catch (e) {",
    "        console.warn('[TH] lodash load failed', e);",
    "        window._ = {",
    "          clamp: function(n,a,b){ return Math.min(b, Math.max(a, n)); },",
    "          get: function(o,k,d){",
    "            if (o == null) return d;",
    "            var parts = String(k || '').split('.');",
    "            var cur = o;",
    "            for (var i=0;i<parts.length;i++){ if(cur==null) return d; cur = cur[parts[i]]; }",
    "            return cur === undefined ? d : cur;",
    "          },",
    "          set: function(o,k,v){",
    "            var parts = String(k || '').split('.');",
    "            var cur = o || {};",
    "            for (var i=0;i<parts.length-1;i++){ if(cur[parts[i]]==null) cur[parts[i]]={}; cur=cur[parts[i]]; }",
    "            cur[parts[parts.length-1]] = v; return o;",
    "          },",
    "          fromPairs: function(pairs){ var o={}; (pairs||[]).forEach(function(p){ if(p) o[p[0]]=p[1]; }); return o; },",
    "          toPairs: function(o){ return Object.keys(o||{}).map(function(k){ return [k, o[k]]; }); },",
    "          take: function(a,n){ return (a||[]).slice(0,n); },",
    "          uniq: function(a){ return Array.from(new Set(a||[])); },",
    "          pick: function(o, keys){ var r={}; (keys||[]).forEach(function(k){ if(o&&k in o) r[k]=o[k]; }); return r; },",
    "          mapValues: function(o, fn){ var r={}; Object.keys(o||{}).forEach(function(k){ r[k]=fn(o[k],k); }); return r; },",
    "          size: function(o){ return o ? (Array.isArray(o)?o.length:Object.keys(o).length) : 0; },",
    "          sortBy: function(a, fn){ return (a||[]).slice().sort(function(x,y){ return (fn(x)>fn(y)?1:-1); }); },",
    "          flatMap: function(a, fn){ return [].concat.apply([], (a||[]).map(fn)); },",
    "        };",
    "      }",
    "    }",
    "    // lodash chain-ish helpers used by mvu_zod: _([...]).sortBy(...).flatMap(...).join",
    "    if (window._ && !window.__sfLodashWrapped) {",
    "      var _raw = window._;",
    "      var wrap = function(v){",
    "        return {",
    "          sortBy: function(fn){ return wrap(_raw.sortBy ? _raw.sortBy(v, fn) : (v||[]).slice().sort(function(a,b){return (fn(a)>fn(b)?1:-1);})); },",
    "          flatMap: function(fn){ return wrap(_raw.flatMap ? _raw.flatMap(v, fn) : [].concat.apply([], (v||[]).map(fn))); },",
    "          join: function(sep){ return (v||[]).join(sep); },",
    "          value: function(){ return v; },",
    "        };",
    "      };",
    "      window._ = function(v){ return Array.isArray(v) || typeof v === 'object' ? wrap(v) : _raw(v); };",
    "      Object.keys(_raw).forEach(function(k){ window._[k] = _raw[k]; });",
    "      window.__sfLodashWrapped = true;",
    "    }",
    "    if (!window.Vue) throw new Error('Vue global missing after preload');",
    "    if (!window.$) throw new Error('jQuery global missing after preload');",
    "  };",
"  window.__sfThImportUrl = async function(entryUrl){",
    "    var cache = Object.create(null);",
    "    async function load(url){",
    "      if (cache[url]) return cache[url];",
    "      cache[url] = (async function(){",
    "        var code = await window.__sfThHostFetchModuleSource(url);",
    "        var re = window.__sfThImportSpecRe();",
    "        var specs = [];",
    "        var m;",
    "        while ((m = re.exec(code)) !== null) {",
    "          var spec = m[1];",
    "          if (!spec) continue;",
    "          if (spec.indexOf('http://') === 0 || spec.indexOf('https://') === 0 || spec.charAt(0) === '.' || spec.charAt(0) === '/') {",
    "            specs.push(spec);",
    "          }",
    "        }",
    "        var map = Object.create(null);",
    "        for (var i = 0; i < specs.length; i++) {",
    "          var sp = specs[i];",
    "          var abs = sp;",
    "          if (sp.charAt(0) === '.' || sp.charAt(0) === '/') {",
    "            try { abs = new URL(sp, url).href; } catch (e) { continue; }",
    "          }",
    "          try { map[sp] = await load(abs); } catch (e) { console.warn('[TH] module dep failed', abs, e); }",
    "        }",
    "        if (Object.keys(map).length) {",
    "          re = window.__sfThImportSpecRe();",
    "          code = code.replace(re, function(full, spec){",
    "            if (!map[spec]) return full;",
    "            return full.replace(spec, map[spec]);",
    "          });",
    "        }",
    "        return window.__sfThModuleSourceToBlobBound(code);",
    "      })();",
    "      return cache[url];",
    "    }",
    "    var blobUrl = await load(entryUrl);",
    "    return import(blobUrl);",
    "  };",
    "  window.__sfThRunScripts = async function(items){",
    "    await window.__sfThEnsureGlobals();",
    "    var results = [];",
    "    for (var i = 0; i < items.length; i++){",
    "      var item = items[i];",
    "      try {",
    "        await window.__sfThReport({ index: i, label: item.label, state: 'running' });",
    "        window.__sfCurrentScriptId = String(item.label || ('th-' + i));",
    "        if (item.kind === 'remote_url'){",
    "          await window.__sfThImportUrl(item.url);",
    "        } else if (item.kind === 'inline_js'){",
    "          await new Promise(function(resolve, reject){",
    "            try {",
    "              var s = document.createElement('script');",
    "              s.text = item.js;",
    "              s.onload = function(){ resolve(); };",
    "              s.onerror = function(e){ reject(e || new Error('inline script error')); };",
    "              document.head.appendChild(s);",
    "              setTimeout(resolve, 0);",
    "            } catch (e) { reject(e); }",
    "          });",
    "        } else {",
    "          throw new Error('unknown th kind: ' + item.kind);",
    "        }",
    "        await window.__sfThReport({ index: i, label: item.label, state: 'ok' });",
    "        results.push({ index: i, ok: true });",
    "      } catch (e) {",
    "        var msg = String((e && e.message) || e);",
    "        await window.__sfThReport({ index: i, label: item.label, state: 'error', detail: msg });",
    "        results.push({ index: i, ok: false, error: msg });",
    "      }",
    "    }",
    "    return results;",
    "  };",
    "  window.addEventListener('message', function(ev){",
    "    var d = ev.data || {};",
    "    if (!d || !d.__sf_th_host) return;",
    "    if (d.type === 'run_scripts') {",
    "      window.__sfThRunScripts(d.items || []).then(function(results){",
    "        parent.postMessage({ __sf_th_bridge: true, type: 'run_done', requestId: d.requestId, results: results }, '*');",
    "      }).catch(function(err){",
    "        parent.postMessage({ __sf_th_bridge: true, type: 'run_done', requestId: d.requestId, error: String((err && err.message) || err) }, '*');",
    "      });",
    "      return;",
    "    }",
    "    if (d.type === 'button') {",
    "      try {",
    "        var name = (d.payload && d.payload.name) || '';",
    "        var th = window.TavernHelper || window.tavernHelper || {};",
    "        if (typeof th.triggerSlash === 'function') { try { th.triggerSlash('/button ' + name); } catch (e1) {} }",
    "        if (typeof window.triggerSlash === 'function') { try { window.triggerSlash('/button ' + name); } catch (e2) {} }",
    "        if (typeof window.eventEmit === 'function') { window.eventEmit('th_button', d.payload || {}); }",
    "        parent.postMessage({ __sf_th_bridge: true, type: 'button_done', requestId: d.requestId, ok: true }, '*');",
    "      } catch (e) {",
    "        parent.postMessage({ __sf_th_bridge: true, type: 'button_done', requestId: d.requestId, ok: false, error: String((e && e.message) || e) }, '*');",
    "      }",
    "    }",
    "  });",
    "  parent.postMessage({ __sf_th_bridge: true, type: 'ready' }, '*');",
"  setTimeout(function(){ parent.postMessage({ __sf_th_bridge: true, type: 'ready' }, '*'); }, 50);",
"  setTimeout(function(){ parent.postMessage({ __sf_th_bridge: true, type: 'ready' }, '*'); }, 250);",
"})();",
  ]
  const body = lines.join('\n')
  return (
    '<!doctype html><html><head><meta charset="utf-8"/>' +
    '<style>html,body{margin:0;padding:0;background:transparent}</style>' +
    sOpen +
    body +
    sClose +
    '</head><body></body></html>'
  )
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
  const blob = new Blob([html], { type: 'text/html' })
  frameBlobUrl = URL.createObjectURL(blob)
  frameSrc.value = frameBlobUrl
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
/**
 * Host-mediated ES module graph as SOURCE TEXT.
 * Parent fetches code and inlines relative deps as absolute http(s) imports so
 * the iframe can build same-origin blob URLs itself (parent blobs are opaque).
 */
async function fetchModuleSource(entryUrl, cache = new Map(), depth = 0) {
  if (depth > 12) throw new Error('module graph too deep: ' + entryUrl)
  if (cache.has(entryUrl)) return cache.get(entryUrl)

  cache.set(entryUrl, null)
  let code = await hostFetchText(entryUrl)

  const re =
    /(?:\bfrom\s+|\bimport\s*\(?|\bimport\s+)['"`]([^'"`]+)['"`]/g
  const specs = new Set()
  let m
  while ((m = re.exec(code)) !== null) {
    const spec = m[1]
    if (!spec) continue
    if (spec.startsWith('http://') || spec.startsWith('https://') || spec.startsWith('.')) {
      specs.add(spec)
    }
  }

  // Rewrite relative imports to absolute http(s) so iframe can re-fetch via host.
  if (specs.size) {
    code = code.replace(re, (full, spec) => {
      if (!(spec.startsWith('.') || spec.startsWith('/'))) return full
      const abs = resolveUrl(entryUrl, spec)
      if (!abs) return full
      return full.replace(spec, abs)
    })
  }

  // Pre-warm dependency sources (best effort) so first run fails less often.
  for (const spec of specs) {
    const abs =
      spec.startsWith('http://') || spec.startsWith('https://')
        ? spec
        : resolveUrl(entryUrl, spec)
    if (!abs || abs === entryUrl) continue
    try {
      await fetchModuleSource(abs, cache, depth + 1)
    } catch (e) {
      console.warn('[TH] dep source fetch failed', abs, e)
    }
  }

  cache.set(entryUrl, code)
  return code
}

async function onBridgeMessage(ev) {
  const d = ev.data
  if (!d || !d.__sf_th_bridge) return
  // blob: iframe is cross-origin — do not require contentWindow identity.

  if (d.type === 'ready') {
    iframeReady.value = true
    return
  }
  if (d.type === 'run_done' || d.type === 'button_done') {
    // handled by waitForBridgeEvent listeners
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
    if (d.type === 'fetch_module_blob' || d.type === 'fetch_module_source') {
      // Return source text; iframe creates its own blob: URL.
      const source = await fetchModuleSource(d.payload.url)
      reply(source)
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

async function waitReady(timeoutMs = 30000) {
  const start = Date.now()
  while (!iframeReady.value) {
    if (Date.now() - start > timeoutMs) throw new Error('TH iframe not ready')
    await new Promise((r) => setTimeout(r, 30))
  }
}

function postToIframe(message) {
  const win = iframeRef.value?.contentWindow
  if (!win) throw new Error('TH iframe missing')
  win.postMessage(message, '*')
}

function waitForBridgeEvent(type, requestId, timeoutMs = 120000) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      window.removeEventListener('message', onMsg)
      reject(new Error(`TH ${type} timeout`))
    }, timeoutMs)
    function onMsg(ev) {
      const d = ev.data || {}
      if (!d || !d.__sf_th_bridge) return
      if (d.type !== type) return
      if (requestId != null && d.requestId !== requestId) return
      clearTimeout(timer)
      window.removeEventListener('message', onMsg)
      resolve(d)
    }
    window.addEventListener('message', onMsg)
  })
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
    setFrameHtml(bootstrapSrcdoc())
    await waitReady(30000)
    if (seq !== runSeq) return
    const payload = []
    for (const s of scripts.value) {
      let js = s.js
      // deferred large inline: fetch on demand
      if (s.kind === 'inline_js' && (!js || s.deferred) && props.characterId) {
        try {
          js = await getCardShellInlineJs(props.characterId, s.label)
        } catch (e) {
          throw new Error('fetch inline TH failed: ' + s.label + ' ' + (e?.message || e))
        }
      }
      payload.push({
        kind: s.kind,
        label: s.label,
        url: s.url,
        js,
      })
    }
    const requestId = 'run_' + Date.now() + '_' + Math.random().toString(36).slice(2)
    const doneP = waitForBridgeEvent('run_done', requestId, 180000)
    postToIframe({ __sf_th_host: true, type: 'run_scripts', requestId, items: payload })
    const done = await doneP
    if (seq !== runSeq) return
    if (done.error) throw new Error(done.error)
    const results = done.results || []
    const failed = results.filter((r) => !r.ok)
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
  // blob: is opaque to parent; readiness is only via postMessage {type:'ready'}.
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
  setFrameHtml(bootstrapSrcdoc())
  if (props.autoRun && scripts.value.length) {
    // wait a tick so iframe starts loading bootstrap before runAll reloads it
    setTimeout(() => { runAll() }, 0)
  }
})

onUnmounted(() => {
  if (bridgeHandler) window.removeEventListener('message', bridgeHandler)
  runSeq++
  revokeBlobs()
  revokeFrameBlob()
})

async function invokeButton(btn) {
  lastError.value = null
  try {
    await waitReady()
    const requestId = 'btn_' + Date.now() + '_' + Math.random().toString(36).slice(2)
    const doneP = waitForBridgeEvent('button_done', requestId, 30000)
    postToIframe({
      __sf_th_host: true,
      type: 'button',
      requestId,
      payload: { name: btn.name, script: btn.scriptLabel },
    })
    const done = await doneP
    if (done && done.ok === false) throw new Error(done.error || 'button failed')
    emit('status', { type: 'button', name: btn.name, script: btn.scriptLabel })
  } catch (e) {
    lastError.value = 'button failed: ' + btn.name + ' ' + (e?.message || e)
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
