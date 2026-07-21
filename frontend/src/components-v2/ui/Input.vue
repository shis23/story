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
    'w-full bg-surface border rounded-md px-3 py-1.5 text-sm text-ink placeholder:text-ink-faint',
    'transition-colors duration-150 outline-none',
    'disabled:bg-surface-2 disabled:text-ink-faint disabled:cursor-not-allowed',
    props.invalid
      ? 'border-err/50 focus:border-err'
      : 'border-line focus:border-accent-border',
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
