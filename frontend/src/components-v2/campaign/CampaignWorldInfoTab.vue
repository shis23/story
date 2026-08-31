<script setup>
/**
 * 本局世界书（Campaign 可读写）。
 * 卡模板世界书只读请用 getCharacterWorldInfo。
 */
import { ref, computed, onMounted, watch, reactive } from 'vue'
import {
  listCampaignWorldInfo,
  addCampaignWorldInfoEntry,
  updateCampaignWorldInfoEntry,
  deleteCampaignWorldInfoEntry,
  setCampaignWorldInfoRoute,
  getCampaignWorldInfoEntry,
} from '../../tauri-api.js'
import { confirmDialog, alertDialog } from '../../components/base/BaseDialog.js'
import Badge from '../ui/Badge.vue'
import Button from '../ui/Button.vue'
import Select from '../ui/Select.vue'
import Input from '../ui/Input.vue'
import EmptyState from '../ui/EmptyState.vue'
import LoadingState from '../ui/LoadingState.vue'
import { errorText } from '../../utils/errorText.js'

const props = defineProps({
  campaignId: { type: String, required: true },
})

const loading = ref(false)
const error = ref(null)
const book = ref(null)
const routeFilter = ref('')
const query = ref('')
const expandedIndex = ref(null)
const showAdd = ref(false)
const adding = ref(false)
const saving = ref(false)
const form = ref({
  keys: '',
  content: '',
  constant: false,
  depth: 2,
  order: 100,
})
const draft = reactive({
  keysText: '',
  content: '',
  constant: false,
  disabled: false,
  depth: 2,
  order: 100,
  route: 'Selective',
})

const routeOptions = [
  { value: '', label: '全部路由' },
  { value: 'Constant', label: '常驻 Constant' },
  { value: 'Selective', label: '触发 Selective' },
  { value: 'Both', label: 'Both' },
  { value: 'Disabled', label: 'Disabled' },
]

const routeSetOptions = [
  { value: 'Constant', label: 'Constant' },
  { value: 'Selective', label: 'Selective' },
  { value: 'Both', label: 'Both' },
  { value: 'Disabled', label: 'Disabled' },
]

async function load() {
  if (!props.campaignId) return
  loading.value = true
  error.value = null
  try {
    book.value = await listCampaignWorldInfo(props.campaignId)
    if (expandedIndex.value != null) {
      const entry = (book.value?.entries || []).find((e) => e.index === expandedIndex.value)
      if (entry) fillDraft(entry)
      else expandedIndex.value = null
    }
  } catch (e) {
    error.value = errorText(e)
  } finally {
    loading.value = false
  }
}

function fillDraft(entry) {
  draft.keysText = (entry.keys || []).join(', ')
  draft.content = entry.content || ''
  draft.constant = !!entry.constant
  draft.disabled = !!entry.disabled
  draft.depth = entry.depth ?? 2
  draft.order = entry.order ?? 100
  draft.route = entry.route || 'Selective'
}

onMounted(load)
watch(
  () => props.campaignId,
  () => {
    expandedIndex.value = null
    if (props.campaignId) load()
  },
)

const entries = computed(() => book.value?.entries || [])

