<template>
  <div class="plugin-host" :class="compact ? 'plugin-host-compact' : ''">
    <iframe
      v-if="iframeSrc"
      ref="iframeRef"
      :src="iframeSrc"
      sandbox="allow-scripts"
      class="plugin-iframe"
      :style="iframeStyle"
      @load="onIframeLoad"
    />
    <div
      v-for="entry in slotEntries"
      :key="entry.slot"
      v-html="entry.html"
      class="plugin-slot-content"
      :data-plugin-slot="entry.slot"
    />
  </div>
</template>

<script>
export const PLUGIN_HOST_DEFAULT_SLOT = 'default'
export const PLUGIN_HOST_HOOK_READY_TIMEOUT_MS = 1000

export function normalizePluginHostSlot(slot) {
  const normalized = typeof slot === 'string' ? slot.trim() : ''
  return normalized || PLUGIN_HOST_DEFAULT_SLOT
}

export function applyPluginSlotMount(currentSlots, mount) {
  const slot = normalizePluginHostSlot(mount?.slot)
  const html = typeof mount?.html === 'string' ? mount.html : ''
  const nextSlots = { ...(currentSlots || {}) }

  if (html) {
    nextSlots[slot] = html
  } else {
    delete nextSlots[slot]
  }

  return nextSlots
}

export function getPluginSlotEntries(slots) {
  return Object.entries(slots || {})
    .filter(([, html]) => typeof html === 'string' && html.length > 0)
    .map(([slot, html]) => ({ slot, html }))
}

/**
 * M-24：插件帧的每次文档装载都要一枚一次性握手令牌。令牌由宿主生成并只嵌入
 * 该文档的桥脚本，所以「帧自我导航」换进来的新文档拿不到它；宿主据此拒绝把
 * 该帧当作可信来源（WindowProxy 身份校验本身挡不住自导航）。
 */
export function createPluginBridgeHandshakeToken() {
  const random = Math.random().toString(36).slice(2)
  const stamp = Date.now().toString(36)
  return `sfh_${stamp}_${random}`
}

/** 已完成的握手必须匹配当前文档令牌，空令牌恒不匹配（fail closed）。 */
export function isPluginBridgeHandshakeValid(expected, received) {
  return Boolean(expected) && received === expected
}

/**
 * M-31a：插件 slot HTML 插入宿主 DOM 前的消毒配置。
 *
 * 刻意在 DOMPurify 默认配置上 FORBID `<style>`：默认允许 `<style>`
 * （dompurify 3.4.12 `src/tags.ts` 默认白名单含 `style`），插进宿主 DOM 的
 * `<style>` 是全局生效的——插件可以隐藏/伪装宿主界面。插件自己的样式走
 * iframe 文档（entry_html）即可，那里是独立文档，`style-src 'unsafe-inline'`
 * 在壳 CSP 内被允许，跨文档不会外溢。
 *
 * 其余仍走默认白名单与默认属性消毒（`<script>` 等本就不在默认白名单内）；
 * 行内 `style="…"` 属性保留，slot 元素的正常排版不受影响。
 *
 * task-29 收口修复：默认 `KEEP_CONTENT: true` 会在移除 `<style>` 元素时**留下它的
 * 文本内容**，于是插件 CSS 会以纯文本形式插进宿主 DOM（虽不生效，但会被用户
 * 看见，且是宿主界面污染）。加 `FORBID_CONTENTS: ['style']`（DOMPurify 默认值
 * 为 `['annotation-xml']`）连内容一起丢掉，只禁用 style 的正文保留，其它元素的
 * 文本保留语义不变。
 */
export const PLUGIN_SLOT_SANITIZE_OPTIONS = Object.freeze({
  FORBID_TAGS: ['style'],
  FORBID_CONTENTS: ['style'],
})

const STYLE_BLOCK_RE = /<style\b[^>]*>[\s\S]*?<\/style\s*>/gi
const STYLE_OPEN_RE = /<style\b/i

