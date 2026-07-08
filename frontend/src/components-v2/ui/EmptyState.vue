<script setup>
defineProps({
  title: { type: String, default: '暂无数据' },
  description: { type: String, default: '' },
})
</script>

<template>
  <div class="flex flex-col items-center justify-center py-12 gap-3 text-center">
    <div v-if="$slots.icon" class="text-ink-faint">
      <slot name="icon" />
    </div>
    <p class="text-ink-soft text-sm font-medium">{{ title }}</p>
    <p v-if="description" class="text-ink-faint text-sm">
      {{ description }}
    </p>
    <div v-if="$slots.action">
      <slot name="action" />
    </div>
  </div>
</template>
