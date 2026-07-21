<script setup>
import { ref, watch, nextTick } from 'vue'

const props = defineProps({
  modelValue: { type: String, default: '' },
  placeholder: { type: String, default: '' },
  disabled: { type: Boolean, default: false },
  autoResize: { type: Boolean, default: false },
  rows: { type: Number, default: 3 },
})
defineEmits(['update:modelValue'])

const el = ref(null)

function resize() {
  if (!props.autoResize || !el.value) return
  el.value.style.height = 'auto'
  el.value.style.height = el.value.scrollHeight + 'px'
}

watch(
  () => props.modelValue,
  () => nextTick(resize)
)
</script>

<template>
  <textarea
    ref="el"
    :value="modelValue"
    :placeholder="placeholder"
    :disabled="disabled"
    :rows="rows"
    class="w-full bg-surface border border-line rounded-md px-3 py-1.5 text-sm text-ink placeholder:text-ink-faint transition-colors duration-150 outline-none focus:border-accent-border disabled:bg-surface-2 disabled:text-ink-faint disabled:cursor-not-allowed resize-y"
    @input="$emit('update:modelValue', $event.target.value)"
  />
</template>