/**
 * M-31a 预处理：把 `<style>…</style>` 整块（**含正文**）从 slot HTML 里去掉。
 *
 * 为什么必须在 sanitize 之前、且用文本级删除：DOMPurify 删除元素时走
 * `KEEP_CONTENT` 语义——元素被删掉、**正文被保留成裸文本**（dompurify 3.4.12 +
 * happy-dom 实测三种配置均如此：`<style>body{display:none}</style>` →
 * `body{display:none}`；`FORBID_CONTENTS: ['style']` 实测无效）。残留 CSS 文本会被
 * `v-html` 原样插进宿主 DOM，虽不生效但会以纯文本形式显示（界面污染）。
 * 也不用 DOMParser 做"解析→删元素→序列化"：该往返依赖 happy-dom/browser 的解析
 * 实现（实测在 happy-dom 下会把 `<button>` 一并丢掉），而本函数只需**删文本**，
 * 不可能引入新内容。
 *
 * 安全边界仍是随后那次 `DOMPurify.sanitize`（`FORBID_TAGS` 保留作纵深防御：
 * 未闭合/畸形形态漏网时也绝不产出可用的 `<style>` 元素）；只有原始 HTML 里真的
 * 出现 `<style` 时才做替换，其余输入与旧版字节一致。
 *
 * N-R4-02 补强：**未闭合**的 `<style>`（含 HTML 里等价于开标签的 `<style/>` 写法）
 * 其后内容按浏览器语义全部是 CSS 文本，因此从其位置**截断**——否则 DOMPurify 删掉
 * 元素后 CSS 正文会以裸文本留在宿主 DOM 中。
 */
export function stripStyleElementsFromSlotHtml(raw) {
  const html = typeof raw === 'string' ? raw : ''
  if (!/<style[\s>/]/i.test(html)) return html
  const withoutBlocks = html.replace(STYLE_BLOCK_RE, '')
  const dangling = withoutBlocks.search(STYLE_OPEN_RE)
  return dangling >= 0 ? withoutBlocks.slice(0, dangling) : withoutBlocks
}

/**
 * 宿主向插件 iframe 投递消息的目标 origin。插件文档统一由 `registerShellDoc`
 * 发布在隔离的 storyforge-shell 源（唯一例外是非 Tauri 的 blob: 兜底，其源为
 * 不透明不可知），因此只在 frameSrc 确实是壳源时收紧，其余情况维持 '*'。
 */
export function pluginIframeTargetOrigin(frameSrc, shellOrigin) {
  const src = typeof frameSrc === 'string' ? frameSrc : ''
  const origin = typeof shellOrigin === 'string' ? shellOrigin : ''
  if (!src || !origin) return '*'
  return src.startsWith(`${origin}/`) ? origin : '*'
}

export function waitForPluginHostReady(isReady, subscribe, timeoutMs = PLUGIN_HOST_HOOK_READY_TIMEOUT_MS) {
  if (isReady()) return Promise.resolve(true)

  return new Promise((resolve) => {
    let settled = false
    let unsubscribe = () => {}

    const finish = (ready) => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      unsubscribe()
      resolve(ready)
    }

    const timer = setTimeout(() => finish(false), timeoutMs)
    unsubscribe = subscribe((ready) => finish(typeof ready === 'boolean' ? ready : !!isReady()))
  })
}
</script>

<script setup>
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import DOMPurify from 'dompurify'
import {
  generateBridgeScript,
  createHostHandler,
  createPluginHookBridge,
  MSG_MOUNT,
  mapPluginEventRecordToPluginEvents,
  postPluginEventToTarget,
} from '../plugin-bridge.js'
import { invoke } from '@tauri-apps/api/core'
import { buildShellCspMetaTag } from '../utils/cardShellCsp.js'
import {
  configureShellDocInvoke,
  registerShellDoc,
  releaseShellDoc,
  SHELL_DOC_ORIGIN,
} from '../utils/shellDocUrl.js'

const props = defineProps({
  /** InstalledPluginDto */
  plugin: { type: Object, required: true },
  /** 紧凑模式（用于 slot 内嵌） */
  compact: { type: Boolean, default: false },
  /** 固定高度（默认自适应） */
  height: { type: String, default: '200px' },
  /** App.vue 广播的流水线/宿主事件 feed */
  pluginEvents: { type: Array, default: () => [] },
})

const emit = defineEmits(['slot-mount', 'ready', 'error'])

const iframeRef = ref(null)
const slotHtmlBySlot = ref({})
const slotEntries = computed(() => getPluginSlotEntries(slotHtmlBySlot.value))
const iframeReady = ref(false)
let handler = null
let hookBridge = null
let lastPluginEventId = 0
const pendingPluginEvents = []
const pendingReadyResolvers = []
const MAX_PENDING_PLUGIN_EVENTS = 100
const HOST_ORIGIN = window.location?.origin || '*'

