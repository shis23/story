<script setup>
const props = defineProps({
  variant: { type: String, default: 'ghost' }, // default | primary | danger | ghost
  size: { type: String, default: 'md' }, // sm | md | lg
  disabled: { type: Boolean, default: false },
  loading: { type: Boolean, default: false },
  title: { type: String, default: '' },
})

defineEmits(['click'])

const variantClass = {
  default: 'bg-surface text-ink border border-line shadow-card hover:border-accent-border hover:text-accent-bright',
  primary: 'bg-accent text-white hover:bg-accent-bright',
  danger: 'text-err border border-err/40 hover:bg-err/10',
  ghost: 'text-ink-soft hover:text-ink hover:bg-surface-2',
}

const sizeClass = {
  sm: 'w-6 h-6',
  md: 'w-8 h-8',
  lg: 'w-10 h-10',
}
</script>

<template>
  <button
    :type="'button'"
    :title="title"
    :class="[
      'inline-flex items-center justify-center rounded-md transition-colors duration-150 select-none',
      'disabled:opacity-40 disabled:cursor-not-allowed',
      variantClass[props.variant] || variantClass.ghost,
      sizeClass[props.size] || sizeClass.md,
    ]"
    :disabled="disabled || loading"
    @click="$emit('click', $event)"
  >
    <span v-if="loading" class="inline-block animate-spin opacity-70" aria-hidden="true">
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><path d="M12 3a9 9 0 1 0 9 9"/></svg>
    </span>
    <slot v-else />
  </button>
</template>
