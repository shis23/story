<script setup>
defineProps({
  message: { type: String, default: '出错了' },
  detail: { type: String, default: '' },
})
</script>

<template>
  <div class="flex flex-col items-center justify-center py-12 px-6 gap-3 text-center border border-err/40 rounded-lg bg-surface">
    <div v-if="$slots.icon" class="border border-err/40 rounded-full p-3 text-err">
      <slot name="icon" />
    </div>
    <p class="text-err text-sm font-semibold">{{ message }}</p>
    <p v-if="detail" class="text-ink-soft text-sm">
      {{ detail }}
    </p>
    <div v-if="$slots.retry" class="mt-1">
      <slot name="retry" />
    </div>
  </div>
</template>
