<script setup>
import { ref, onMounted } from 'vue'
import {
  metaListPendingPatches,
  metaAcceptPatch,
  metaDismissPatch,
} from '../../tauri-api.js'
import Button from '../ui/Button.vue'
import Badge from '../ui/Badge.vue'
import EmptyState from '../ui/EmptyState.vue'

// 待采纳的（非类型化）Patch 列表：meta_list_pending_patches / meta_accept_patch / meta_dismiss_patch。
// 原始 MetaPanel 右侧栏「📝 待采纳 Patch」区块迁移。
// patch.actions 为 Create/Update/Delete 结构化操作，用列表形式渲染（非 before/after diff）。

const emit = defineEmits(['error', 'patch-count-change'])

// ─── 状态 ───
const patches = ref([]) // Patch[]
const expandedId = ref(null)
const error = ref('')

async function load() {
  try {
    patches.value = await metaListPendingPatches()
    emit('patch-count-change', patches.value.length)
  } catch (e) {
    error.value = '加载待采纳 Patch 失败: ' + e
    emit('error', error.value)
  }
}

onMounted(load)

async function handleAccept(patchId) {
  error.value = ''
  try {
    await metaAcceptPatch(patchId)
    await load()
  } catch (e) {
    error.value = '采纳失败: ' + e
    emit('error', error.value)
  }
}

async function handleDismiss(patchId) {
  error.value = ''
  try {
    await metaDismissPatch(patchId)
    await load()
  } catch (e) {
    error.value = '忽略失败: ' + e
    emit('error', error.value)
  }
}

function toggle(patchId) {
  expandedId.value = expandedId.value === patchId ? null : patchId
}

// patch actions 摘要（沿用原 MetaPanel.patchActionSummary 逻辑）
function actionSummary(patch) {
  if (!patch.actions || patch.actions.length === 0) return []
  return patch.actions.map(a => {
    if (a.Create) return { op: 'create', label: `创建 ${a.Create.target}` }
    if (a.Update) return { op: 'update', label: `改 ${a.Update.target}.${a.Update.field}` }
    if (a.Delete) return { op: 'delete', label: `删 ${a.Delete.target}` }
    return { op: 'other', label: JSON.stringify(a) }
  })
}

const opVariant = { create: 'ok', update: 'accent', delete: 'err', other: 'neutral' }

defineExpose({ refresh: load })
</script>

<template>
  <div class="space-y-3">
    <div v-if="error" class="text-xs text-err">{{ error }}</div>

    <EmptyState
      v-if="patches.length === 0"
      title="无待采纳 Patch"
      description="与 Meta 助手对话后，提议的补丁会出现在这里"
    />

    <div v-else class="space-y-2">
      <div
        v-for="patch in patches"
        :key="patch.id"
        class="bg-surface rounded-lg border border-line p-3"
      >
        <div class="flex items-start justify-between gap-2 mb-1">
          <div class="text-sm font-medium text-ink">{{ patch.description }}</div>
          <Badge variant="accent" size="sm">{{ (patch.actions || []).length }} 个操作</Badge>
        </div>

        <button
          type="button"
          class="text-xs text-ink-soft underline hover:text-ink"
          @click="toggle(patch.id)"
        >{{ expandedId === patch.id ? '收起操作' : '查看操作' }}</button>

        <div v-if="expandedId === patch.id" class="mt-2 space-y-1">
          <div
            v-for="(a, ai) in actionSummary(patch)"
            :key="ai"
            class="flex items-center gap-1.5 text-[11px]"
          >
            <Badge :variant="opVariant[a.op] || 'neutral'" size="sm">{{ a.op }}</Badge>
            <span class="text-ink-soft break-all">{{ a.label }}</span>
          </div>
          <div v-if="actionSummary(patch).length === 0" class="text-[11px] text-ink-soft">（无操作）</div>
        </div>

        <div class="flex gap-1.5 mt-2.5">
          <Button
            variant="primary"
            size="sm"
            class="flex-1"
            @click="handleAccept(patch.id)"
          >采纳</Button>
          <Button
            variant="ghost"
            size="sm"
            class="flex-1"
            @click="handleDismiss(patch.id)"
          >忽略</Button>
        </div>
      </div>
    </div>
  </div>
</template>
