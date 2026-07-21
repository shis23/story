<script setup>
defineProps({
  title: { type: String, default: '暂无数据' },
  description: { type: String, default: '' },
})
</script>

<template>
  <div class="flex flex-col items-center justify-center py-14 gap-3 text-center">
    <div v-if="$slots.icon" class="text-accent/60 mb-1">
      <slot name="icon" />
    </div>
    <p class="font-semibold text-lg text-ink">{{ title }}</p>
    <p v-if="description" class="text-ink-soft text-sm max-w-sm leading-relaxed">
      {{ description }}
    </p>
    <div v-if="$slots.action" class="mt-2">
      <slot name="action" />
    </div>
  </div>
</template>
