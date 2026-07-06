<script setup>
import { computed } from 'vue'
import DOMPurify from 'dompurify'
import { formatContent, shouldRenderHtmlDisplay } from '../utils/formatContent.js'

const props = defineProps({
  content: { type: String, default: '' },
  sourceContent: { type: String, default: '' },
})

const SANITIZE_CONFIG = Object.freeze({
  USE_PROFILES: { html: true },
  ALLOW_ARIA_ATTR: true,
  ALLOW_DATA_ATTR: true,
  FORBID_TAGS: [
    'script',
    'iframe',
    'object',
    'embed',
    'link',
    'meta',
    'base',
    'form',
    'input',
    'button',
    'textarea',
    'select',
    'option',
  ],
  FORBID_ATTR: [
    'onabort',
    'onblur',
    'onchange',
    'onclick',
    'ondblclick',
    'onerror',
    'onfocus',
    'oninput',
    'onkeydown',
    'onkeypress',
    'onkeyup',
    'onload',
    'onmousedown',
    'onmouseenter',
    'onmouseleave',
    'onmousemove',
    'onmouseout',
    'onmouseover',
    'onmouseup',
    'onsubmit',
  ],
})

const renderAsHtml = computed(() => shouldRenderHtmlDisplay(props.content, props.sourceContent))
const formattedText = computed(() => formatContent(props.content))
const sanitizedHtml = computed(() => DOMPurify.sanitize(props.content || '', SANITIZE_CONFIG))
</script>

<template>
  <div v-if="renderAsHtml" class="rich-html-fragment" v-html="sanitizedHtml"></div>
  <span v-else v-html="formattedText"></span>
</template>

<style scoped>
.rich-html-fragment {
  max-width: 100%;
  overflow-x: auto;
}

.rich-html-fragment :deep(*) {
  max-width: 100%;
}
</style>