// M-24：每份插件文档一枚握手令牌。`handshakeValid` 只有在收到「当前文档」
// 回传的令牌后才为真——帧自我导航后新文档没有该令牌，宿主不再把它当可信帧
// （事件/hook/API 一律停发），直到它再次完成握手。
//
// 顺序不变量（task-29 修正，原注释的错误前提见下）：**令牌必须先于文档组装
// 生成，且令牌不能是 watch 源的响应式依赖**。旧实现把令牌嵌进 computed、由
// `watch(computed)` 在回调里换令牌，导致两个缺陷：
//   1. 回调用的是「先求值好的文档参数」（含上一轮令牌，首次是空串），换令牌后
//      从不重读文档 ⇒ 注册给帧的文档永远带 T_{n-1}，桥脚本发 T_{n-1} 而宿主
//      期望 T_n ⇒ **握手永不成功、插件桥整体失效**；
//   2. 换令牌使 computed 变脏 → watch 再次触发 ⇒ 自激重注册循环。
// 现在：watch 只依赖 `handshakeSourceKey`（插件身份 + entry_html，与令牌无关），
// 回调里**先生成令牌、再组装文档、再注册**，每次源变化恰好注册一次。
let handshakeToken = ''
let handshakeDocumentSequence = 0
// M-24（task-29 收口修复）：标记「下一次 load 是否由宿主自己注册文档引发」。
// 只有那一次可以保留当前令牌（桥脚本正带着它完成握手）；任何**额外** load
// 都是帧自我导航/文档被替换，必须轮换令牌，否则旧文档的令牌可被重放重新取得
// 信任（原实现只重置 handshakeValid，旧令牌仍能通过校验）。
let expectedFrameLoad = false
const handshakeValid = ref(false)

function newPluginHandshakeToken() {
  handshakeDocumentSequence += 1
  handshakeToken = createPluginBridgeHandshakeToken()
  handshakeValid.value = false
  return handshakeToken
}

// V5 CSP isolation: the plugin document is served on the isolated
// storyforge-shell origin so its inline bridge does not inherit the main app
// CSP. composeShellDoc holds the full HTML (with its own shell CSP meta,
// mirroring the HTTP header set by shell_doc_protocol.rs); iframeSrc holds the
// resolved protocol URL and is what the iframe loads via :src.
//
// 纯函数：令牌由调用方传入（见 newPluginHandshakeToken / handshakeSourceKey 的
// 顺序不变量），因此组装结果与令牌一一对应，不存在「注册的文档 = 旧令牌」。
function composeShellDoc(plugin, token) {
  const rawHtml = plugin?.manifest?.entry_html || plugin?.entry_html || ''
  if (!rawHtml) return ''
  // ADD_TAGS:['script'] 是刻意的：entry_html 的 <script> 就是插件本体（事件
  // 订阅 / prompt hook 都靠它注册），默认 sanitize 会整体剥除（dompurify
  // 3.4.12 真 Chromium 实测），插件桥形同虚设。安全边界不在这里——本文档整体
  // 跑在 sandbox="allow-scripts" 的 opaque-origin iframe 里，对宿主的唯一通道
  // 是经权限门控的桥消息（isTrustedPluginSource + createHostHandler 的插件
  // 权限校验）；DOMPurify 只负责收掉 script 之外的注入面（事件处理器属性、
  // javascript: URL 等在真 Chromium 实测中仍被剥除）。
  const entryHtml = DOMPurify.sanitize(rawHtml, { ADD_TAGS: ['script'] })
  const bridgeScript = generateBridgeScript(plugin.id, HOST_ORIGIN, token)
  return `<!DOCTYPE html><html><head>${buildShellCspMetaTag([])}${bridgeScript}</head><body>${entryHtml}</body></html>`
}

// watch 源：只依赖插件身份与 entry_html，**不读令牌**（令牌在回调内生成），
// 因此换令牌不会反过来触发注册。
const handshakeSourceKey = computed(() => {
  const plugin = props.plugin
  const rawHtml = plugin?.manifest?.entry_html || plugin?.entry_html || ''
  return rawHtml ? `${plugin?.id || ''}\u0000${rawHtml}` : ''
})
const iframeSrc = ref('')
let iframeUrl = null

// V5 CSP isolation: the plugin document is always served from the isolated
// storyforge-shell origin, so host → plugin delivery can pin that origin
// instead of '*'。只在 frameSrc 确实落在壳源时才收紧，blob: 兜底维持 '*'。
const pluginTargetOrigin = computed(() =>
  pluginIframeTargetOrigin(iframeSrc.value, SHELL_DOC_ORIGIN),
)

