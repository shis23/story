/**
 * 卡片界面（档 3）：消息里的卡片壳在 Turn 尾部的 sandbox iframe 里运行。
 *
 * 壳文档由服务端拼：浏览器只说「挂第几条回复里的哪个界面」，服务端重新取展示文本、
 * 按卡清单判断能不能挂、取远程页面、注入桥接脚本，登记成一次性路径交给 iframe 加载。
 * 浏览器因此不能让 DSH 同源路径发出任意 HTML。文档响应头带 CSP（含 `sandbox
 * allow-scripts`：直接在新标签页打开也是不透明源），网络类指令钉在主机白名单上。
 *
 * 信任规则与桌面相同（`ShellAwareContent.vue`）：`.load(url)` 只自动挂载卡清单里出现过的
 * URL；内联文档要逐个对上卡正则在原文里的命中（`trustedInlineShellDocStarts`）。其余要用户
 * 点「信任并挂载」。注入的脚本大部分直接来自桌面前端（`vendor/card-shell.mjs`）；桥接与
 * 预加载按 DSH 的环境改写：模块在 iframe 里转 blob 运行，常用库直接从 CDN 加载。
 */

import { createHash, randomBytes } from 'node:crypto'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { isIP } from 'node:net'
import path from 'node:path'

import { SHELL_ONLY_BODY, renderableBlocks, splitForDsh } from './html.js'
import {
  analyzeEsmModuleSource,
  createCardShellFetchProxyScript,
  createCardShellMapReadyFallbackScript,
  createCardShellRuntimeCompatibilityScript,
  generateBridgeScript,
  mergeCardShellVariables,
  normalizeCardShellVariables,
  patchCardShellVariables,
  rewriteCardShellTopBridgeAccess,
  segmentShellContent,
  trustedInlineShellDocStarts,
  updateCardShellVariables,
} from './vendor/card-shell.mjs'

export const OPENING_KINDS = new Set(['opening_home', 'opening_custom'])

const SHELL_LABELS = {
  status: '状态栏界面',
  opening_home: '首页界面',
  opening_custom: '自定义开局界面',
  message_html: '卡片界面',
}

export const shellLabel = (kind) => SHELL_LABELS[kind] ?? SHELL_LABELS.message_html

//#region 信任与切分

/** sidecar `card_shell.manifest` → 自动挂载的依据（桌面 `trustedMessageShellUrls` / `inlineShellTriggers`）。 */
export function shellTrust(manifest) {
  const shells = Array.isArray(manifest?.shells) ? manifest.shells : []
  const urls = new Set()
  for (const shell of shells) if (shell.url) urls.add(shell.url)
  for (const url of manifest?.remote_urls ?? []) if (url) urls.add(url)
  const triggers = shells
    .filter((shell) => shell.source === 'inline_html')
    .map((shell) => ({ label: shell.label || '', trigger: shell.trigger || '' }))
  return { urls, triggers }
}

/**
 * 一条回复的尾部内容（按出现顺序）：卡片 HTML 块（档 1，净化渲染）与卡片界面（档 3）。
 * 没有卡片界面时与档 1 完全一致。界面不带内联正文——挂载时服务端重新取。
 * @returns {Array<{type:'html', blocks:Array<{html,text}>} | {type:'shell', mode, kind, url?, start?, trusted, label}>}
 */
export function tailParts(display, source, trust) {
  const segments = segmentShellContent(display)
  const trustedStarts = trustedInlineShellDocStarts(display, source || display, trust.triggers)
  const parts = []
  for (const segment of segments) {
    if (segment.type === 'text') {
      const blocks = renderableBlocks(segment.content)
      if (blocks.length === 0) continue
      const last = parts.at(-1)
      if (last?.type === 'html') last.blocks.push(...blocks)
      else parts.push({ type: 'html', blocks })
      continue
    }
    const base = { type: 'shell', mode: segment.mode, kind: segment.kind, label: shellLabel(segment.kind) }
    parts.push(
      segment.mode === 'load'
        ? { ...base, url: segment.url, trusted: trust.urls.has(segment.url) }
        : { ...base, start: segment.start, trusted: trustedStarts.has(segment.start) },
    )
  }
  return parts
}

/** 开场白原文的短指纹：记进开场轮的回放数据（`contentHash`），见 `turnTailParts`。 */
export function openingContentHash(content) {
  return typeof content === 'string' ? createHash('sha256').update(content, 'utf8').digest('hex').slice(0, 16) : null
}

/** TavernHelper `getCharWorldbookNames('current')` 报给壳的书名（与桌面相同）。 */
export function shellWorldbookName(campaignId) {
  return typeof campaignId === 'string' && campaignId ? `storyforge:campaign:${campaignId}` : ''
}

/**
 * 还在开场态：这条回复是会话当前存档对话的开场白，且对话里只有它一个节点（sidecar
 * `node_count`）。只有这时开场界面能挂、能改写开场白（桌面同样只在开局前显示序章壳）。
 */
