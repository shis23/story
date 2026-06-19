<script setup>
import { computed } from 'vue'

const props = defineProps({
  variant: { type: String, default: 'ghost' }, // primary | ghost | danger | subtle
  size: { type: String, default: 'md' },       // sm | md
  disabled: { type: Boolean, default: false },
  type: { type: String, default: 'button' },
})

const variantClass = computed(() => ({
  primary: 'bg-accent text-white hover:opacity-90 border border-accent',
  ghost: 'text-ink-soft hover:bg-accent-soft border border-transparent',
  danger: 'bg-err/10 text-err hover:bg-err/20 border border-err/30',
  subtle: 'bg-bg text-ink-soft hover:bg-line/40 border border-line',
}[props.variant] || ''))

const sizeClass = computed(() => ({
  // md ≥44px 触控；sm 用于工具栏但仍 ≥36px
  sm: 'min-h-[36px] px-3 text-xs',
  md: 'min-h-[44px] px-4 text-sm',
}[props.size] || 'min-h-[44px] px-4 text-sm'))
</script>

<template>
  <button
    :type="type"
    :disabled="disabled"
    class="inline-flex items-center justify-center gap-1.5 rounded-lg font-medium transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
    :class="[variantClass, sizeClass]"
  >
    <slot />
  </button>
</template>
