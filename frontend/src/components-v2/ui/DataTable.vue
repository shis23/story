<script setup>
import { computed, ref, useSlots } from 'vue'
const props = defineProps({
  columns: { type: Array, default: () => [] }, // [{key, label, width?, mono?}]
  rows: { type: Array, default: () => [] },
  emptyTitle: { type: String, default: '暂无数据' },
  // 行稳定键：属性名字符串或 (row) => key 函数。空 = 回退数组下标（F-20）
  rowKey: { type: [String, Function], default: '' },
  // 首屏最多渲染多少行（0 = 不限制）。大列表避免一次性全量渲染（F-34）
  maxItems: { type: Number, default: 0 },
})

// F-33：`row-action` 从来不作为事件被使用（3 个调用点用的都是 #row-action 插槽），
// 死声明已删除；空态 colspan 也不能恒 +1，否则没有插槽的调用点会多出一列。
const slots = useSlots()
const hasRowAction = computed(() => Boolean(slots['row-action']))

const showAll = ref(false)
const renderedRows = computed(() => {
  if (props.maxItems > 0 && !showAll.value) return props.rows.slice(0, props.maxItems)
  return props.rows
})
const hiddenCount = computed(() =>
  props.maxItems > 0 && !showAll.value ? Math.max(0, props.rows.length - props.maxItems) : 0,
)
const colSpan = computed(() => props.columns.length + (hasRowAction.value ? 1 : 0))

function rowKeyValue(row, index) {
  if (typeof props.rowKey === 'function') return props.rowKey(row, index)
  if (typeof props.rowKey === 'string' && props.rowKey && row && row[props.rowKey] != null) {
    return row[props.rowKey]
  }
  return index
}

function cellValue(row, col) {
  const v = row[col.key]
  if (v == null) return ''
  if (typeof v === 'boolean') return v ? '是' : '否'
  if (typeof v === 'object') return JSON.stringify(v)
  return String(v)
}
</script>

<template>
  <div class="rounded-lg border border-line bg-surface overflow-hidden">
    <table class="w-full text-sm">
      <thead>
        <tr class="border-b border-line">
          <th
            v-for="col in columns"
            :key="col.key"
            class="text-left text-xs font-medium text-ink-faint px-3 py-2"
            :class="{ 'font-mono': col.mono }"
            :style="col.width ? { width: col.width } : {}"
          >
            {{ col.label }}
          </th>
          <th v-if="hasRowAction" class="w-10"></th>
        </tr>
      </thead>
      <tbody>
        <tr v-if="rows.length === 0">
          <td :colspan="colSpan" class="text-center text-ink-faint py-8">
            {{ emptyTitle }}
          </td>
        </tr>
        <tr
          v-for="(row, ri) in renderedRows"
          :key="rowKeyValue(row, ri)"
          class="border-b border-line last:border-0 hover:bg-surface-2/60 transition-colors"
        >
          <td
            v-for="col in columns"
            :key="col.key"
            class="px-3 py-2 text-ink"
            :class="{ 'font-mono text-xs': col.mono }"
          >
            <slot :name="`cell-${col.key}`" :row="row" :value="row[col.key]">
              {{ cellValue(row, col) }}
            </slot>
          </td>
          <td v-if="hasRowAction" class="px-2 text-right">
            <slot name="row-action" :row="row" :index="ri" />
          </td>
        </tr>
      </tbody>
    </table>
    <button
      v-if="hiddenCount > 0"
      type="button"
      class="w-full border-t border-line px-3 py-2 text-xs text-ink-soft hover:bg-surface-2/60 transition-colors"
      @click="showAll = true"
    >显示更多（还有 {{ hiddenCount }} 行）</button>
  </div>
</template>