export function isOpeningOpen(item, campaignId) {
  return Boolean(campaignId) && item?.node_index === 0 && item?.node_count === 1 && item?.by_position !== true
}

/**
 * 一条回复的尾部数据（`tailParts` 加上开场界面的状态）：
 * - 可信的开场界面（首页 / 自定义开局）在开场态标 `opening: 'open'`（挂载、能改写开场白），
 *   故事开始后标 `'closed'`（只显示说明）；不可信的照常要确认、当普通界面挂。
 * - 开场轮的回放数据带 `contentHash`：存档里的开场白已被开场界面改写（指纹对不上）时，
 *   在最前面补一段新开场的文字（DSH 正文里还是开局时的那份）。
 */
export function turnTailParts(item, ref, trust, { openingOpen = false } = {}) {
  const parts = tailParts(item.display_content, item.content, trust).map((part) =>
    part.type === 'shell' && part.trusted && OPENING_KINDS.has(part.kind) ? { ...part, opening: openingOpen ? 'open' : 'closed' } : part,
  )
  if (typeof ref?.contentHash === 'string' && ref.contentHash !== openingContentHash(item.content)) {
    const { body } = splitForDsh(item.display_content)
    if (body && body !== SHELL_ONLY_BODY) parts.unshift({ type: 'text', text: body })
  }
  return parts
}

/** 在展示文本里找浏览器要挂的那个界面（`.load` 按 URL，内联按起点）。 */
export function findShellSegment(display, wanted) {
  return segmentShellContent(display).find(
    (segment) =>
      segment.type === 'shell' &&
      segment.mode === wanted?.mode &&
      (segment.mode === 'load' ? segment.url === wanted.url : segment.start === wanted.start),
  )
}

//#endregion

//#region 壳文档

/** 与桌面 `CardShellHost.vue` 的 `CARD_SHELL_MODULE_PREAMBLE` 相同（`test/shell.test.mjs` 比对）。 */
export const CARD_SHELL_MODULE_PREAMBLE =
  [
    'const __sfG = globalThis;',
    'const Vue = __sfG.Vue;',
    'const $ = __sfG.$ || __sfG.jQuery;',
    'const jQuery = __sfG.jQuery || __sfG.$;',
    'const _ = __sfG._;',
    'const getvar = __sfG.getvar;',
    'const setvar = __sfG.setvar;',
    'const TavernHelper = __sfG.TavernHelper || __sfG.tavernHelper;',
    'const tavernHelper = __sfG.tavernHelper || __sfG.TavernHelper;',
    'const SillyTavern = __sfG.SillyTavern;',
    'const z = __sfG.z;',
    'const Zod = __sfG.Zod || __sfG.z;',
    'const getVariables = __sfG.getVariables;',
    'const setVariables = __sfG.setVariables;',
    'const insertOrAssignVariables = __sfG.insertOrAssignVariables;',
    'const deleteVariable = __sfG.deleteVariable;',
    'const updateVariablesWith = __sfG.updateVariablesWith;',
    'const getChatMessages = __sfG.getChatMessages;',
    'const getLastMessageId = __sfG.getLastMessageId;',
    'const getCurrentMessageId = __sfG.getCurrentMessageId;',
    'const triggerSlash = __sfG.triggerSlash;',
    'const eventOn = __sfG.eventOn;',
    'const eventEmit = __sfG.eventEmit;',
    'const substituteParams = __sfG.substituteParams;',
    'const getScriptId = __sfG.getScriptId;',
    'const getContext = __sfG.getContext;',
    'const Mvu = __sfG.Mvu;',
  ].join('\n') + '\n'

/** 预加载的常用库（与桌面预加载同版本、同地址）。按顺序同步加载，卡自己的脚本运行前就绪。 */
const PRELOAD_SCRIPTS = [
  'https://cdnjs.cloudflare.com/ajax/libs/jquery/3.7.1/jquery.min.js',
  'https://cdnjs.cloudflare.com/ajax/libs/lodash.js/4.17.21/lodash.min.js',
  'https://cdn.jsdelivr.net/npm/vue@3.5.13/dist/vue.global.prod.js',
  // 酒馆助手的界面环境把 VueRouter 放在全局；命定之诗的自定义开局在模块顶层就读它。
  'https://cdn.jsdelivr.net/npm/vue-router@4.5.1/dist/vue-router.global.prod.js',
  'https://cdn.jsdelivr.net/npm/ejs@3.1.10/ejs.min.js',
]
const ZOD_URLS = ['https://testingcf.jsdelivr.net/npm/zod@4.4.3/+esm', 'https://cdn.jsdelivr.net/npm/zod@4.4.3/+esm']

