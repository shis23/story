<script setup>
const props = defineProps({
  modelValue: { type: String, default: '' },
  placeholder: { type: String, default: '' },
  disabled: { type: Boolean, default: false },
  invalid: { type: Boolean, default: false },
  type: { type: String, default: 'text' },
})
defineEmits(['update:modelValue'])

function classes() {
  return [
    'w-full bg-surface-2 border rounded-lg px-3 py-1.5 text-sm text-ink placeholder:text-ink-faint',
    'transition-colors duration-150',
    'disabled:opacity-40 disabled:cursor-not-allowed',
    props.invalid
      ? 'border-err/50 focus:border-err text-err'
      : 'border-line focus:border-accent',
  ]
}
</script>

<template>
  <input
    :type="type"
    :value="modelValue"
    :placeholder="placeholder"
    :disabled="disabled"
    :class="classes()"
    @input="$emit('update:modelValue', $event.target.value)"
  />
</template>