const filtered = computed(() => {
  const q = query.value.trim().toLowerCase()
  return entries.value.filter((e) => {
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

function sourceLabel(s) {
  if (s === 'user') return '本局'
  if (s === 'merged_global') return '全局'
  return '卡模板'
}

async function openEdit(entry) {
  if (expandedIndex.value === entry.index) {
    expandedIndex.value = null
    return
  }
  expandedIndex.value = entry.index
  let full = entry
  if (entry.content_truncated) {
    try {
      const dto = await getCampaignWorldInfoEntry(props.campaignId, entry.index)
      if (dto) full = { ...entry, ...dto, content_truncated: false }
    } catch (e) {
      await alertDialog('加载完整正文失败: ' + errorText(e))
    }
  }
  fillDraft(full)
}

async function handleAdd() {
  const keys = form.value.keys
    .split(/[,，\n]/)
    .map((k) => k.trim())
    .filter(Boolean)
  if (!form.value.content.trim()) {
    await alertDialog('请填写内容')
    return
  }
  adding.value = true
  try {
    await addCampaignWorldInfoEntry({
      campaignId: props.campaignId,
      keys,
      content: form.value.content.trim(),
      constant: form.value.constant,
      depth: Number(form.value.depth) || 2,
      order: Number(form.value.order) || 100,
    })
    form.value = { keys: '', content: '', constant: false, depth: 2, order: 100 }
    showAdd.value = false
    await load()
  } catch (e) {
    await alertDialog('新增失败: ' + errorText(e))
  } finally {
    adding.value = false
  }
}

async function handleRouteChange(route) {
  if (expandedIndex.value == null) return
  draft.route = route
  try {
    await setCampaignWorldInfoRoute(props.campaignId, expandedIndex.value, route)
    await load()
  } catch (e) {
    await alertDialog('改路由失败: ' + errorText(e))
  }
}

async function handleDelete(entry) {
  const ok = await confirmDialog(
    `确定删除本局世界书条目 #${entry.index}？\n（不会修改角色卡模板）`,
    { title: '删除世界书条目' },
  )
  if (!ok) return
  try {
    await deleteCampaignWorldInfoEntry(props.campaignId, entry.index)
    if (expandedIndex.value === entry.index) expandedIndex.value = null
    await load()
  } catch (e) {
    await alertDialog('删除失败: ' + errorText(e))
  }
}

async function handleSave() {
  if (expandedIndex.value == null) return
  saving.value = true
  try {
    await updateCampaignWorldInfoEntry({
      campaignId: props.campaignId,
      entryIndex: expandedIndex.value,
      keys: draft.keysText
        .split(/[,，]/)
        .map((k) => k.trim())
        .filter(Boolean),
      content: draft.content,
      constant: draft.constant,
      disabled: draft.disabled,
      depth: Number(draft.depth) || 2,
      order: Number(draft.order) || 100,
      route: draft.route,
    })
    await load()
  } catch (e) {
    await alertDialog('保存失败: ' + errorText(e))
  } finally {
    saving.value = false
  }
}

defineExpose({ refresh: load })
</script>

<template>
  <div class="space-y-3 min-w-0">
    <div class="space-y-2">
      <div class="flex items-center justify-between gap-3">
        <div class="min-w-0 text-xs text-ink-soft">
          本局世界书
          <template v-if="book">
            · {{ book.entry_count }} 条 · 常驻 {{ book.constant_count }} · 触发
            {{ book.selective_count }}
          </template>
        </div>
        <div class="flex shrink-0 items-center gap-2">
          <Button size="sm" variant="primary" @click="showAdd = !showAdd">
            {{ showAdd ? '取消新增' : '新增条目' }}
          </Button>
          <Button size="sm" variant="default" @click="load">刷新</Button>
        </div>
      </div>
      <div class="grid grid-cols-[minmax(7.5rem,9rem)_minmax(0,1fr)] gap-2">
        <div class="min-w-0">
          <Select v-model="routeFilter" :options="routeOptions" />
        </div>
        <Input v-model="query" placeholder="搜索关键词或内容" />
      </div>
    </div>

    <div v-if="showAdd" class="rounded-lg border border-line bg-surface p-3 space-y-2">
      <div class="text-xs font-medium text-ink">新增本局条目（不写回角色卡）</div>
      <Input v-model="form.keys" placeholder="关键词，逗号分隔（绿灯触发用）" />
      <textarea
        v-model="form.content"
        rows="3"
        class="w-full rounded-md border border-line bg-surface-2 px-2 py-1.5 text-xs text-ink font-mono"
        placeholder="内容"
      />
      <label class="flex items-center gap-2 text-xs text-ink-soft">
        <input v-model="form.constant" type="checkbox" />
        常驻（Constant / 蓝灯）
      </label>
      <Button size="sm" variant="primary" :loading="adding" @click="handleAdd">保存</Button>
    </div>

    <LoadingState v-if="loading && !book" />
    <div v-else-if="error" class="text-xs text-err">{{ error }}</div>
    <EmptyState
      v-else-if="!filtered.length"
      title="暂无世界书条目"
      description="开档会从角色卡拷贝模板；也可在此为本局新增设定。"
    />
    <template v-else>
      <div class="overflow-hidden rounded-xl border border-line bg-surface">
        <article
          v-for="row in filtered"
          :key="row.index"
          data-testid="world-info-entry"
          class="border-b border-line p-3.5 last:border-b-0 transition-colors hover:bg-surface-2/45"
        >
          <header class="flex items-center gap-2">
            <Badge :variant="routeVariant(row.route)" size="sm">{{ row.route }}</Badge>
            <span class="text-[11px] text-ink-faint">{{ sourceLabel(row.source) }}</span>
            <div
              data-testid="world-info-entry-actions"
              class="ml-auto flex shrink-0 items-center gap-1 whitespace-nowrap"
            >
              <Button size="sm" variant="ghost" @click="openEdit(row)">
                {{ expandedIndex === row.index ? '收起' : '编辑' }}
              </Button>
              <Button size="sm" variant="danger" @click="handleDelete(row)">删除</Button>
            </div>
          </header>
          <div class="mt-2 text-xs font-medium leading-relaxed text-ink break-words">
            {{ (row.keys || []).join(' · ') || '无关键词' }}
          </div>
          <button
            type="button"
            class="mt-1.5 w-full text-left text-xs leading-relaxed text-ink-soft break-words line-clamp-2 transition-colors hover:text-accent"
            @click="openEdit(row)"
          >
            {{ row.content }}
          </button>
        </article>
      </div>

      <div
        v-if="expandedIndex != null"
        class="rounded-lg border border-line bg-surface-2/50 p-3 space-y-2"
      >
        <div class="text-xs text-ink-soft">编辑条目 #{{ expandedIndex }}</div>
        <Input v-model="draft.keysText" placeholder="关键词" />
        <textarea
          v-model="draft.content"
          rows="4"
          class="w-full rounded-md border border-line bg-surface px-2 py-1.5 text-xs text-ink font-mono"
        />
        <div class="flex flex-wrap gap-3 items-center text-xs">
          <label class="flex items-center gap-1">
            <input v-model="draft.constant" type="checkbox" />
            常驻
          </label>
          <label class="flex items-center gap-1">
            <input v-model="draft.disabled" type="checkbox" />
            禁用
          </label>
          <span>
            depth
            <input
              v-model.number="draft.depth"
              type="number"
              class="w-16 border border-line rounded px-1 ml-1"
            />
          </span>
          <span>
            order
            <input
              v-model.number="draft.order"
              type="number"
              class="w-16 border border-line rounded px-1 ml-1"
            />
          </span>
          <div class="min-w-[120px]">
            <Select
              v-model="draft.route"
              :options="routeSetOptions"
              @update:model-value="handleRouteChange"
            />
          </div>
          <Button size="sm" variant="primary" :loading="saving" @click="handleSave">
            保存修改
          </Button>
        </div>
      </div>
    </template>
  </div>
</template>
