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
  <div class="relative rounded-lg bg-surface-2 border border-line overflow-hidden">
    <div
      v-if="language || $slots.toolbar"
      class="flex items-center justify-between px-3 py-1.5 border-b border-line"
    >
      <span class="text-xs text-ink-faint font-mono">{{ language }}</span>
      <slot name="toolbar">
        <button
          type="button"
          class="inline-flex items-center gap-1 text-xs text-ink-soft hover:text-ink transition-colors"
          @click="copy"
        >
          <svg v-if="!copied" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="9" y="9" width="12" height="12" rx="2"/><path d="M5 15V5a2 2 0 0 1 2-2h10"/></svg>
          <svg v-else width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="text-ok" aria-hidden="true"><path d="M4 12.5l5 5L20 6.5"/></svg>
          {{ copied ? '已复制' : '复制' }}
        </button>
      </slot>
    </div>
    <button
      v-else
      type="button"
      class="absolute top-1.5 right-1.5 inline-flex items-center gap-1 text-xs text-ink-faint hover:text-ink transition-colors"
      @click="copy"
    >
      <svg v-if="!copied" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="9" y="9" width="12" height="12" rx="2"/><path d="M5 15V5a2 2 0 0 1 2-2h10"/></svg>
      <svg v-else width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="text-ok" aria-hidden="true"><path d="M4 12.5l5 5L20 6.5"/></svg>
      {{ copied ? '已复制' : '复制' }}
    </button>
    <pre
      class="p-3 text-xs font-mono text-ink-soft overflow-auto"
      :class="wrap ? 'whitespace-pre-wrap break-words' : 'whitespace-pre'"
    ><code>{{ code }}</code></pre>
  </div>
</template>
