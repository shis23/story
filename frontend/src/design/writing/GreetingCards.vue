<script setup>
/**
 * GreetingCards — 多开场白选择（重设计：从横向 pill 条 → 可读的卡片列表）。
 * 纯展示。事件：select(index)，对齐 useGreeting.selectGreeting。
 */
defineProps({
  options: { type: Array, default: () => [] }, // [{ label, content }]
  selectedIndex: { type: Number, default: 0 },
})
const emit = defineEmits(['select'])
</script>

<template>
  <section class="mb-8">
    <div class="flex items-center gap-2 mb-3">
      <span class="text-[11px] tracking-wide text-ink-faint">选择开场</span>
      <span class="h-px flex-1 bg-line"></span>
    </div>
    <div class="space-y-2">
      <button
        v-for="(option, i) in options"
        :key="option.label"
        @click="emit('select', i)"
        class="w-full rounded-lg border p-4 text-left transition-all"
        :class="i === selectedIndex
          ? 'border-accent-border bg-accent-soft/40 shadow-card'
          : 'border-line bg-surface hover:border-accent-border hover:shadow-card'"
      >
        <div class="flex items-center gap-2 mb-1.5">
          <span
            class="text-xs font-medium"
            :class="i === selectedIndex ? 'text-accent-bright' : 'text-ink-soft'"
          >{{ option.label }}</span>
          <span v-if="i === selectedIndex" class="ml-auto w-1.5 h-1.5 rounded-full bg-accent"></span>
        </div>
        <p class="text-[13px] leading-relaxed text-ink-soft line-clamp-2 prose-fiction">{{ option.content }}</p>
      </button>
    </div>
  </section>
</template>
