<script setup>
const props = defineProps({
  modelValue: { type: Number, default: 0 },
  min: { type: Number, default: 0 },
  max: { type: Number, default: 100 },
  step: { type: Number, default: 1 },
  disabled: { type: Boolean, default: false },
})
defineEmits(['update:modelValue'])
</script>

<template>
  <input
    type="range"
    :value="modelValue"
    :min="min"
    :max="max"
    :step="step"
    :disabled="disabled"
    class="w-full h-2 rounded-full bg-surface-2 border border-line appearance-none cursor-pointer accent-accent outline-none transition-colors duration-150 disabled:opacity-40 disabled:cursor-not-allowed
      [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:w-4 [&::-webkit-slider-thumb]:h-4 [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-accent [&::-webkit-slider-thumb]:border [&::-webkit-slider-thumb]:border-accent-bright [&::-webkit-slider-thumb]:shadow-card [&::-webkit-slider-thumb]:cursor-pointer [&::-webkit-slider-thumb]:transition-colors [&::-webkit-slider-thumb]:hover:bg-accent-bright
      [&::-moz-range-thumb]:w-4 [&::-moz-range-thumb]:h-4 [&::-moz-range-thumb]:rounded-full [&::-moz-range-thumb]:bg-accent [&::-moz-range-thumb]:border [&::-moz-range-thumb]:border-accent-bright [&::-moz-range-thumb]:shadow-card [&::-moz-range-thumb]:cursor-pointer [&::-moz-range-thumb]:transition-colors [&::-moz-range-thumb]:hover:bg-accent-bright"
    @input="$emit('update:modelValue', Number($event.target.value))"
  />
</template>
