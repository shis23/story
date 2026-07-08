<script setup>
/**
 * PluginEventLog — 插件事件 feed。
 *
 * 读 pluginStore.pluginPipelineEvents(最近 100 条,store 已裁剪到 MAX)。
 * 渲染 type / detail / seq,支持按事件类型过滤(ui/Select)。
 *
 * 事件记录结构(见 usePluginBridge.pushPluginEventRecord):
 *   流水线事件: { id, event: { event_type, data } }
 *   普通插件事件: { id, event: 'EVENT_NAME', data: {...} }
 * 这里统一提取 type 字符串供显示与过滤。
 */
import { computed, ref } from 'vue'
import { usePluginStore } from '../../stores/index.js'
import DataTable from '../ui/DataTable.vue'
import Select from '../ui/Select.vue'
import EmptyState from '../ui/EmptyState.vue'
import Badge from '../ui/Badge.vue'

const plugin = usePluginStore()

// ─── 事件类型提取 ───
function eventType(record) {
  if (!record?.event) return ''
  if (typeof record.event === 'object') return record.event.event_type || ''
  return String(record.event)
}

function eventDetail(record) {
  const data = typeof record?.event === 'object' ? record.event.data : record.data
  if (!data) return ''
  try {
    return JSON.stringify(data)
  } catch {
    return String(data)
  }
}

// ─── 最近 100 条倒序(最新在上) ───
const allEvents = computed(() =>
  plugin.pluginPipelineEvents.slice().reverse(),
)

// ─── 过滤选项:从已有事件去重 ───
const typeOptions = computed(() => {
  const seen = new Set()
  const opts = [{ value: '', label: '全部类型' }]
  for (const r of allEvents.value) {
    const t = eventType(r)
    if (t && !seen.has(t)) {
      seen.add(t)
      opts.push({ value: t, label: t })
    }
  }
  return opts
})

const typeFilter = ref('')

const filteredEvents = computed(() => {
  if (!typeFilter.value) return allEvents.value
  return allEvents.value.filter((r) => eventType(r) === typeFilter.value)
})

// ─── DataTable 配置 ───
const columns = [
  { key: 'seq', label: '#', width: '60px' },
  { key: 'type', label: '类型', width: '180px' },
  { key: 'detail', label: '详情' },
]

// ─── 流水线事件标记(对象型 event 来自 broadcastPluginPipelineEvent) ───
function isPipeline(record) {
  return record?.event && typeof record.event === 'object'
}
</script>

<template>
  <section class="flex flex-col gap-3">
    <header class="flex items-center gap-2">
      <h3 class="text-sm font-semibold text-ink">插件事件</h3>
      <Badge variant="neutral" size="sm">{{ filteredEvents.length }} / {{ allEvents.length }}</Badge>
    </header>

    <div class="flex gap-2">
      <div class="min-w-[180px] flex-1">
        <Select v-model="typeFilter" :options="typeOptions" placeholder="按事件类型过滤" />
      </div>
    </div>

    <EmptyState
      v-if="filteredEvents.length === 0"
      :title="allEvents.length === 0 ? '无插件事件' : '当前过滤条件下无匹配'"
    />

    <DataTable
      v-else
      :columns="columns"
      :rows="filteredEvents"
      empty-title="无插件事件"
    >
      <template #cell-seq="{ row }">
        <span class="text-xs font-mono text-ink-faint">{{ row.id }}</span>
      </template>
      <template #cell-type="{ row }">
        <div class="flex items-center gap-1.5">
          <Badge v-if="isPipeline(row)" variant="accent" size="sm">pipeline</Badge>
          <span class="text-xs font-mono text-ink">{{ eventType(row) }}</span>
        </div>
      </template>
      <template #cell-detail="{ row }">
        <span class="text-xs font-mono text-ink-soft break-all">{{ eventDetail(row) }}</span>
      </template>
    </DataTable>
  </section>
</template>
