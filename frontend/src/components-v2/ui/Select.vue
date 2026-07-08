<script setup>
import { computed } from 'vue'

const props = defineProps({
  modelValue: { type: [String, Number, null], default: null },
  options: { type: Array, default: () => [] },
  placeholder: { type: String, default: '' },
  disabled: { type: Boolean, default: false },
  loading: { type: Boolean, default: false },
})
defineEmits(['update:modelValue'])

const normalized = computed(() =>
  props.options.map((o) =>
    typeof o === 'string' ? { label: o, value: o } : o
  )
)

const isDisabled = computed(() => props.disabled || props.loading)
</script>

<template>
  <select
    :value="modelValue"
    :disabled="isDisabled"
    class="w-full bg-surface-2 border border-line rounded-lg px-3 py-1.5 text-sm text-ink transition-colors duration-150 focus:border-accent disabled:opacity-40 disabled:cursor-not-allowed"
    @change="$emit('update:modelValue', $event.target.value)"
  >
    <option v-if="placeholder || loading" :value="null" disabled>
      {{ loading ? '加载中…' : placeholder }}
    </option>
    <option
      v-for="opt in normalized"
      :key="opt.value"
      :value="opt.value"
    >
      {{ opt.label }}
    </option>
  </select>
</template>
