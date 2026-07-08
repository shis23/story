<script setup>
/**
 * GreetingSelector — 开场白选择条（v2 迁移）
 *
 * 迁移自 App.vue:1308-1321 的开场白横向滚动按钮条。
 * 纯展示组件，状态由父级（ConversationViewport → writingStore）持有。
 */
defineProps({
  /** greetingOptions 数组，元素形如 { label, content } */
  options: { type: Array, default: () => [] },
  /** 当前选中的开场白索引 */
  selectedIndex: { type: Number, default: 0 },
})

const emit = defineEmits(['select'])
</script>

<template>
  <div class="pt-3 pb-2">
    <div class="flex items-center gap-2 overflow-x-auto">
      <span class="shrink-0 text-xs text-ink-soft">开场</span>
      <button
        v-for="(option, i) in options"
        :key="option.label"
        @click="emit('select', i)"
        class="shrink-0 min-h-[36px] px-3 rounded-lg text-xs font-medium border transition-colors"
        :class="selectedIndex === i
          ? 'bg-accent text-white border-accent shadow-glow-accent'
          : 'bg-surface text-ink-soft border-line hover:border-accent hover:text-accent'"
      >
        {{ option.label }}
      </button>
    </div>
  </div>
</template>
