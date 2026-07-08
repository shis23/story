<script setup>
const props = defineProps({
  variant: { type: String, default: 'neutral' }, // ok | warn | err | accent | neutral
  size: { type: String, default: 'sm' }, // sm | md
})

const variantClass = {
  ok: 'bg-ok/15 text-ok',
  warn: 'bg-warn/15 text-warn',
  err: 'bg-err/15 text-err',
  accent: 'bg-accent-soft text-accent-bright',
  neutral: 'bg-surface-2 text-ink-soft',
}

const sizeClass = {
  sm: 'px-1.5 py-0.5 text-xs',
  md: 'px-2 py-0.5 text-sm',
}
</script>

<template>
  <span
    :class="[
      'inline-flex items-center rounded-md font-medium',
      variantClass[props.variant] || variantClass.neutral,
      sizeClass[props.size] || sizeClass.sm,
    ]"
  >
    <slot />
  </span>
</template>
