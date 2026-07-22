<script setup>
/**
 * PluginEventLog — 插件事件 feed（调试抽屉）。
 *
 * 不再用三列表格塞整段 JSON（窄列 break-all 会竖排字符）。
 * 改为：事件卡列表 + 一行人话摘要；详情默认折叠，展开后 pre 格式化。
 */
import { computed, ref } from 'vue'
import { usePluginStore } from '../../stores/index.js'
import Select from '../ui/Select.vue'
import EmptyState from '../ui/EmptyState.vue'
import Badge from '../ui/Badge.vue'

const plugin = usePluginStore()

function eventType(record) {
  if (!record?.event) return ''
  if (typeof record.event === 'object') return record.event.event_type || ''
  return String(record.event)
}

function eventData(record) {
  if (typeof record?.event === 'object') return record.event.data ?? null
  return record?.data ?? null
}

function isPipeline(record) {
  return record?.event && typeof record.event === 'object'
}

/** 一行摘要：关键字段优先，否则短 JSON */
function eventSummary(record) {
  const data = eventData(record)
  if (data == null) return '（无 payload）'
  if (typeof data !== 'object') return String(data)

  const bits = []
  if (data.version != null) bits.push(`v${data.version}`)
  if (data.writingMode != null) bits.push(`mode ${data.writingMode}`)
  if (data.campaignId) bits.push(`campaign ${String(data.campaignId).slice(0, 8)}…`)
  if (data.characterId) bits.push(`char ${String(data.characterId).slice(0, 8)}…`)
  if (data.conversationId) bits.push(`conv ${String(data.conversationId).slice(0, 8)}…`)
  if (data.messageCount != null) bits.push(`${data.messageCount} msgs`)
  if (data.pluginId) bits.push(data.pluginId)
  if (bits.length) return bits.join(' · ')

  try {
    const s = JSON.stringify(data)
    return s.length > 96 ? s.slice(0, 96) + '…' : s
  } catch {
    return String(data)
  }
}

function eventPretty(record) {
  const data = eventData(record)
  if (data == null) return ''
  try {
    return JSON.stringify(data, null, 2)
  } catch {
    return String(data)
  }
}

const allEvents = computed(() => plugin.pluginPipelineEvents.slice().reverse())

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
const expanded = ref(new Set())

const filteredEvents = computed(() => {
  if (!typeFilter.value) return allEvents.value
  return allEvents.value.filter((r) => eventType(r) === typeFilter.value)
})

function toggle(id) {
  const next = new Set(expanded.value)
  if (next.has(id)) next.delete(id)
  else next.add(id)
  expanded.value = next
}

function isOpen(id) {
  return expanded.value.has(id)
}

async function copyPayload(record) {
  const text = eventPretty(record)
  if (!text) return
  try {
    await navigator.clipboard.writeText(text)
  } catch {
    /* ignore */
  }
}
</script>

<template>
  <section class="flex flex-col gap-3 min-w-0">
    <header class="flex items-center gap-2 min-w-0">
      <h3 class="text-sm font-semibold text-ink shrink-0">插件事件</h3>
      <Badge variant="neutral" size="sm">{{ filteredEvents.length }} / {{ allEvents.length }}</Badge>
    </header>

    <Select v-model="typeFilter" :options="typeOptions" placeholder="按事件类型过滤" />

    <EmptyState
      v-if="filteredEvents.length === 0"
      :title="allEvents.length === 0 ? '无插件事件' : '当前过滤条件下无匹配'"
      description="启动应用或生成一轮后，插件广播会出现在这里。"
    />

    <ul v-else class="space-y-2 min-w-0">
      <li
        v-for="row in filteredEvents"
        :key="row.id"
        class="rounded-lg border border-line bg-surface overflow-hidden min-w-0"
      >
        <button
          type="button"
          class="w-full text-left px-3 py-2.5 flex items-start gap-2 hover:bg-surface-2/50 transition-colors min-w-0"
          @click="toggle(row.id)"
        >
          <span class="shrink-0 mt-0.5 text-[10px] font-mono text-ink-faint w-6 tabular-nums">{{ row.id }}</span>
          <div class="flex-1 min-w-0">
            <div class="flex items-center gap-1.5 flex-wrap">
              <Badge v-if="isPipeline(row)" variant="accent" size="sm">pipeline</Badge>
              <span class="text-[13px] font-medium text-ink font-mono truncate">{{ eventType(row) }}</span>
            </div>
            <p class="mt-0.5 text-xs text-ink-soft truncate" :title="eventSummary(row)">
              {{ eventSummary(row) }}
            </p>
          </div>
          <span class="shrink-0 text-ink-faint text-xs mt-0.5">{{ isOpen(row.id) ? '▴' : '▾' }}</span>
        </button>

        <div v-if="isOpen(row.id)" class="border-t border-line bg-surface-2/40 px-3 py-2 space-y-2">
          <div class="flex justify-end">
            <button
              type="button"
              class="text-[11px] text-accent hover:text-accent-bright"
              @click.stop="copyPayload(row)"
            >复制 JSON</button>
          </div>
          <pre
            class="text-[11px] leading-relaxed font-mono text-ink-soft whitespace-pre-wrap break-words max-h-48 overflow-y-auto m-0"
          >{{ eventPretty(row) || '（无 payload）' }}</pre>
        </div>
      </li>
    </ul>
  </section>
</template>
