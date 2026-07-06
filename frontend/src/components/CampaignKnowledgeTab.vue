<script setup>
import { ref, computed, onMounted, watch } from 'vue'
import { listCharacterKnowledge } from '../tauri-api.js'

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

// ─── 按来源分组 ───
const groupedBySource = computed(() => {
  const groups = {}
  for (const k of filteredKnowledge.value) {
    const src = k.source || 'unknown'
    if (!groups[src]) groups[src] = []
    groups[src].push(k)
  }
  return groups
})

// ─── 来源显示 ───
const sourceOptions = [
  { value: '', label: '全部来源' },
  { value: 'witnessed', label: '👁 亲眼' },
  { value: 'told_by_other', label: '💬 被告知' },
  { value: 'inferred', label: '🔮 推断' },
  { value: 'backstory', label: '📖 背景' },
]

function knowledgeSourceText(source) {
  const map = { witnessed: '👁 亲眼', told_by_other: '💬 被告知', inferred: '🔮 推断', backstory: '📖 背景' }
  return map[source] || source
}

// ─── 实例过滤选项：从已有知识的 character_id 去重，避免要求用户手输 ID ───
const instanceOptions = computed(() => {
  const seen = new Map() // id -> 显示名（这里只有 id，截断显示）
  for (const k of knowledge.value) {
    if (k.character_id && !seen.has(k.character_id)) {
      seen.set(k.character_id, k.character_id.slice(0, 8))
    }
  }
  return [{ value: '', label: '全部实例' }, ...[...seen.entries()].map(([value, label]) => ({ value, label: '实例 ' + label }))]
})

// ─── 暴露 refresh 给父组件 ───
defineExpose({ refresh: load })
</script>

<template>
  <!-- 过滤条 -->
  <div class="flex gap-2 mb-3 flex-wrap">
    <select
      v-model="filterSource"
      class="min-h-[44px] px-3 text-xs rounded-lg border border-line bg-bg focus:outline-none focus:border-accent"
    >
      <option v-for="opt in sourceOptions" :key="opt.value" :value="opt.value">{{ opt.label }}</option>
    </select>
    <select
      v-model="filterInstanceId"
      class="min-h-[44px] px-3 text-xs rounded-lg border border-line bg-bg focus:outline-none focus:border-accent flex-1 min-w-0"
    >
      <option v-for="opt in instanceOptions" :key="opt.value" :value="opt.value">{{ opt.label }}</option>
    </select>
  </div>

  <div v-if="loading" class="text-center text-ink-soft text-sm py-8">加载中…</div>
  <div v-else-if="error" class="text-center text-warn text-sm py-8">加载失败: {{ error }}</div>
  <div v-else-if="filteredKnowledge.length === 0" class="text-center text-ink-soft text-sm py-8">
    {{ knowledge.length === 0 ? '暂无知识' : '当前过滤条件下无匹配' }}
  </div>

  <template v-else>
    <div v-for="(items, src) in groupedBySource" :key="src" class="mb-3">
      <div class="text-xs font-medium text-ink-soft mb-1.5">{{ knowledgeSourceText(src) }}</div>
      <div
        v-for="k in items" :key="k.id"
        class="bg-surface rounded-xl border border-line px-3 py-2 mb-1.5"
      >
        <div class="text-xs text-ink">{{ k.knowledge_text }}</div>
        <div class="flex items-center gap-2 mt-1">
          <span v-if="k.turn_number" class="text-[10px] text-ink-soft">轮次 {{ k.turn_number }}</span>
          <span v-if="k.character_id" class="text-[10px] text-ink-soft">实例 {{ k.character_id.slice(0, 8) }}</span>
          <span v-if="k.pinned" class="text-[10px] text-accent">📌 已固定</span>
        </div>
      </div>
    </div>
  </template>
</template>