// Re-register the document whenever the plugin's HTML (or its identity) changes.
watch(handshakeSourceKey, async (key, _previousKey, onCleanup) => {
  let cancelled = false
  onCleanup(() => { cancelled = true })

  // M-24 顺序不变量：**先生成令牌，再用它组装文档**，最后注册这份文档。
  // 因此注册给帧的文档里的令牌 == 随后 `sf:ready` 必须回传的令牌（旧实现在
  // 换令牌前就用已求值的文档，导致文档永远带上一轮令牌 ⇒ 握手永不成功）。
  const token = newPluginHandshakeToken()
  const doc = key ? composeShellDoc(props.plugin, token) : ''

  const previousUrl = iframeUrl
  iframeUrl = null
  if (previousUrl?.startsWith('blob:')) {
    try { URL.revokeObjectURL(previousUrl) } catch (_) {}
  } else if (previousUrl) {
    void releaseShellDoc(previousUrl).catch(() => {})
  }

  if (!doc) {
    iframeSrc.value = ''
    expectedFrameLoad = false
    return
  }
  configureShellDocInvoke(invoke)
  try {
    const nextUrl = await registerShellDoc(doc)
    if (cancelled) {
      void releaseShellDoc(nextUrl).catch(() => {})
      return
    }
    iframeUrl = nextUrl
    iframeSrc.value = nextUrl
    expectedFrameLoad = true
  } catch (e) {
    if (typeof window !== 'undefined' && window.__TAURI_INTERNALS__) throw e
    const blob = new Blob([doc], { type: 'text/html' })
    iframeUrl = URL.createObjectURL(blob)
    iframeSrc.value = iframeUrl
    expectedFrameLoad = true
  }
  // The plugin identity changed; reset ready state until the new bridge boots.
  iframeReady.value = false
}, { immediate: true })

const iframeStyle = computed(() => ({
  width: '100%',
  height: props.height,
  border: 'none',
  borderRadius: '8px',
}))

// M-24：@load 在「帧自我导航」后同样会触发（WindowProxy 身份不变）。这里不再
// 把 load 当作就绪——就绪只由当前文档的握手令牌确认。**并且**：只有宿主自己
// 注册文档引发的那次 load 可以保留令牌；任何额外 load 都轮换令牌，使旧文档的
// 令牌立即作废 ⇒ 自导航后的新文档拿不到新令牌，宿主永不再信任该 WindowProxy。
function onIframeLoad() {
  iframeReady.value = false
  handshakeValid.value = false
  if (expectedFrameLoad) {
    expectedFrameLoad = false
    return
  }
  newPluginHandshakeToken()
}

function resolvePendingReadyWaiters(ready) {
  while (pendingReadyResolvers.length > 0) {
    const resolve = pendingReadyResolvers.shift()
    resolve(ready)
  }
}

function subscribeIframeReady(resolve) {
  pendingReadyResolvers.push(resolve)
  return () => {
    const index = pendingReadyResolvers.indexOf(resolve)
    if (index >= 0) {
      pendingReadyResolvers.splice(index, 1)
    }
  }
}

function queuePluginEvent(pluginEvent) {
  pendingPluginEvents.push(pluginEvent)
  if (pendingPluginEvents.length > MAX_PENDING_PLUGIN_EVENTS) {
    pendingPluginEvents.shift()
  }
}

function postPluginEvent(pluginEvent) {
  const target = iframeRef.value?.contentWindow
  if (!iframeReady.value || !target) {
    queuePluginEvent(pluginEvent)
    return
  }

  // 广播前统一解克隆：事件 feed 常携带 Vue reactive 代理，直接 postMessage
  // 会抛 DataCloneError（rerun-1a 验收实录）。
  postPluginEventToTarget(target, pluginEvent, pluginTargetOrigin.value)
}

function flushPendingPluginEvents() {
  const target = iframeRef.value?.contentWindow
  if (!iframeReady.value || !target) return

  while (pendingPluginEvents.length > 0) {
    postPluginEventToTarget(target, pendingPluginEvents.shift(), pluginTargetOrigin.value)
  }
}

function dispatchPluginEventRecord(record) {
  for (const pluginEvent of mapPluginEventRecordToPluginEvents(record, props.plugin)) {
    postPluginEvent(pluginEvent)
  }
}

function consumePluginEvents(events) {
  for (const record of events || []) {
    const eventId = Number(record?.id || 0)
    if (eventId > 0 && eventId <= lastPluginEventId) continue

    dispatchPluginEventRecord(record)
    if (eventId > lastPluginEventId) {
      lastPluginEventId = eventId
    }
  }
}

// 帧归属（WindowProxy 身份）+ 当前文档握手双条件。仅靠身份挡不住自导航：
// sandbox 不禁止 `location.href=…`，导航后 WindowProxy 仍是同一个。
function isTrustedPluginSource(event) {
  if (!iframeRef.value?.contentWindow || event.source !== iframeRef.value.contentWindow) {
    return false
  }
  return handshakeValid.value
}

