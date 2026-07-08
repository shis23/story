<script setup>
defineProps({
  label: { type: String, default: '加载中…' },
  compact: { type: Boolean, default: false },
})
</script>

<template>
  <div
    v-if="compact"
    class="inline-flex items-center justify-center gap-2"
  >
    <span class="inline-block animate-spin text-ink-soft">◌</span>
    <span v-if="label" class="text-ink-soft text-sm">{{ label }}</span>
  </div>
  <div
    v-else
    class="flex flex-col items-center py-12 gap-2"
  >
    <span class="inline-block animate-spin text-ink-soft text-lg">◌</span>
    <span v-if="label" class="text-ink-soft text-sm">{{ label }}</span>
  </div>
</template>
