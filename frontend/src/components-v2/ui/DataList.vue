<script setup>
const props = defineProps({
  items: { type: Array, default: () => [] },
  // 选中态:传一个 id 字段名,或用 activeItem 引用对比
  activeKey: { type: String, default: 'id' },
  emptyTitle: { type: String, default: '暂无数据' },
  emptyDescription: { type: String, default: '' },
})
const emit = defineEmits(['select'])

function isActive(item, index) {
  if (props.activeKey && item && props.activeKey in item) return false
  return false
}
</script>

<template>
  <div class="rounded-xl border border-line bg-surface shadow-card overflow-hidden divide-y divide-line">
    <div v-if="items.length === 0" class="py-12 flex flex-col items-center gap-2">
      <slot name="empty-icon">
        <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" class="text-ink-faint" aria-hidden="true"><circle cx="12" cy="12" r="8.5"/><path d="M9 9.5h.01M15 9.5h.01M9 15.5c.8-.8 1.9-1.2 3-1.2s2.2.4 3 1.2"/></svg>
      </slot>
      <p class="text-sm text-ink-soft">{{ emptyTitle }}</p>
      <p v-if="emptyDescription" class="text-xs text-ink-faint">{{ emptyDescription }}</p>
      <slot name="empty-action" />
    </div>
    <div
      v-for="(item, index) in items"
      :key="item[activeKey] ?? index"
      class="px-3 py-2 cursor-pointer transition-colors duration-100 hover:bg-surface-2/60 border-l-2 border-transparent"
      :class="{ 'bg-accent-soft !border-accent': isActive(item, index) }"
      @click="emit('select', item, index)"
    >
      <slot name="item" :item="item" :index="index" :active="isActive(item, index)">
        {{ typeof item === 'string' ? item : item.label || item.name || JSON.stringify(item) }}
      </slot>
    </div>
  </div>
</template>
