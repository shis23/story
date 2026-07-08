<script setup>
const props = defineProps({
  modelValue: { type: [String, Number], default: null },
  options: { type: Array, default: () => [] }, // [{label, value}] 或字符串数组
  disabled: { type: Boolean, default: false },
})
const emit = defineEmits(['update:modelValue'])

function normalized() {
  return props.options.map((o) =>
    typeof o === 'string' || typeof o === 'number' ? { label: String(o), value: o } : o,
  )
}
function select(value) {
  if (props.disabled) return
  emit('update:modelValue', value)
}
</script>

<template>
  <div
    class="inline-flex items-center gap-0.5 rounded-lg bg-surface-2 p-0.5"
    :class="{ 'opacity-40 pointer-events-none': disabled }"
  >
    <button
      v-for="opt in normalized()"
      :key="opt.value"
      type="button"
      class="px-3 py-1 text-xs rounded-md transition-colors duration-150 select-none"
      :class="
        modelValue === opt.value
          ? 'bg-accent text-bg font-medium shadow-glow-accent'
          : 'text-ink-soft hover:text-ink'
      "
      @click="select(opt.value)"
    >
      {{ opt.label }}
    </button>
  </div>
</template>
