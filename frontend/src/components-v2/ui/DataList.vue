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
  <div class="flex flex-col gap-0.5">
    <div v-if="items.length === 0" class="py-12 flex flex-col items-center gap-2">
      <slot name="empty-icon">
        <span class="text-2xl text-ink-faint">∅</span>
      </slot>
      <p class="text-sm text-ink-soft">{{ emptyTitle }}</p>
      <p v-if="emptyDescription" class="text-xs text-ink-faint">{{ emptyDescription }}</p>
      <slot name="empty-action" />
    </div>
    <div
      v-for="(item, index) in items"
      :key="item[activeKey] ?? index"
      class="rounded-lg px-3 py-2 cursor-pointer transition-colors duration-100 hover:bg-surface-2"
      :class="{ 'bg-accent-soft border border-accent-border': isActive(item, index) }"
      @click="emit('select', item, index)"
    >
      <slot name="item" :item="item" :index="index" :active="isActive(item, index)">
        {{ typeof item === 'string' ? item : item.label || item.name || JSON.stringify(item) }}
      </slot>
    </div>
  </div>
</template>
