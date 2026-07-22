<script setup>
/**
 * 角色卡世界书只读面板（模板）。
 * 可写路径在 CampaignWorldInfoTab（本局真相源）。
 */
import { ref, computed, watch, onMounted } from 'vue'
import { getCharacterWorldInfo, getCharacterWorldInfoEntry } from '../../tauri-api.js'
import DataTable from '../ui/DataTable.vue'
import Badge from '../ui/Badge.vue'
import Button from '../ui/Button.vue'
import Select from '../ui/Select.vue'
import Input from '../ui/Input.vue'
import EmptyState from '../ui/EmptyState.vue'
import LoadingState from '../ui/LoadingState.vue'

const props = defineProps({
  /** CharacterStore id（source_character_id） */
  characterId: { type: String, default: null },
})

const loading = ref(false)
const error = ref(null)
const book = ref(null)
const routeFilter = ref('')
const query = ref('')
const expandedIndex = ref(null)
const expandedContent = ref('')
const expanding = ref(false)

const routeOptions = [
  { value: '', label: '全部路由' },
  { value: 'Constant', label: '常驻 Constant' },
  { value: 'Selective', label: '触发 Selective' },
  { value: 'Both', label: 'Both' },
  { value: 'Disabled', label: 'Disabled' },
]

const columns = [
  { key: 'route', label: '路由', width: '100px' },
  { key: 'keys', label: '关键词', width: '140px' },
  { key: 'content', label: '内容' },
]

async function load() {
  if (!props.characterId) {
    book.value = null
    return
  }
  loading.value = true
  error.value = null
  try {
    book.value = await getCharacterWorldInfo(props.characterId)
  } catch (e) {
    error.value = String(e)
    book.value = null
  } finally {
    loading.value = false
  }
}

onMounted(load)
watch(
  () => props.characterId,
  () => {
    expandedIndex.value = null
    load()
  },
)

const filtered = computed(() => {
  const entries = book.value?.entries || []
  const q = query.value.trim().toLowerCase()
  return entries.filter((e) => {
    if (routeFilter.value && e.route !== routeFilter.value) return false
    if (!q) return true
    const hay = `${(e.keys || []).join(' ')} ${e.content || ''}`.toLowerCase()
    return hay.includes(q)
  })
})

function routeVariant(route) {
  if (route === 'Constant' || route === 'Both') return 'ok'
  if (route === 'Selective') return 'accent'
  return 'neutral'
}

async function toggleExpand(entry) {
  if (expandedIndex.value === entry.index) {
    expandedIndex.value = null
    expandedContent.value = ''
    return
  }
  expandedIndex.value = entry.index
  expandedContent.value = entry.content || ''
  if (entry.content_truncated && props.characterId != null) {
    expanding.value = true
    try {
      const full = await getCharacterWorldInfoEntry(props.characterId, entry.index)
      if (expandedIndex.value === entry.index && full) {
        expandedContent.value = full.content || ''
      }
    } catch (e) {
      expandedContent.value = `${entry.content || ''}\n\n[完整正文加载失败] ${e}`
    } finally {
      expanding.value = false
    }
  }
}

defineExpose({ refresh: load })
</script>

<template>
  <div class="space-y-2 min-w-0">
    <div class="flex flex-wrap items-center gap-2">
      <div class="text-xs text-ink-soft">
        卡模板世界书
        <span class="text-ink-faint">（只读 · 游玩请改活动世界书）</span>
        <template v-if="book">
          · {{ book.entry_count }} 条 · 常驻 {{ book.constant_count }} · 触发
          {{ book.selective_count }}
        </template>
      </div>
      <div class="ml-auto flex flex-wrap gap-2">
        <div class="min-w-[120px]">
          <Select v-model="routeFilter" :options="routeOptions" />
        </div>
        <Input v-model="query" placeholder="搜索 keys / 内容" class="w-40" />
        <Button size="sm" variant="default" @click="load">刷新</Button>
      </div>
    </div>

    <LoadingState v-if="loading && !book" />
    <div v-else-if="error" class="text-xs text-err">{{ error }}</div>
    <EmptyState
      v-else-if="!characterId"
      title="无角色卡"
      description="缺少 source_character_id，无法读取模板世界书。"
    />
    <EmptyState
      v-else-if="!(book?.entries || []).length"
      title="此卡无世界书条目"
      description="导入后若卡内含 lorebook，会显示在这里。"
    />
    <template v-else>
      <DataTable :columns="columns" :rows="filtered" empty-title="无匹配">
        <template #cell-route="{ row }">
          <Badge :variant="routeVariant(row.route)" size="sm">{{ row.route }}</Badge>
        </template>
        <template #cell-keys="{ row }">
          <span class="text-xs text-ink break-words">{{
            (row.keys || []).join(', ') || '—'
          }}</span>
        </template>
        <template #cell-content="{ row }">
          <button
            type="button"
            class="text-left text-xs text-ink break-words line-clamp-2 hover:text-accent w-full"
            @click="toggleExpand(row)"
          >
            {{ row.content }}
          </button>
        </template>
      </DataTable>
      <div
        v-if="expandedIndex != null"
        class="rounded-lg border border-line bg-surface-2/50 p-3 text-xs text-ink font-mono whitespace-pre-wrap break-words"
      >
        <div class="text-ink-soft mb-1 font-sans">条目 #{{ expandedIndex }}（只读）</div>
        <span v-if="expanding" class="text-ink-faint font-sans">加载完整正文…</span>
        <template v-else>{{ expandedContent }}</template>
      </div>
    </template>
  </div>
</template>
