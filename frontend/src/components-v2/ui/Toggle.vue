<script setup>
const props = defineProps({
  modelValue: { type: Boolean, default: false },
  disabled: { type: Boolean, default: false },
})
const emit = defineEmits(['update:modelValue'])

function toggle() {
  if (props.disabled) return
  emit('update:modelValue', !props.modelValue)
}
</script>

<template>
  <button
    type="button"
    role="switch"
    :aria-checked="modelValue"
    :disabled="disabled"
    :class="[
      'relative inline-flex w-10 h-5 rounded-full border transition-colors duration-150 ease-soft',
      modelValue ? 'bg-accent border-accent' : 'bg-surface-2 border-line',
      disabled ? 'opacity-40 cursor-not-allowed' : 'cursor-pointer',
      !disabled && !modelValue ? 'hover:border-accent-border' : '',
    ]"
    @click="toggle"
  >
    <span
      :class="[
        'absolute top-0.5 left-0.5 w-3.5 h-3.5 rounded-full transition-transform duration-150 ease-soft',
        modelValue ? 'translate-x-5 bg-white' : 'translate-x-0 bg-surface border border-line shadow-card',
      ]"
    />
  </button>
</template>