async function emitPluginEventAndWait(event, data = {}) {
  if (!hookBridge) return data

  const ready = await waitForPluginHostReady(
    () => iframeReady.value && handshakeValid.value && !!iframeRef.value?.contentWindow,
    subscribeIframeReady,
  )
  if (!ready) return data
  if (!hookBridge) return data

  return await hookBridge.emitAndWait(event, data)
}

// Handle messages from the plugin iframe.
function onWindowMessage(event) {
  const data = event.data

  // M-24：握手消息本身是「不可信帧 → 可信帧」的唯一入口，故先于可信校验处理，
  // 但仍要求窗口归属正确 + 令牌与该文档一致。握手完成后本帧才放行其余消息。
  if (
    data?.type === 'sf:ready'
    && data.pluginId === props.plugin.id
    && !!iframeRef.value?.contentWindow
    && event.source === iframeRef.value.contentWindow
  ) {
    if (!isPluginBridgeHandshakeValid(handshakeToken, data.handshake)) return
    handshakeValid.value = true
    iframeReady.value = true
    flushPendingPluginEvents()
    resolvePendingReadyWaiters(true)
    emit('ready', props.plugin.id)
    return
  }

  if (!isTrustedPluginSource(event)) return

  // 插件 UI 挂载请求
  if (data?.type === MSG_MOUNT && data.pluginId === props.plugin.id) {
    // 消毒插件 HTML，防止 XSS 注入宿主 DOM；M-31a：额外禁止 <style>，
    // 否则插件可向宿主文档注入全局 CSS（界面伪装/隐藏）。
    const html = DOMPurify.sanitize(
      stripStyleElementsFromSlotHtml(data.html || ''),
      PLUGIN_SLOT_SANITIZE_OPTIONS,
    )
    const slot = normalizePluginHostSlot(data.slot)
    slotHtmlBySlot.value = applyPluginSlotMount(slotHtmlBySlot.value, {
      slot,
      html,
    })
    emit('slot-mount', { pluginId: data.pluginId, slot, html })
  }

  if (hookBridge?.handleMessage(event)) {
    return
  }

  // API 请求由 handler 处理
  if (handler) {
    handler(event)
  }
}

onMounted(() => {
  handler = createHostHandler(props.plugin, invoke, { isTrustedSource: isTrustedPluginSource })
  // Outer promptHooks.js owns the timeout budget and timeout audit status.
  // Disabling the bridge timer prevents double 5s timeouts that would settle
  // as ok with the fallback payload before the outer runtime can classify
  // status=timeout.
  hookBridge = createPluginHookBridge(props.plugin, {
    getTarget: () => iframeRef.value?.contentWindow,
    isTrustedSource: isTrustedPluginSource,
    targetOrigin: pluginTargetOrigin.value,
    timeoutMs: null,
    onError: (error) => {
      emit('error', { pluginId: props.plugin.id, error: String(error?.message || error) })
    },
  })
  window.addEventListener('message', onWindowMessage)
})

onUnmounted(() => {
  window.removeEventListener('message', onWindowMessage)
  resolvePendingReadyWaiters(false)
  hookBridge?.dispose()
  handler = null
  hookBridge = null
  if (iframeUrl?.startsWith('blob:')) {
    try { URL.revokeObjectURL(iframeUrl) } catch (_) {}
  } else if (iframeUrl) {
    void releaseShellDoc(iframeUrl).catch(() => {})
  }
  iframeUrl = null
})

// 插件变化时重建 handler
watch(() => props.plugin, (newPlugin) => {
  if (newPlugin) {
    handler = createHostHandler(newPlugin, invoke, { isTrustedSource: isTrustedPluginSource })
    hookBridge?.dispose()
    hookBridge = createPluginHookBridge(newPlugin, {
      getTarget: () => iframeRef.value?.contentWindow,
      isTrustedSource: isTrustedPluginSource,
      targetOrigin: pluginTargetOrigin.value,
      timeoutMs: null,
      onError: (error) => {
        emit('error', { pluginId: newPlugin.id, error: String(error?.message || error) })
      },
    })
  }
})

watch(() => props.pluginEvents, consumePluginEvents, { immediate: true })

defineExpose({
  emitPluginEventAndWait,
})
</script>

<style scoped>
.plugin-host {
  position: relative;
}

.plugin-host-compact .plugin-iframe {
  height: 60px !important;
}

.plugin-slot-content {
  overflow: hidden;
}
</style>
