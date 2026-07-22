<script setup>
defineProps({
  title: { type: String, default: '暂无数据' },
  description: { type: String, default: '' },
})
</script>

<template>
  <!-- w-full min-w-0：在窄抽屉里占满可用宽，不按最长文案撑开父级 -->
  <div class="w-full min-w-0 flex flex-col items-center justify-center py-10 px-2 gap-3 text-center box-border">
    <div v-if="$slots.icon" class="text-accent/60 mb-1">
      <slot name="icon" />
    </div>
    <p class="font-semibold text-base sm:text-lg text-ink max-w-full break-words">{{ title }}</p>
    <p v-if="description" class="text-ink-soft text-sm max-w-[280px] w-full leading-relaxed break-words">
      {{ description }}
    </p>
    <div v-if="$slots.action" class="mt-2">
      <slot name="action" />
    </div>
  </div>
</template>
