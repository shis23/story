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
    <div v-if="slotHtml" v-html="slotHtml" class="plugin-slot-content" />
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import DOMPurify from 'dompurify'
import { generateBridgeScript, createHostHandler, MSG_MOUNT } from '../plugin-bridge.js'
import { invoke } from '@tauri-apps/api/core'

const props = defineProps({
  /** InstalledPluginDto */
  plugin: { type: Object, required: true },
  /** 紧凑模式（用于 slot 内嵌） */
  compact: { type: Boolean, default: false },
  /** 固定高度（默认自适应） */
  height: { type: String, default: '200px' },
})

const emit = defineEmits(['slot-mount', 'ready', 'error'])

const iframeRef = ref(null)
const slotHtml = ref('')
let handler = null

// 构建 srcdoc：bridge script + 插件 HTML（entry_html 已由 iframe sandbox 隔离，
// 但仍做消毒防止沙箱逃逸场景）
const iframeSrc = computed(() => {
  if (!props.plugin?.manifest?.entry_html && !props.plugin?.entry_html) return ''
  const rawHtml = props.plugin?.manifest?.entry_html || props.plugin?.entry_html || ''
  const entryHtml = DOMPurify.sanitize(rawHtml)
  const bridgeScript = generateBridgeScript(props.plugin.id)
  return `<!DOCTYPE html><html><head>${bridgeScript}</head><body>${entryHtml}</body></html>`
})

const iframeStyle = computed(() => ({
  width: '100%',
  height: props.height,
  border: 'none',
  borderRadius: '8px',
}))

function onIframeLoad() {
  emit('ready', props.plugin.id)
}

// 处理来自 iframe 的消息
function onWindowMessage(event) {
  const data = event.data

  // 插件 UI 挂载请求
  if (data?.type === MSG_MOUNT && data.pluginId === props.plugin.id) {
    // 消毒插件 HTML，防止 XSS 注入宿主 DOM
    slotHtml.value = DOMPurify.sanitize(data.html || '')
    emit('slot-mount', { pluginId: data.pluginId, slot: data.slot, html: data.html })
  }

  // API 请求由 handler 处理
  if (handler) {
    handler(event)
  }
}

onMounted(() => {
  handler = createHostHandler(props.plugin, invoke)
  window.addEventListener('message', onWindowMessage)
})

onUnmounted(() => {
  window.removeEventListener('message', onWindowMessage)
  handler = null
})

// 插件变化时重建 handler
watch(() => props.plugin, (newPlugin) => {
  if (newPlugin) {
    handler = createHostHandler(newPlugin, invoke)
  }
}, { deep: true })
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
