<script setup>
const props = defineProps({
  modelValue: { type: Boolean, default: false },
  disabled: { type: Boolean, default: false },
  label: { type: String, default: '' },
})
defineEmits(['update:modelValue'])
</script>

<template>
  <label
    class="inline-flex items-center gap-2 select-none"
    :class="disabled ? 'cursor-not-allowed' : 'cursor-pointer'"
  >
    <input
      type="checkbox"
      :checked="modelValue"
      :disabled="disabled"
      class="peer sr-only"
      @change="$emit('update:modelValue', $event.target.checked)"
    />
    <span
      class="h-4 w-4 rounded-sm border flex items-center justify-center transition-colors duration-150"
      :class="[
        modelValue ? 'bg-accent border-accent' : 'bg-surface border-line',
        disabled ? 'opacity-40' : 'peer-hover:border-accent-border',
      ]"
    >
      <svg
        v-if="modelValue"
        class="text-white"
        width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d="M5 12.5l4.5 4.5L19 7" />
      </svg>
    </span>
    <span v-if="label" class="text-sm" :class="disabled ? 'text-ink-faint' : 'text-ink'">{{ label }}</span>
  </label>
</template>
