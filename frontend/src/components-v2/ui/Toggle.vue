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
      'relative inline-flex w-10 h-5 rounded-full transition-colors duration-150',
      modelValue ? 'bg-accent' : 'bg-line',
      disabled ? 'opacity-40 cursor-not-allowed' : 'cursor-pointer',
    ]"
    @click="toggle"
  >
    <span
      :class="[
        'absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-bg transition-transform duration-150',
        modelValue ? 'translate-x-5' : 'translate-x-0',
      ]"
    />
  </button>
</template>
