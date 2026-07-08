<script setup>
defineProps({
  message: { type: String, default: '出错了' },
  detail: { type: String, default: '' },
})
</script>

<template>
  <div class="flex flex-col items-center justify-center py-12 gap-3 text-center">
    <div v-if="$slots.icon" class="bg-err/10 rounded-full p-3 text-err">
      <slot name="icon" />
    </div>
    <p class="text-err text-sm font-medium">{{ message }}</p>
    <p v-if="detail" class="text-ink-faint text-sm">
      {{ detail }}
    </p>
    <div v-if="$slots.retry">
      <slot name="retry" />
    </div>
  </div>
</template>
