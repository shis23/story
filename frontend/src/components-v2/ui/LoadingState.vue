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
    <span class="inline-block animate-spin text-ink-soft" aria-hidden="true">
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><path d="M12 3a9 9 0 1 0 9 9"/></svg>
    </span>
    <span v-if="label" class="text-ink-soft text-sm">{{ label }}</span>
  </div>
  <div
    v-else
    class="flex flex-col items-center py-12 gap-3"
  >
    <!-- 骨架块：surface-2 呼吸，不用旋转大 spinner 占屏 -->
    <div class="w-full max-w-sm flex flex-col gap-2.5 px-6" aria-hidden="true">
      <div class="h-3 w-2/3 rounded bg-surface-2 animate-pulse" />
      <div class="h-3 w-full rounded bg-surface-2 animate-pulse" />
      <div class="h-3 w-4/5 rounded bg-surface-2 animate-pulse" />
    </div>
    <span v-if="label" class="text-ink-soft text-sm">{{ label }}</span>
  </div>
</template>
