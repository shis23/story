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
  default: 'bg-surface-2 text-ink border border-line hover:border-accent-border',
  primary: 'bg-accent text-bg shadow-glow-accent hover:opacity-90',
  danger: 'text-err border border-err/40 hover:bg-err/15',
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
    <span v-if="loading" class="inline-block animate-spin opacity-70">◌</span>
    <slot v-else />
  </button>
</template>
