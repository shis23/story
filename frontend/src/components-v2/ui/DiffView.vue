<script setup>
const props = defineProps({
  before: { type: String, default: '' },
  after: { type: String, default: '' },
})

// 逐行 diff:简单实现,不引入 diff 库。按行比对,标记 added/removed。
// 适用于 patch preview / MVU schema apply 这类小规模文本差异。
function lines() {
  const beforeLines = props.before.split('\n')
  const afterLines = props.after.split('\n')
  const result = []
  const max = Math.max(beforeLines.length, afterLines.length)
  for (let i = 0; i < max; i += 1) {
    const b = beforeLines[i]
    const a = afterLines[i]
    if (b === undefined && a !== undefined) result.push({ type: 'added', text: a })
    else if (a === undefined && b !== undefined) result.push({ type: 'removed', text: b })
    else if (b !== a) {
      result.push({ type: 'removed', text: b })
      result.push({ type: 'added', text: a })
    } else {
      result.push({ type: 'unchanged', text: a })
    }
  }
  return result
}
const rowClass = {
  added: 'bg-ok/10 text-ok',
  removed: 'bg-err/10 text-err line-through opacity-70',
  unchanged: 'text-ink-soft',
}
const rowPrefix = { added: '+', removed: '−', unchanged: ' ' }
</script>

<template>
  <div class="rounded-lg border border-line overflow-auto font-mono text-xs">
    <div
      v-for="(row, i) in lines()"
      :key="i"
      class="px-2 py-0.5 flex gap-1"
      :class="rowClass[row.type]"
    >
      <span class="select-none opacity-50">{{ rowPrefix[row.type] }}</span>
      <span class="whitespace-pre-wrap break-all">{{ row.text }}</span>
    </div>
  </div>
</template>
