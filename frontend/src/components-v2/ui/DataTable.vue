<script setup>
const props = defineProps({
  columns: { type: Array, default: () => [] }, // [{key, label, width?}]
  rows: { type: Array, default: () => [] },
  emptyTitle: { type: String, default: '暂无数据' },
})
const emit = defineEmits(['row-action'])

function cellValue(row, col) {
  const v = row[col.key]
  if (v == null) return ''
  if (typeof v === 'boolean') return v ? '是' : '否'
  if (typeof v === 'object') return JSON.stringify(v)
  return String(v)
}
</script>

<template>
  <div class="rounded-lg border border-line overflow-hidden">
    <table class="w-full text-sm">
      <thead>
        <tr class="bg-surface-2 border-b border-line">
          <th
            v-for="col in columns"
            :key="col.key"
            class="text-left font-medium text-ink-soft px-3 py-2"
            :style="col.width ? { width: col.width } : {}"
          >
            {{ col.label }}
          </th>
          <th v-if="$slots['row-action']" class="w-10"></th>
        </tr>
      </thead>
      <tbody>
        <tr v-if="rows.length === 0">
          <td :colspan="columns.length + 1" class="text-center text-ink-faint py-8">
            {{ emptyTitle }}
          </td>
        </tr>
        <tr
          v-for="(row, ri) in rows"
          :key="ri"
          class="border-b border-line last:border-0 hover:bg-surface-2/50 transition-colors"
        >
          <td v-for="col in columns" :key="col.key" class="px-3 py-2 text-ink">
            <slot :name="`cell-${col.key}`" :row="row" :value="row[col.key]">
              {{ cellValue(row, col) }}
            </slot>
          </td>
          <td v-if="$slots['row-action']" class="px-2 text-right">
            <slot name="row-action" :row="row" :index="ri" />
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
