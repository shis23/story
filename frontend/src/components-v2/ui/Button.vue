<script setup>
const props = defineProps({
  variant: { type: String, default: 'default' }, // default | primary | danger | ghost
  size: { type: String, default: 'md' }, // sm | md | lg
  loading: { type: Boolean, default: false },
  disabled: { type: Boolean, default: false },
  type: { type: String, default: 'button' }, // button | submit | reset
})
defineEmits(['click'])

const variantClass = {
  default: 'bg-surface text-ink border border-line shadow-card hover:border-accent-border hover:text-accent-bright',
  primary: 'bg-accent text-white font-medium hover:bg-accent-bright',
  danger: 'text-err border border-err/40 hover:bg-err/10',
  ghost: 'text-ink-soft hover:text-ink hover:bg-surface-2',
}
const sizeClass = {
  sm: 'px-2.5 py-1 text-xs rounded-md',
  md: 'px-3.5 py-1.5 text-sm rounded-md',
  lg: 'px-5 py-2.5 text-base rounded-lg',
}
function classes() {
  return [
    'inline-flex items-center justify-center gap-1.5 transition-colors duration-150 select-none',
    'disabled:opacity-40 disabled:cursor-not-allowed',
    variantClass[props.variant] || variantClass.default,
    sizeClass[props.size] || sizeClass.md,
  ]
}
</script>

<template>
  <button
    :type="type"
    :class="classes()"
    :disabled="disabled || loading"
    @click="$emit('click', $event)"
  >
    <span v-if="loading" class="inline-block animate-spin opacity-70" aria-hidden="true">
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><path d="M12 3a9 9 0 1 0 9 9"/></svg>
    </span>
    <slot v-else />
  </button>
</template>
