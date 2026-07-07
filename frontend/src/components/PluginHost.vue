<template>
  <div class="plugin-host" :class="compact ? 'plugin-host-compact' : ''">
    <iframe
      v-if="iframeSrc"
      ref="iframeRef"
      :srcdoc="iframeSrc"
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
</script>

<script setup>
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import DOMPurify from 'dompurify'
import {
  generateBridgeScript,
  createHostHandler,
  createPluginHookBridge,
  MSG_EVENT,
  MSG_MOUNT,
  mapPluginEventRecordToPluginEvents,
} from '../plugin-bridge.js'
import { invoke } from '@tauri-apps/api/core'

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
const MAX_PENDING_PLUGIN_EVENTS = 100
const HOST_ORIGIN = window.location?.origin || '*'
// Sandboxed srcdoc iframes have an opaque origin, so host-to-plugin delivery
// must use '*'. Source checks below keep inbound messages scoped to this iframe.
const PLUGIN_IFRAME_TARGET_ORIGIN = '*'

// 构建 srcdoc：bridge script + 插件 HTML（entry_html 已由 iframe sandbox 隔离，
// 但仍做消毒防止沙箱逃逸场景）
const iframeSrc = computed(() => {
  if (!props.plugin?.manifest?.entry_html && !props.plugin?.entry_html) return ''
  const rawHtml = props.plugin?.manifest?.entry_html || props.plugin?.entry_html || ''
  const entryHtml = DOMPurify.sanitize(rawHtml)
  const bridgeScript = generateBridgeScript(props.plugin.id, HOST_ORIGIN)
  return `<!DOCTYPE html><html><head>${bridgeScript}</head><body>${entryHtml}</body></html>`
})

const iframeStyle = computed(() => ({
  width: '100%',
  height: props.height,
  border: 'none',
  borderRadius: '8px',
}))

function onIframeLoad() {
  iframeReady.value = true
  flushPendingPluginEvents()
  emit('ready', props.plugin.id)
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

  target.postMessage({
    type: MSG_EVENT,
    event: pluginEvent.event,
    data: pluginEvent.data,
  }, PLUGIN_IFRAME_TARGET_ORIGIN)
}

function flushPendingPluginEvents() {
  const target = iframeRef.value?.contentWindow
  if (!iframeReady.value || !target) return

  while (pendingPluginEvents.length > 0) {
    const pluginEvent = pendingPluginEvents.shift()
    target.postMessage({
      type: MSG_EVENT,
      event: pluginEvent.event,
      data: pluginEvent.data,
    }, PLUGIN_IFRAME_TARGET_ORIGIN)
  }
}

function dispatchPluginEventRecord(record) {
  for (const pluginEvent of mapPluginEventRecordToPluginEvents(record)) {
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

function isTrustedPluginSource(event) {
  return !!iframeRef.value?.contentWindow && event.source === iframeRef.value.contentWindow
}

async function emitPluginEventAndWait(event, data = {}) {
  if (!hookBridge || !iframeReady.value) return data
  return await hookBridge.emitAndWait(event, data)
}

// 处理来自 iframe 的消息
function onWindowMessage(event) {
  if (!isTrustedPluginSource(event)) return

  const data = event.data

  // 插件 UI 挂载请求
  if (data?.type === MSG_MOUNT && data.pluginId === props.plugin.id) {
    // 消毒插件 HTML，防止 XSS 注入宿主 DOM
    const html = DOMPurify.sanitize(data.html || '')
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
  hookBridge = createPluginHookBridge(props.plugin, {
    getTarget: () => iframeRef.value?.contentWindow,
    isTrustedSource: isTrustedPluginSource,
    targetOrigin: PLUGIN_IFRAME_TARGET_ORIGIN,
    onError: (error) => {
      emit('error', { pluginId: props.plugin.id, error: String(error?.message || error) })
    },
  })
  window.addEventListener('message', onWindowMessage)
})

onUnmounted(() => {
  window.removeEventListener('message', onWindowMessage)
  hookBridge?.dispose()
  handler = null
  hookBridge = null
})

// 插件变化时重建 handler
watch(() => props.plugin, (newPlugin) => {
  if (newPlugin) {
    handler = createHostHandler(newPlugin, invoke, { isTrustedSource: isTrustedPluginSource })
    hookBridge?.dispose()
    hookBridge = createPluginHookBridge(newPlugin, {
      getTarget: () => iframeRef.value?.contentWindow,
      isTrustedSource: isTrustedPluginSource,
      targetOrigin: PLUGIN_IFRAME_TARGET_ORIGIN,
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
