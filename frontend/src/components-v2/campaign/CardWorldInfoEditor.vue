<script setup>
/**
 * 角色卡模板世界书（可编辑）。
 *
 * 改的是卡模板：之后新开的活动从这里拷贝，不影响已开活动的本局世界书
 * （本局世界书在 CampaignWorldInfoTab 里改）。标记「全局」的条目切到其它角色卡
 * 也生效。
 */
import { ref, reactive, computed, watch, onMounted } from 'vue'
import {
  getCharacterWorldInfo,
  getCharacterWorldInfoEntry,
  addCharacterWorldInfoEntry,
  updateCharacterWorldInfoEntry,
  setCharacterWorldInfoRoute,
  deleteCharacterWorldInfoEntry,
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
  /** CharacterStore id 或 source_character_id */
  characterId: { type: String, default: null },
})

const loading = ref(false)
const error = ref(null)
const book = ref(null)
const routeFilter = ref('')
const query = ref('')
const editingIndex = ref(null)
const loadingEntry = ref(false)
const showAdd = ref(false)
const adding = ref(false)
const saving = ref(false)
const savingRoute = ref(false)

const addForm = reactive({ keys: '', content: '', constant: false, isGlobal: false })
const draft = reactive({
  keysText: '',
  content: '',
  constant: false,
  isGlobal: false,
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
  { value: 'Disabled', label: 'Disabled（禁用）' },
]

function splitKeys(text) {
  return text
    .split(/[,，\n]/)
    .map((k) => k.trim())
    .filter(Boolean)
}

async function load() {
  if (!props.characterId) {
    book.value = null
    return
  }
  loading.value = true
  error.value = null
  try {
    book.value = await getCharacterWorldInfo(props.characterId)
    if (editingIndex.value != null && !entries.value.some((e) => e.index === editingIndex.value)) {
      editingIndex.value = null
    }
  } catch (e) {
    error.value = errorText(e)
    book.value = null
  } finally {
    loading.value = false
  }
}

onMounted(load)
watch(
  () => props.characterId,
  () => {
    editingIndex.value = null
    showAdd.value = false
    load()
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

function fillDraft(entry) {
  draft.keysText = (entry.keys || []).join(', ')
  draft.content = entry.content || ''
  draft.constant = !!entry.constant
  draft.isGlobal = !!entry.is_global
  draft.depth = entry.depth ?? 2
  draft.order = entry.order ?? 100
  draft.route = entry.route || 'Selective'
}

// 每次打开编辑区递增；异步加载完成时只认最新一次（连点两条截断条目时，
// 前一条的结果和结束不能覆盖后一条的草稿与加载态）。
let openSeq = 0

async function openEdit(entry) {
  const seq = ++openSeq
  loadingEntry.value = false
  if (editingIndex.value === entry.index) {
    editingIndex.value = null
    return
  }
  editingIndex.value = entry.index
  fillDraft(entry)
  if (!entry.content_truncated) return
  loadingEntry.value = true
  try {
    const full = await getCharacterWorldInfoEntry(props.characterId, entry.index)
    if (seq === openSeq && full) fillDraft(full)
  } catch (e) {
    if (seq === openSeq) editingIndex.value = null
    await alertDialog('加载完整正文失败: ' + errorText(e))
  } finally {
    if (seq === openSeq) loadingEntry.value = false
  }
}

async function handleAdd() {
  if (!addForm.content.trim()) {
    await alertDialog('请填写内容')
    return
  }
  adding.value = true
  try {
    await addCharacterWorldInfoEntry({
      characterId: props.characterId,
      keys: splitKeys(addForm.keys),
      content: addForm.content.trim(),
      constant: addForm.constant,
      isGlobal: addForm.isGlobal,
    })
    Object.assign(addForm, { keys: '', content: '', constant: false, isGlobal: false })
    showAdd.value = false
    await load()
  } catch (e) {
    await alertDialog('新增失败: ' + errorText(e))
  } finally {
    adding.value = false
  }
}

async function handleSave() {
  if (editingIndex.value == null) return
  saving.value = true
  try {
    await updateCharacterWorldInfoEntry({
      characterId: props.characterId,
      entryIndex: editingIndex.value,
      keys: splitKeys(draft.keysText),
      content: draft.content,
      constant: draft.constant,
      isGlobal: draft.isGlobal,
      depth: Number(draft.depth) || 0,
      order: Number(draft.order) || 0,
    })
    await load()
  } catch (e) {
    await alertDialog('保存失败: ' + errorText(e))
  } finally {
    saving.value = false
  }
}

// 失败时用服务端真值覆盖草稿，避免下次「保存修改」时草稿与服务端不一致。
async function handleRouteChange(route) {
  if (editingIndex.value == null || savingRoute.value) return
  const previous = draft.route
  draft.route = route
  savingRoute.value = true
  try {
    await setCharacterWorldInfoRoute(props.characterId, editingIndex.value, route)
    await load()
  } catch (e) {
    draft.route = previous
    await load().catch(() => {})
    await alertDialog('改路由失败: ' + errorText(e))
  } finally {
    savingRoute.value = false
  }
}

async function handleDelete(entry) {
  const ok = await confirmDialog(
    `确定删除卡模板世界书条目 #${entry.index}？\n已开活动的本局世界书不受影响。`,
    { title: '删除世界书条目' },
  )
  if (!ok) return
  try {
    await deleteCharacterWorldInfoEntry(props.characterId, entry.index)
    if (editingIndex.value === entry.index) editingIndex.value = null
    await load()
  } catch (e) {
    await alertDialog('删除失败: ' + errorText(e))
  }
}

defineExpose({ refresh: load })
</script>

<template>
  <div class="space-y-2 min-w-0">
    <div class="space-y-2">
      <div class="flex items-center justify-between gap-3">
        <div class="min-w-0 text-xs text-ink-soft">
          卡模板世界书
          <span class="text-ink-faint">（新开的活动从这里拷贝）</span>
          <template v-if="book">
            · {{ book.entry_count }} 条 · 常驻 {{ book.constant_count }} · 触发
            {{ book.selective_count }}
          </template>
        </div>
        <div v-if="characterId" class="flex shrink-0 items-center gap-2">
          <Button size="sm" variant="primary" @click="showAdd = !showAdd">
            {{ showAdd ? '取消新增' : '新增条目' }}
          </Button>
          <Button size="sm" variant="default" @click="load">刷新</Button>
        </div>
      </div>
      <div
        v-if="entries.length"
        class="grid grid-cols-[minmax(7.5rem,9rem)_minmax(0,1fr)] gap-2"
      >
        <div class="min-w-0">
          <Select v-model="routeFilter" :options="routeOptions" />
        </div>
        <Input v-model="query" placeholder="搜索关键词或内容" />
      </div>
    </div>

    <div v-if="showAdd" class="rounded-lg border border-line bg-surface p-3 space-y-2">
      <div class="text-xs font-medium text-ink">新增卡模板条目</div>
      <Input v-model="addForm.keys" placeholder="关键词，逗号分隔（绿灯触发用）" />
      <textarea
        v-model="addForm.content"
        rows="3"
        class="w-full rounded-md border border-line bg-surface-2 px-2 py-1.5 text-xs text-ink font-mono"
        placeholder="内容"
      />
      <div class="flex flex-wrap gap-3 text-xs text-ink-soft">
        <label class="flex items-center gap-2">
          <input v-model="addForm.constant" type="checkbox" />
          常驻（Constant / 蓝灯）
        </label>
        <label class="flex items-center gap-2">
          <input v-model="addForm.isGlobal" type="checkbox" />
          全局（其它角色卡也生效）
        </label>
      </div>
      <Button size="sm" variant="primary" :loading="adding" @click="handleAdd">保存</Button>
    </div>

    <LoadingState v-if="loading && !book" />
    <div v-else-if="error" class="text-xs text-err">{{ error }}</div>
    <EmptyState
      v-else-if="!characterId"
      title="无角色卡"
      description="缺少 source_character_id，无法读取模板世界书。"
    />
    <EmptyState
      v-else-if="!entries.length"
      title="此卡无世界书条目"
      description="导入时卡内的 lorebook 会出现在这里，也可以在这里新增。"
    />
    <template v-else>
      <!-- 不加 overflow-hidden：行内编辑区的路由下拉框需要溢出容器显示 -->
      <div class="rounded-xl border border-line bg-surface">
        <article
          v-for="row in filtered"
          :key="row.index"
          data-testid="card-world-info-entry"
          class="border-b border-line p-3 first:rounded-t-xl last:rounded-b-xl last:border-b-0 transition-colors hover:bg-surface-2/45"
        >
          <header class="flex items-center gap-2">
            <Badge :variant="routeVariant(row.route)" size="sm">{{ row.route }}</Badge>
            <span v-if="row.is_global" class="text-[11px] text-accent">全局</span>
            <span v-if="row.source === 'user'" class="text-[11px] text-ink-faint">手动添加</span>
            <div class="ml-auto flex shrink-0 items-center gap-1 whitespace-nowrap">
              <Button size="sm" variant="ghost" @click="openEdit(row)">
                {{ editingIndex === row.index ? '收起' : '编辑' }}
              </Button>
              <Button size="sm" variant="danger" @click="handleDelete(row)">删除</Button>
            </div>
          </header>
          <div class="mt-1.5 text-xs font-medium leading-relaxed text-ink break-words">
            {{ (row.keys || []).join(' · ') || '无关键词' }}
          </div>
          <button
            type="button"
            class="mt-1 w-full text-left text-xs leading-relaxed text-ink-soft break-words line-clamp-2 transition-colors hover:text-accent"
            @click="openEdit(row)"
          >
            {{ row.content }}
          </button>

          <div
            v-if="editingIndex === row.index"
            class="mt-2 rounded-lg border border-line bg-surface-2/50 p-3 space-y-2"
          >
            <div class="text-xs text-ink-soft">编辑条目 #{{ row.index }}</div>
            <LoadingState v-if="loadingEntry" label="加载完整正文…" />
            <template v-else>
              <Input v-model="draft.keysText" placeholder="关键词，逗号分隔" />
              <textarea
                v-model="draft.content"
                rows="5"
                class="w-full rounded-md border border-line bg-surface px-2 py-1.5 text-xs text-ink font-mono"
              />
              <div class="flex flex-wrap gap-3 items-center text-xs">
                <label class="flex items-center gap-1">
                  <input v-model="draft.constant" type="checkbox" />
                  常驻
                </label>
                <label class="flex items-center gap-1">
                  <input v-model="draft.isGlobal" type="checkbox" />
                  全局
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
                <div class="min-w-[140px]">
                  <Select
                    :model-value="draft.route"
                    :options="routeSetOptions"
                    :disabled="savingRoute"
                    @update:model-value="handleRouteChange"
                  />
                </div>
                <Button size="sm" variant="primary" :loading="saving" @click="handleSave">
                  保存修改
                </Button>
              </div>
            </template>
          </div>
        </article>
      </div>
    </template>
  </div>
</template>