/** 主机名白名单 → CSP 来源（只收主机名，挡住借条目夹带的协议 / 路径；桌面 `sanitizeHosts` 同口径）。 */
function cspHosts(allowedHosts) {
  const out = []
  const seen = new Set()
  for (const raw of Array.isArray(allowedHosts) ? allowedHosts : []) {
    const host = String(raw || '').trim().toLowerCase()
    if (!host || !/^[a-z0-9][a-z0-9.-]*$/.test(host) || seen.has(host)) continue
    seen.add(host)
    out.push(`https://${host}`)
  }
  return out
}

/**
 * 壳文档的 CSP（桌面 `buildShellCspContent` 去掉 Tauri 专用来源）。`header` 时带 `sandbox
 * allow-scripts`：meta 里的 sandbox 无效，只有响应头能保证文档在任何打开方式下都是不透明源。
 */
export function shellCsp(allowedHosts, { header = false } = {}) {
  const net = cspHosts(allowedHosts)
  const join = (sources) => sources.join(' ')
  const directives = [
    "default-src 'none'",
    `script-src ${join(["'unsafe-inline'", "'unsafe-eval'", 'blob:', 'data:', ...net])}`,
    `style-src ${join(["'unsafe-inline'", 'blob:', 'data:', ...net])}`,
    `img-src ${join(['data:', 'blob:', ...net])}`,
    `font-src ${join(['data:', 'blob:', ...net])}`,
    `media-src ${join(['data:', 'blob:', ...net])}`,
    `connect-src ${join(['data:', 'blob:', ...net])}`,
    'frame-src blob: data:',
    'worker-src blob:',
    'child-src blob:',
    "object-src 'none'",
    "form-action 'none'",
  ]
  return (header ? ['sandbox allow-scripts', ...directives] : directives).join('; ')
}

/** 嵌进 `<script>` 的 JSON：转义能闭合标签或打断脚本的字符（桌面 `scriptJson` 同口径）。 */
function scriptJson(value) {
  return JSON.stringify(value)
    .replace(/</g, '\\u003c')
    .replace(/>/g, '\\u003e')
    .replace(/&/g, '\\u0026')
    .replace(/\u2028/g, '\\u2028')
    .replace(/\u2029/g, '\\u2029')
}

/** 入口模块里的相对 import 按页面地址改成绝对地址（blob 模块没有基地址）。依赖由浏览器从原地址加载。 */
export function absolutizeModuleImports(source, pageUrl) {
  if (!pageUrl) return source
  let analysis
  try {
    analysis = analyzeEsmModuleSource(source)
  } catch {
    return source
  }
  let out = source
  const replacements = analysis.imports
    .filter((imported) => imported.specifier && /^(\.{1,2}\/|\/(?!\/))/.test(imported.specifier))
    .sort((left, right) => right.start - left.start)
  for (const imported of replacements) {
    let absolute
    try {
      absolute = new URL(imported.specifier, pageUrl).href
    } catch {
      continue
    }
    out = out.slice(0, imported.start) + absolute + out.slice(imported.end)
  }
  return out
}

