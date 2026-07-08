<script setup>
import { ref } from 'vue'

const props = defineProps({
  code: { type: String, default: '' },
  language: { type: String, default: '' }, // 语言标签(仅显示用,不做语法高亮)
  wrap: { type: Boolean, default: false },
})
const copied = ref(false)
async function copy() {
  try {
    await navigator.clipboard.writeText(props.code)
    copied.value = true
    setTimeout(() => (copied.value = false), 1500)
  } catch {
    // clipboard 不可用时静默
  }
}
</script>

<template>
  <div class="relative rounded-lg bg-bg border border-line overflow-hidden">
    <div
      v-if="language || $slots.toolbar"
      class="flex items-center justify-between px-3 py-1 border-b border-line bg-surface-2"
    >
      <span class="text-xs text-ink-faint font-mono">{{ language }}</span>
      <slot name="toolbar">
        <button
          type="button"
          class="text-xs text-ink-soft hover:text-accent transition-colors"
          @click="copy"
        >
          {{ copied ? '已复制' : '复制' }}
        </button>
      </slot>
    </div>
    <button
      v-else
      type="button"
      class="absolute top-1.5 right-1.5 text-xs text-ink-faint hover:text-accent transition-colors"
      @click="copy"
    >
      {{ copied ? '已复制' : '复制' }}
    </button>
    <pre
      class="p-3 text-xs font-mono text-ink-soft overflow-auto"
      :class="wrap ? 'whitespace-pre-wrap break-words' : 'whitespace-pre'"
    ><code>{{ code }}</code></pre>
  </div>
</template>
