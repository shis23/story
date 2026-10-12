<script setup>
import { computed } from 'vue'
import DOMPurify from 'dompurify'
import {
  formatContent,
  shouldRenderHtmlDisplay,
  splitRawHtmlBlocks,
} from '../../utils/formatContent.js'

const props = defineProps({
  content: { type: String, default: '' },
  sourceContent: { type: String, default: '' },
})

const SANITIZE_CONFIG = Object.freeze({
  USE_PROFILES: { html: true },
  ALLOW_ARIA_ATTR: true,
  ALLOW_DATA_ATTR: true,
  FORBID_TAGS: [
    'script', 'iframe', 'object', 'embed', 'link', 'meta', 'base',
    'form', 'input', 'button', 'textarea', 'select', 'option',
  ],
  FORBID_ATTR: [
    'onabort', 'onblur', 'onchange', 'onclick', 'ondblclick', 'onerror',
    'onfocus', 'oninput', 'onkeydown', 'onkeypress', 'onkeyup', 'onload',
    'onmousedown', 'onmouseenter', 'onmouseleave', 'onmousemove', 'onmouseout',
    'onmouseover', 'onmouseup', 'onsubmit',
  ],
})

function sanitize(html) {
  return DOMPurify.sanitize(html || '', SANITIZE_CONFIG)
}

const renderAsHtml = computed(() => shouldRenderHtmlDisplay(props.content, props.sourceContent))
const formattedText = computed(() => formatContent(props.content))
const sanitizedHtml = computed(() => sanitize(props.content))
// 原文里卡自带的完整 HTML 块（状态栏等）就地渲染，其余照旧走转义文本。
const rawSegments = computed(() => {
  if (renderAsHtml.value) return []
  const segments = splitRawHtmlBlocks(props.content)
  if (!segments.some((seg) => seg.type === 'html')) return []
  return segments.map((seg) => (seg.type === 'html'
    ? { type: 'html', html: sanitize(seg.content) }
    : { type: 'text', html: formatContent(seg.content) }))
})
</script>

<template>
  <div v-if="renderAsHtml" class="rich-html-fragment" v-html="sanitizedHtml"></div>
  <div v-else-if="rawSegments.length" class="rich-mixed">
    <template v-for="(seg, index) in rawSegments" :key="index">
      <div
        v-if="seg.type === 'html'"
        class="rich-html-fragment rich-raw-html"
        data-testid="rich-raw-html"
        v-html="seg.html"
      ></div>
      <div v-else v-html="seg.html"></div>
    </template>
  </div>
  <span v-else v-html="formattedText"></span>
</template>

<style scoped>
.rich-html-fragment { max-width: 100%; overflow-x: auto; }
.rich-html-fragment :deep(*) { max-width: 100%; }
/* 模型写出的 HTML：fixed/absolute 定位不得盖出消息框 */
.rich-raw-html { contain: paint; margin: 0.5rem 0; }
</style>
