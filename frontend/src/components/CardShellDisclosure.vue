<script setup>
import { ref } from 'vue'

const props = defineProps({
  label: { type: String, default: '当前状态' },
  initiallyExpanded: { type: Boolean, default: false },
})

const expanded = ref(props.initiallyExpanded)
</script>

<template>
  <section class="my-5 overflow-hidden rounded-lg border border-line bg-surface-2/35">
    <button
      type="button"
      class="flex w-full items-center gap-3 px-4 py-2.5 text-left transition-colors hover:bg-surface-2/70"
      :aria-expanded="expanded"
      @click="expanded = !expanded"
    >
      <span class="h-1.5 w-1.5 rounded-full bg-accent"></span>
      <span class="flex-1 text-xs font-medium tracking-[0.08em] text-ink-soft">{{ label }}</span>
      <span class="text-[11px] text-ink-faint">{{ expanded ? '收起' : '展开' }}</span>
      <span class="text-ink-faint transition-transform" :class="expanded ? 'rotate-180' : ''" aria-hidden="true">⌄</span>
    </button>
    <div v-if="expanded" class="border-t border-line bg-bg/30 px-3 py-3">
      <slot />
    </div>
  </section>
</template>
