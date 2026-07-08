<script setup>
const props = defineProps({
  modelValue: { type: Boolean, default: false },
  disabled: { type: Boolean, default: false },
  label: { type: String, default: '' },
})
defineEmits(['update:modelValue'])
</script>

<template>
  <label class="inline-flex items-center gap-2 select-none" :class="{ 'opacity-40 cursor-not-allowed': disabled }">
    <input
      type="checkbox"
      :checked="modelValue"
      :disabled="disabled"
      class="h-4 w-4 rounded border-line bg-surface-2 text-accent focus:ring-accent focus:ring-offset-0 cursor-pointer"
      @change="$emit('update:modelValue', $event.target.checked)"
    />
    <span v-if="label" class="text-sm text-ink">{{ label }}</span>
  </label>
</template>
