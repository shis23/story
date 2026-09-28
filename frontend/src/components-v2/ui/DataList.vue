<script setup>
import { computed, ref } from 'vue'
const props = defineProps({
  items: { type: Array, default: () => [] },
  // 行的业务主键字段名（默认 id），用于 v-for key 与 activeId 比对
  activeKey: { type: String, default: 'id' },
  // 当前选中行的 activeKey 取值（F-05：此前没有这个 prop，isActive 恒 false）
  activeId: { type: [String, Number], default: null },
  emptyTitle: { type: String, default: '暂无数据' },
  emptyDescription: { type: String, default: '' },
  // 首屏最多渲染多少行（0 = 不限制）。大列表（角色卡库、日志）避免一次性全量渲染（F-34）
  maxItems: { type: Number, default: 0 },
})
const emit = defineEmits(['select'])

const showAll = ref(false)

function isActive(item, index) {
  if (props.activeId == null) return false
  if (item == null) return false
  const key = props.activeKey
  if (key && typeof item === 'object' && key in item) return item[key] === props.activeId
  return index === props.activeId
}

const renderedItems = computed(() => {
  if (props.maxItems > 0 && !showAll.value) return props.items.slice(0, props.maxItems)
  return props.items
})
const hiddenCount = computed(() =>
  props.maxItems > 0 && !showAll.value ? Math.max(0, props.items.length - props.maxItems) : 0,
)
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
      v-for="(item, index) in renderedItems"
      :key="item?.[activeKey] ?? index"
      class="px-3 py-2 cursor-pointer transition-colors duration-100 hover:bg-surface-2/60 border-l-2 border-transparent"
      :class="{ 'bg-accent-soft !border-accent': isActive(item, index) }"
      @click="emit('select', item, index)"
    >
      <slot name="item" :item="item" :index="index" :active="isActive(item, index)">
        {{ typeof item === 'string' ? item : item.label || item.name || JSON.stringify(item) }}
      </slot>
    </div>
    <button
      v-if="hiddenCount > 0"
      type="button"
      class="w-full px-3 py-2 text-xs text-ink-soft hover:bg-surface-2/60 transition-colors"
      @click="showAll = true"
    >显示更多（还有 {{ hiddenCount }} 条）</button>
  </div>
</template>
