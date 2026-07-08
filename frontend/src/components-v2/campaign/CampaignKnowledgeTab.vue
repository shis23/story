<script setup>
import { ref, computed, onMounted, watch } from 'vue'
import { listCharacterKnowledge } from '../../tauri-api.js'
import {
  instanceLabel,
  knowledgeSourceText,
  propagationText,
  relayChainText,
  shortId,
  sourceLabel,
} from '../../utils/campaignDisplay.js'
import DataTable from '../ui/DataTable.vue'
import Badge from '../ui/Badge.vue'
import Select from '../ui/Select.vue'
import EmptyState from '../ui/EmptyState.vue'
import LoadingState from '../ui/LoadingState.vue'

const props = defineProps({
  campaignId: { type: String, required: true }
})

// ─── 状态 ───
const knowledge = ref([])
const loading = ref(false)
const error = ref(null)
const filterSource = ref('') // '' = 全部
const filterInstanceId = ref('') // '' = 全部

// ─── 加载 ───
async function load() {
  if (!props.campaignId) return
  loading.value = true
  error.value = null
  try {
    knowledge.value = await listCharacterKnowledge(props.campaignId)
  } catch (e) {
    error.value = String(e)
  } finally {
    loading.value = false
  }
}

onMounted(load)

watch(() => props.campaignId, () => {
  if (props.campaignId) load()
})

// ─── 过滤后的知识列表 ───
const filteredKnowledge = computed(() => {
  return knowledge.value.filter(k => {
    if (filterSource.value && k.source !== filterSource.value) return false
    if (filterInstanceId.value && k.character_id !== filterInstanceId.value) return false
    return true
  })
})

// ─── 来源显示 ───
const sourceOptions = [
  { value: '', label: '全部来源' },
  { value: 'witnessed', label: '👁 亲眼' },
  { value: 'told_by_other', label: '💬 被告知' },
  { value: 'inferred', label: '🔮 推断' },
  { value: 'backstory', label: '📖 背景' },
]

// ─── 来源 → Badge variant 映射 ───
const sourceBadgeVariant = {
  witnessed: 'ok',
  told_by_other: 'accent',
  inferred: 'warn',
  backstory: 'neutral',
}

// ─── 实例过滤选项:从已有知识的 character_id 去重 ───
const instanceOptions = computed(() => {
  const seen = new Map()
  for (const k of knowledge.value) {
    if (k.character_id && !seen.has(k.character_id)) {
      seen.set(k.character_id, k.character_name || shortId(k.character_id))
    }
  }
  return [{ value: '', label: '全部实例' }, ...[...seen.entries()].map(([value, label]) => ({ value, label }))]
})

// ─── DataTable 配置 ───
const columns = [
  { key: 'knowledge_text', label: '知识' },
  { key: 'source', label: '来源', width: '110px' },
  { key: 'provenance', label: '知道者/来源角色', width: '180px' },
  { key: 'turn_number', label: '轮次', width: '70px' },
]

// ─── 暴露 refresh 给父组件 ───
defineExpose({ refresh: load })
</script>

<template>
  <!-- 过滤条 -->
  <div class="flex gap-2 mb-3 flex-wrap">
    <div class="min-w-[120px]">
      <Select v-model="filterSource" :options="sourceOptions" />
    </div>
    <div class="flex-1 min-w-0">
      <Select v-model="filterInstanceId" :options="instanceOptions" />
    </div>
  </div>

  <LoadingState v-if="loading" />
  <div v-else-if="error" class="text-center text-err text-sm py-8">加载失败: {{ error }}</div>
  <EmptyState
    v-else-if="filteredKnowledge.length === 0"
    :title="knowledge.length === 0 ? '暂无知识' : '当前过滤条件下无匹配'"
  />

  <template v-else>
    <DataTable :columns="columns" :rows="filteredKnowledge" empty-title="暂无知识">
      <template #cell-knowledge_text="{ row }">
        <div class="text-xs text-ink leading-relaxed">{{ row.knowledge_text }}</div>
        <div v-if="row.pinned" class="text-[10px] text-accent mt-1">📌 已固定</div>
        <div v-if="propagationText(row)" class="mt-1">
          <Badge variant="warn" size="sm">{{ propagationText(row) }}</Badge>
        </div>
      </template>
      <template #cell-source="{ row }">
        <Badge :variant="sourceBadgeVariant[row.source] || 'neutral'" size="sm">
          {{ knowledgeSourceText(row.source) }}
        </Badge>
      </template>
      <template #cell-provenance="{ row }">
        <div class="text-[10px] text-ink-soft leading-snug">
          <div>{{ row.provenance_text || instanceLabel(row) }}</div>
          <div v-if="sourceLabel(row)">来源 {{ sourceLabel(row) }}</div>
          <div v-if="relayChainText(row)">链路 {{ relayChainText(row) }}</div>
        </div>
      </template>
      <template #cell-turn_number="{ row }">
        <span v-if="row.turn_number" class="text-[10px] text-ink-soft">{{ row.turn_number }}</span>
      </template>
    </DataTable>
  </template>
</template>
