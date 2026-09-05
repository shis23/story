<script setup>
import { ref, onMounted, watch } from 'vue'
import { listRoundSummaries } from '../../tauri-api.js'
import DataTable from '../ui/DataTable.vue'
import Badge from '../ui/Badge.vue'
import EmptyState from '../ui/EmptyState.vue'
import LoadingState from '../ui/LoadingState.vue'
import { errorText } from '../../utils/errorText.js'

const props = defineProps({
  campaignId: { type: String, required: true }
})

// ─── 状态 ───
const summaries = ref([])
const loading = ref(false)
const error = ref(null)

// ─── 加载 ───
async function load() {
  if (!props.campaignId) return
  loading.value = true
  error.value = null
  try {
    summaries.value = await listRoundSummaries(props.campaignId)
  } catch (e) {
    error.value = errorText(e)
  } finally {
    loading.value = false
  }
}

onMounted(load)

watch(() => props.campaignId, () => {
  if (props.campaignId) load()
})

// ─── 时间格式化(与其他组件统一用 toLocaleString) ───
function formatTime(ts) {
  if (!ts) return ''
  try {
    return new Date(ts).toLocaleString('zh-CN')
  } catch {
    return String(ts)
  }
}

// ─── DataTable 配置 ───
const columns = [
  { key: 'turn', label: '轮次', width: '90px' },
  { key: 'content', label: '摘要' },
  { key: 'created_at', label: '时间线', width: '180px' },
]

// ─── 暴露 refresh 给父组件 ───
defineExpose({ refresh: load })
</script>

<template>
  <LoadingState v-if="loading" />
  <div v-else-if="error" class="text-center text-err text-sm py-8">加载失败: {{ error }}</div>
  <EmptyState v-else-if="summaries.length === 0" title="暂无摘要" />

  <template v-else>
    <ul class="sm:hidden divide-y divide-line" data-testid="summary-mobile">
      <li v-for="row in summaries" :key="row.id" class="py-3">
        <div class="flex flex-wrap items-center gap-2 mb-2">
          <Badge variant="accent" size="sm">第 {{ row.turn }} 轮</Badge>
          <time class="text-[10px] text-ink-soft">{{ formatTime(row.created_at) }}</time>
        </div>
        <p class="text-xs text-ink-soft leading-relaxed whitespace-pre-wrap [overflow-wrap:anywhere]">{{ row.content }}</p>
      </li>
    </ul>
    <DataTable class="hidden sm:block" :columns="columns" :rows="summaries" empty-title="暂无摘要">
      <template #cell-turn="{ row }">
        <Badge variant="accent" size="sm">第 {{ row.turn }} 轮</Badge>
      </template>
      <template #cell-content="{ row }">
        <div class="text-xs text-ink-soft leading-relaxed">{{ row.content }}</div>
      </template>
      <template #cell-created_at="{ row }">
        <span v-if="row.created_at" class="text-[10px] text-ink-soft">{{ formatTime(row.created_at) }}</span>
      </template>
    </DataTable>
  </template>
</template>