const MODULE_SCRIPT_RE = /<script\b([^>]*?\btype\s*=\s*["']module["'][^>]*)>([\s\S]*?)<\/script\s*>/gi

/**
 * 卡里的内联 `type=module` 脚本改成「源码块 + 运行桩」：桩等预加载结束，再把源码（前面加上
 * 与桌面相同的全局别名）转成 iframe 自己的 blob 地址 import。带 src 的模块脚本原样保留。
 */
export function rewriteModuleScripts(html, pageUrl) {
  let index = 0
  return String(html || '').replace(MODULE_SCRIPT_RE, (whole, attrs, code) => {
    if (/\bsrc\s*=/i.test(attrs) || !code.trim()) return whole
    index += 1
    const id = `m${index}`
    // 源码原本就在 <script> 里，不会含 `</script`；换个类型让浏览器不执行、只当数据。
    return (
      `<script type="text/x-storyforge-module" data-sf-module="${id}">${absolutizeModuleImports(code, pageUrl)}</script>` +
      `<script type="module">window.__sfShellRunModule(${scriptJson(id)}).catch(function(err){console.error("[CardShell] module", err);});</script>`
    )
  })
}

/**
 * 开场界面要的 `SillyTavern.chat[0]`（带 swipes）与 `reloadCurrentChat` 钩子：卡切换 swipe 后
 * 调 `reloadCurrentChat()`，钩子把选择经桥发给宿主（`opening_chat_applied`）。与桌面
 * `CardShellHost.vue` 桥脚本里的同一段逐字相同（`test/shell.test.mjs` 比对），跑在桥脚本的
 * IIFE 里、用它的 `ask`；种子为 null 时两个函数直接返回。
 */
export const OPENING_SEED_LINES = [
  '  function seedOpeningChat(){',
  '    if (!OPENING_CHAT_SEED || !window.SillyTavern) return;',
  '    var chat = window.SillyTavern.chat;',
  '    if (!Array.isArray(chat)) {',
  '      chat = [];',
  '      try { window.SillyTavern.chat = chat; } catch (eChat) {}',
  '    }',
  '    var seed = OPENING_CHAT_SEED;',
  '    var entry = {',
  "      name: seed.name || 'Assistant',",
  '      is_user: false,',
  '      is_name: true,',
  "      mes: seed.mes || '',",
  "      message: seed.message || seed.mes || '',",
  "      swipe_id: (typeof seed.swipe_id === 'number') ? seed.swipe_id : -1,",
  "      swipes: Array.isArray(seed.swipes) ? seed.swipes.slice() : [seed.mes || '']",
  '    };',
  '    if (chat.length === 0) chat.push(entry);',
  '    else {',
  '      // Preserve any host-hydrated chat body, but always ensure swipes exist so',
  "      // Destiny's「开始旅程」can switch scenario instead of no-oping.",
  '      var first = chat[0] || {};',
  '      if (!Array.isArray(first.swipes) || !first.swipes.length) first.swipes = entry.swipes.slice();',
  "      if (typeof first.swipe_id !== 'number') first.swipe_id = entry.swipe_id;",
  '      if (!first.mes && entry.mes) first.mes = entry.mes;',
  '      if (!first.message && entry.message) first.message = entry.message;',
  '      chat[0] = first;',
  '    }',
  '  }',
  '  function installOpeningReloadHook(){',
  '    if (!OPENING_CHAT_SEED || !window.SillyTavern) return;',
  '    if (window.SillyTavern.__sfOpeningReloadHooked) return;',
  '    window.SillyTavern.__sfOpeningReloadHooked = true;',
  "    var prevReload = typeof window.SillyTavern.reloadCurrentChat === 'function'",
  '      ? window.SillyTavern.reloadCurrentChat.bind(window.SillyTavern)',
  '      : null;',
  '    window.SillyTavern.reloadCurrentChat = async function(){',
  '      try {',
  '        var first = (window.SillyTavern.chat && window.SillyTavern.chat[0]) || {};',
  "        await ask('opening_chat_applied', {",
  '          swipe_id: first.swipe_id,',
  "          mes: first.mes || first.message || '',",
  '          swipes: Array.isArray(first.swipes) ? first.swipes : []',
  '        });',
  '      } catch (err) {',
  "        console.error('[CardShell] opening_chat_applied failed', err);",
  '        throw err;',
  '      }',
  '      if (prevReload) return prevReload();',
  '      return true;',
  '    };',
  '  }',
  '  try { seedOpeningChat(); installOpeningReloadHook(); } catch (eSeed) {}',
]

function bridgeScript(bridgeSession, openingChatSeed = null) {
  return [
    '(function(){',
    "  'use strict';",
    `  var BRIDGE_SESSION = ${scriptJson(bridgeSession)};`,
    `  var PREAMBLE = ${scriptJson(CARD_SHELL_MODULE_PREAMBLE)};`,
    '  window.__sfShellBridgeSession = BRIDGE_SESSION;',
    '  function ask(type, payload){',
    '    return new Promise(function(resolve, reject){',
    "      var id = 'sf_' + Math.random().toString(36).slice(2);",
    '      function onMsg(ev){',
    '        var d = ev.data || {};',
    '        if (!d || d.__sf_shell_bridge_res !== id) return;',
    "        window.removeEventListener('message', onMsg);",
    '        if (d.error) reject(new Error(d.error));',
    '        else resolve(d.result);',
    '      }',
    "      window.addEventListener('message', onMsg);",
    "      parent.postMessage({ __sf_shell_bridge: true, shellSession: BRIDGE_SESSION, id: id, type: type, payload: payload || {} }, '*');",
    "      setTimeout(function(){ window.removeEventListener('message', onMsg); reject(new Error('shell bridge timeout')); }, 120000);",
    '    });',
    '  }',
    '  window.__sfShellAsk = ask;',
    "  window.__sfHostFetchText = function(url){ return ask('fetch_text', { url: url }); };",
    "  window.__sfHostFetchDataUrl = function(url){ return ask('fetch_data_url', { url: url }); };",
    '  window.__sfShellReport = function(kind, detail){',
    "    try { parent.postMessage({ __sf_shell_bridge: true, shellSession: BRIDGE_SESSION, type: 'shell_runtime', payload: { kind: kind, detail: String(detail || '') } }, '*'); } catch (e) {}",
    '  };',
    '  window.__sfShellRunModule = async function(id){',
    '    await (window.__sfShellPreloadPromise || Promise.resolve());',
    "    var node = document.querySelector('script[data-sf-module=\"' + id + '\"]');",
    "    if (!node) throw new Error('module source missing: ' + id);",
    "    var url = URL.createObjectURL(new Blob([PREAMBLE + node.textContent], { type: 'text/javascript' }));",
    "    window.__sfShellReport('module_start', id);",
    '    try {',
    '      await import(url);',
    "      window.__sfShellReport('module_ok', id);",
    '    } catch (err) {',
    "      window.__sfShellReport('module_error', (err && err.message) || err);",
    '      throw err;',
    '    } finally {',
    '      URL.revokeObjectURL(url);',
    '    }',
    '  };',
    // 卡里的 $.fn.load / $.getScript 经宿主代取（远程 HTML 多半没有 CORS 头）。与桌面相同。
    '  function patch$(){',
    '    if (!window.jQuery || window.jQuery.__sfLoadPatched) return !!window.jQuery;',
    '    var $ = window.jQuery;',
    '    $.fn.load = function(url, data, complete){',
    '      var self = this;',
    '      var cb = complete;',
    "      if (typeof data === 'function') { cb = data; data = undefined; }",
    '      var href = url;',
    "      if (typeof url === 'string' && url.indexOf(' ') >= 0) href = url.split(' ')[0];",
    '      window.__sfHostFetchText(href).then(function(html){',
    '        try { self.html(html); } catch (e) {}',
    "        if (typeof cb === 'function') cb.call(self, html, 'success', null);",
    '      }).catch(function(err){',
    "        if (typeof cb === 'function') cb.call(self, null, 'error', err);",
    "        console.error('[CardShell] $.load failed', href, err);",
    '      });',
    '      return self;',
    '    };',
    '    $.getScript = function(url, success){',
    '      return window.__sfHostFetchText(url).then(function(code){',
    "        var s = document.createElement('script');",
    '        s.text = code;',
    '        document.head.appendChild(s);',
    "        if (typeof success === 'function') success();",
    '      });',
    '    };',
    '    $.__sfLoadPatched = true;',
    '    return true;',
    '  }',
    '  patch$();',
    '  new MutationObserver(function(){ patch$(); }).observe(document.documentElement, { childList: true, subtree: true });',
    '  var n = 0; var t = setInterval(function(){ if (patch$() || ++n > 40) clearInterval(t); }, 100);',
    `  var OPENING_CHAT_SEED = ${scriptJson(openingChatSeed && typeof openingChatSeed === 'object' ? openingChatSeed : null)};`,
    ...OPENING_SEED_LINES,
    '})();',
  ].join('\n')
}

/** Zod v4 是 ES 模块，异步加载；内联模块的运行桩等它（桌面同样等预加载）。EJS 模板设置与桌面相同。 */
function preloadScript() {
  return [
    '(function(){',
    '  window.__sfShellPreloadPromise = (async function(){',
    "    if (!window.z || typeof window.z.object !== 'function') {",
    `      var zodUrls = ${scriptJson(ZOD_URLS)};`,
    '      var lastErr = null; var api = null;',
    '      for (var zi = 0; zi < zodUrls.length && !api; zi++) {',
    '        try {',
    '          var zmod = await import(zodUrls[zi]);',
    '          var cand = null;',
    "          if (zmod && typeof zmod.object === 'function') cand = zmod;",
    "          else if (zmod && zmod.z && typeof zmod.z.object === 'function') cand = zmod.z;",
    "          else if (zmod && zmod.default && typeof zmod.default.object === 'function') cand = zmod.default;",
    "          if (!cand) throw new Error('zod shape unexpected');",
    "          if (typeof cand.string().prefault !== 'function') throw new Error('not zod v4');",
    '          api = cand;',
    "        } catch (e) { lastErr = e; console.warn('[CardShell] zod load fail', zodUrls[zi], e); }",
    '      }',
    "      if (!api) throw new Error('Zod v4 load failed: ' + String((lastErr && lastErr.message) || lastErr));",
    '      var root = Object.assign({}, api, { z: api });',
    '      window.z = root; window.Zod = root;',
    '    }',
    "    if (!window.Vue) throw new Error('Vue missing after shell preload');",
    "    if (!window.ejs || typeof window.ejs.render !== 'function') throw new Error('EJS missing after shell preload');",
    '    var shellSettings = window.extension_settings || {};',
    "    var ejsTemplate = { enabled: true, engine: 'ejs@3.1.10', render: window.ejs.render.bind(window.ejs) };",
    '    shellSettings.EjsTemplate = ejsTemplate;',
    '    window.extension_settings = shellSettings;',
    '    if (window.SillyTavern) { window.SillyTavern.extension_settings = shellSettings; window.SillyTavern.extensionSettings = shellSettings; }',
    '    if (window.TavernHelper) window.TavernHelper.renderTemplate = ejsTemplate.render;',
    "    window.__sfShellReport('preload_ok', 'vue+jquery+zod');",
    '  })().catch(function(err){',
    "    window.__sfShellReport('preload_error', (err && err.message) || err);",
    '  });',
    '})();',
  ].join('\n')
}

/** 文档高度经桥报给宿主（sandbox iframe 的父页面量不到）。与桌面相同。 */
const RESIZE_REPORTER = [
  '(function(){',
  '  var queued = false;',
  '  function report(){',
  '    queued = false;',
  '    var root = document.documentElement;',
  '    var body = document.body;',
  '    var h = Math.max(root ? root.scrollHeight : 0, body ? body.scrollHeight : 0, root ? root.offsetHeight : 0, body ? body.offsetHeight : 0);',
  "    try { parent.postMessage({ __sf_shell_bridge: true, shellSession: window.__sfShellBridgeSession, type: 'shell_resize', payload: { height: h } }, '*'); } catch (_) {}",
  '  }',
  '  function schedule(){ if (!queued) { queued = true; requestAnimationFrame(report); } }',
  "  if (typeof ResizeObserver === 'function') { new ResizeObserver(schedule).observe(document.documentElement); }",
  '  if (document.body && typeof MutationObserver === "function") { new MutationObserver(schedule).observe(document.body, { childList: true, subtree: true, attributes: true, characterData: true }); }',
  "  window.addEventListener('load', schedule);",
  '  setTimeout(schedule, 0);',
  '  setTimeout(schedule, 500);',
  '})();',
].join('\n')

/**
 * 卡片 HTML → 可以直接交给 iframe 的壳文档。
 * `worldbookName` 是 TavernHelper `getCharWorldbookNames('current')` 报的书名（桌面
 * `storyforge:campaign:<id>`，没有存档为空串）；`openingChatSeed` 只给开场界面
 * （`buildOpeningChatSeed` 的结果）。
 * @param {{html: string, pageUrl?: string|null, bridgeSession: string, pluginId: string,
 *   selectorVariables?: object, mvuStatData?: object, allowedHosts: string[],
 *   worldbookName?: string, openingChatSeed?: object|null}} options
 */
export function buildShellDocument({
  html,
  pageUrl = null,
  bridgeSession,
  pluginId,
  selectorVariables = {},
  mvuStatData = {},
  allowedHosts,
  worldbookName = '',
  openingChatSeed = null,
}) {
  const script = (code) => `<script>${code}</script>`
  let baseHref = ''
  if (pageUrl) {
    try {
      baseHref = new URL('.', pageUrl).href
    } catch {
      baseHref = ''
    }
  }
  const headInject = [
    `<meta http-equiv="Content-Security-Policy" content="${shellCsp(allowedHosts).replace(/"/g, '&quot;')}">`,
    baseHref ? `<base href="${baseHref.replace(/"/g, '%22')}">` : '',
    generateBridgeScript(pluginId, '*'),
    script(bridgeScript(bridgeSession, openingChatSeed)),
    script(createCardShellRuntimeCompatibilityScript({ worldbookName, selectorVariables, mvuStatData })),
    ...PRELOAD_SCRIPTS.map((url) => `<script src="${url}"></script>`),
    script(preloadScript()),
    script(createCardShellFetchProxyScript()),
    script(createCardShellMapReadyFallbackScript()),
    script(RESIZE_REPORTER),
  ].join('')

  let doc = rewriteModuleScripts(rewriteCardShellTopBridgeAccess(html), pageUrl)
  const hasHtml = /<html[\s>]/i.test(doc)
  const hasHead = /<head[\s>]/i.test(doc)
  const hasBody = /<body[\s>]/i.test(doc)
  if (hasHtml) {
    return hasHead
      ? doc.replace(/<head([^>]*)>/i, (tag) => `${tag}${headInject}`)
      : doc.replace(/<html([^>]*)>/i, (tag) => `${tag}<head>${headInject}</head>`)
  }
  if (hasHead || hasBody) {
    doc = hasHead ? doc.replace(/<head([^>]*)>/i, (tag) => `${tag}${headInject}`) : `<head>${headInject}</head>${doc}`
    return `<!doctype html><html>${doc}</html>`
  }
  return `<!doctype html><html><head><meta charset="utf-8">${headInject}<style>html,body{margin:0;padding:0;background:transparent;}</style></head><body>${doc}</body></html>`
}

//#endregion

//#region 一次性文档路径

/**
 * 登记过的壳文档：随机 token → 文档。iframe 重新加载（卡调 `location.reload()`）要能再取，
 * 所以不是取一次就删；按存活时间、条数、总字节淘汰最旧的。
 */
export function createDocRegistry({ maxDocs = 64, maxBytes = 64 * 1024 * 1024, ttlMs = 60 * 60 * 1000, now = Date.now } = {}) {
  const docs = new Map()
  let bytes = 0
  const remove = (token) => {
    const doc = docs.get(token)
    if (!doc) return
    docs.delete(token)
    bytes -= doc.size
  }
  const prune = () => {
    const cutoff = now() - ttlMs
    for (const [token, doc] of docs) if (doc.at < cutoff) remove(token)
    for (const token of docs.keys()) {
      if (docs.size <= maxDocs && bytes <= maxBytes) break
      remove(token)
    }
  }
  return {
    put(doc) {
      const token = randomBytes(24).toString('base64url')
      const size = Buffer.byteLength(doc.html, 'utf8')
      docs.set(token, { ...doc, size, at: now() })
      bytes += size
      prune()
      return token
    },
    get(token) {
      prune()
      return typeof token === 'string' ? (docs.get(token) ?? null) : null
    },
    delete: remove,
    get size() {
      return docs.size
    },
  }
}

/** 壳文档响应：CSP 响应头（含 sandbox）、不缓存、不带 Referer。 */
export function shellDocResponse(doc) {
  return new Response(doc.html, {
    headers: {
      'content-type': 'text/html; charset=utf-8',
      'content-security-policy': shellCsp(doc.allowedHosts, { header: true }),
      'cache-control': 'no-store',
      'referrer-policy': 'no-referrer',
      'x-content-type-options': 'nosniff',
    },
  })
}

export const newBridgeSession = () => `sfshell:${randomBytes(16).toString('base64url')}`
export const newShellPluginId = () => `card-shell-${randomBytes(6).toString('hex')}`

//#endregion

//#region 宿主代取（桌面 `card_shell_fetch_url` 同语义）

const MAX_REDIRECTS = 5

function shellFetchUrl(raw, allowed) {
  let url
  try {
    url = new URL(String(raw ?? ''))
  } catch {
    throw Object.assign(new Error(`不是合法的 URL：${raw}`), { status: 400 })
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') {
    throw Object.assign(new Error(`只支持 http(s)：${raw}`), { status: 400 })
  }
  const host = url.hostname.toLowerCase()
  const bare = host.replace(/^\[|\]$/g, '')
  if (isIP(bare)) throw Object.assign(new Error(`不能代取 IP 地址：${raw}`), { status: 400 })
  if (!allowed.has(host)) {
    throw Object.assign(new Error(`URL host 不在 allowlist: ${host}（url=${url.href}）`), { status: 400 })
  }
  return url
}

/** 与桌面 `to_result` 相同的文本判定。 */
export function isTextResource(contentType, url) {
  const type = String(contentType || '')
  return (
    type.startsWith('text/') ||
    ['javascript', 'json', 'xml', 'svg'].some((kind) => type.includes(kind)) ||
    ['.js', '.css', '.html', '.htm', '.mjs'].some((ext) => url.endsWith(ext))
  )
}

/** 缓存文件名：URL 的 sha256 + 末段（桌面 `cache_path_for_url` 同口径）。 */
export function cacheName(url) {
  const hex = createHash('sha256').update(url).digest('hex')
  const suffix = (url.split('/').pop() || 'bin').replace(/[^A-Za-z0-9._-]/g, '').slice(0, 48)
  return suffix ? `${hex}_${suffix}` : hex
}

async function readLimited(response, maxBytes, url) {
  const declared = Number(response.headers.get('content-length'))
  if (Number.isFinite(declared) && declared > maxBytes) {
    throw new Error(`shell resource too large: ${declared} bytes (max ${maxBytes}) url=${url}`)
  }
  const chunks = []
  let received = 0
  for await (const chunk of response.body ?? []) {
    received += chunk.byteLength
    if (received > maxBytes) throw new Error(`shell resource too large: > ${maxBytes} bytes url=${url}`)
    chunks.push(chunk)
  }
  return Buffer.concat(chunks)
}

/**
 * 宿主代取：白名单主机、拒绝 IP、手动跟随重定向且每一跳都复核、40 MB 上限、30 秒超时、
 * 磁盘缓存（首取即冻结，与桌面相同）。
 */
export function createShellFetcher({ cacheDir, allowedHosts, fetchImpl = fetch, maxBytes = 40 * 1024 * 1024, timeoutMs = 30_000 }) {
  const allowed = () => new Set((allowedHosts() ?? []).map((host) => String(host).toLowerCase()))
  const cachePath = (url) => path.join(cacheDir, cacheName(url))

  function readCache(url) {
    try {
      const file = cachePath(url)
      const meta = JSON.parse(readFileSync(`${file}.meta.json`, 'utf8'))
      return { bytes: readFileSync(file), contentType: meta.content_type || 'application/octet-stream' }
    } catch {
      return null
    }
  }

  function writeCache(url, contentType, bytes) {
    mkdirSync(cacheDir, { recursive: true })
    const file = cachePath(url)
    writeFileSync(file, bytes)
    writeFileSync(`${file}.meta.json`, JSON.stringify({ url, content_type: contentType, byte_len: bytes.length }, null, 2))
  }

  const toResult = (url, contentType, bytes, fromCache) => {
    const text = isTextResource(contentType, url)
    return {
      url,
      content_type: contentType,
      body_text: text ? bytes.toString('utf8') : null,
      body_base64: text ? null : bytes.toString('base64'),
      from_cache: fromCache,
      byte_len: bytes.length,
    }
  }

  return async function fetchShellResource(rawUrl) {
    const hosts = allowed()
    const first = shellFetchUrl(rawUrl, hosts)
    const cached = readCache(first.href)
    if (cached) return toResult(first.href, cached.contentType, cached.bytes, true)
    let current = first
    for (let hop = 0; ; hop += 1) {
      const response = await fetchImpl(current.href, {
        redirect: 'manual',
        signal: AbortSignal.timeout(timeoutMs),
        headers: { 'user-agent': 'StoryForge-CardShell/0.1' },
      })
      if (response.status >= 300 && response.status < 400) {
        if (hop >= MAX_REDIRECTS) throw new Error(`shell fetch exceeded redirect limit: ${first.href}`)
        const location = response.headers.get('location')
        if (!location) throw new Error(`shell redirect has no valid Location: ${current.href}`)
        current = shellFetchUrl(new URL(location, current).href, hosts)
        continue
      }
      if (!response.ok) throw new Error(`shell fetch HTTP ${response.status}: ${first.href}`)
      const contentType = response.headers.get('content-type') || 'application/octet-stream'
      const bytes = await readLimited(response, maxBytes, first.href)
      writeCache(first.href, contentType, bytes)
      return toResult(first.href, contentType, bytes, false)
    }
  }
}

//#endregion

//#region 选择器变量桶（壳自己的 getVariables / setVariables）

/** 整份桶的上限，与 app-service `CARD_SHELL_VARIABLES_MAX_BYTES` 相同（后端照样会拒）。 */
export const CARD_SHELL_VARIABLES_MAX_BYTES = 2 * 1024 * 1024

/**
 * 壳用 TavernHelper 变量接口存的界面状态。关联了存档的会话写回存档的 Campaign 变量
 * `__storyforge_card_shell_variables`（与桌面同一个键、同一个后端入口
 * `set_card_shell_variables`），DSH 重启后还在；没关联存档的会话只放进程内存。
 *
 * - 同一作用域的读改写串行（桌面 `enqueueCardShellVariableMutation` 同理：两个壳先后写，
 *   后写的不会抹掉先写的键）。
 * - 进程内快照是读写依据，回复壳不等落盘：sidecar 的写入要排在写作锁后面，一轮写作期间
 *   壳里的操作不该卡住几十秒。
 * - 落盘按作用域合并：上一次还没写完时只记一笔「还有更新」，写完再把最新快照写一次；
 *   失败记日志，下一次写入会带着整份快照重试。
 *
 * `scope` 为 `{ key, campaignId }`：`campaignId` 为空时只在内存里。
 *
 * @param {{ load: (campaignId: string) => Promise<unknown>,
 *   save: (campaignId: string, variables: object) => Promise<unknown>,
 *   log?: (line: string) => void, maxBytes?: number }} options
 */
export function createShellVariableStore({ load, save, log = () => {}, maxBytes = CARD_SHELL_VARIABLES_MAX_BYTES }) {
  const snapshots = new Map()
  const chains = new Map()
  const saving = new Map()

  function serial(key, task) {
    const run = (chains.get(key) ?? Promise.resolve()).then(task)
    const tail = run.catch(() => undefined)
    chains.set(key, tail)
    void tail.then(() => {
      if (chains.get(key) === tail) chains.delete(key)
    })
    return run
  }

  async function current(scope) {
    if (snapshots.has(scope.key)) return snapshots.get(scope.key)
    const loaded = scope.campaignId ? normalizeCardShellVariables(await load(scope.campaignId)) : {}
    snapshots.set(scope.key, loaded)
    return loaded
  }

  function persist(scope) {
    const pending = saving.get(scope.key)
    if (pending) {
      pending.dirty = true
      return
    }
    const state = { dirty: true, done: null }
    saving.set(scope.key, state)
    state.done = (async () => {
      while (state.dirty) {
        state.dirty = false
        try {
          await save(scope.campaignId, snapshots.get(scope.key))
        } catch (error) {
          log(`card shell variables not saved: campaign=${scope.campaignId} ${error?.message ?? error}`)
        }
      }
      saving.delete(scope.key)
    })()
  }

  return {
    get: (scope) => serial(scope.key, () => current(scope)),
    apply: (scope, selector, variables, mode) =>
      serial(scope.key, async () => {
        const snapshot = await current(scope)
        const next =
          mode === 'patch'
            ? patchCardShellVariables(snapshot, selector, variables)
            : mode === 'merge'
              ? mergeCardShellVariables(snapshot, selector, variables)
              : updateCardShellVariables(snapshot, selector, variables)
        if (Buffer.byteLength(JSON.stringify(next), 'utf8') > maxBytes) {
          throw Object.assign(new Error(`界面状态超过 ${maxBytes / 1024} KiB`), { status: 413 })
        }
        snapshots.set(scope.key, next)
        if (scope.campaignId) persist(scope)
        return next
      }),
    /** 等手上的落盘写完（含写的过程中又来的更新）。 */
    async flush() {
      while (saving.size > 0) await Promise.all([...saving.values()].map((state) => state.done))
    },
  }
}

//#endregion
