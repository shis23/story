<script setup>
import { ref } from 'vue'
import {
  metaHealthCheck,
  metaProposeCampaignRepairs,
  metaPreviewTypedPatch,
  metaListTypedPatches,
  metaAcceptTypedPatch,
  metaDismissTypedPatch,
} from '../../tauri-api.js'
import {
  sortHealthIssues,
  proposeRepairsFlow,
  refreshTypedPatchesFlow,
  acceptTypedPatchFlow,
  dismissTypedPatchFlow,
} from '../../utils/metaPanelFlow.js'
import { formatDiffValue } from '../../utils/campaignDisplay.js'
import Button from '../ui/Button.vue'
import Badge from '../ui/Badge.vue'
import EmptyState from '../ui/EmptyState.vue'
import LoadingState from '../ui/LoadingState.vue'

// Campaign 健康检查 + 类型化修复建议（第三轮闭环）。
// 原始 MetaPanel 右侧栏的「🩺 Campaign 体检」+「🔧 修复建议」两个区块合并。
// 复用 utils/metaPanelFlow.js 的 propose/refresh/accept/dismiss flow。

const props = defineProps({
  activeCampaign: { type: Object, default: null },
})

const emit = defineEmits(['error'])

// ─── 体检状态 ───
const healthIssues = ref([]) // HealthIssue[]
const healthLoading = ref(false)
const healthRan = ref(false) // 区分「未检查」与「检查后无问题」

// ─── 类型化修复建议 ───
const typedPatches = ref([]) // TypedPatch[]
const patchesLoading = ref(false)
const expandedPatchId = ref(null) // 展开的 typed patch id

const error = ref('')

// ─── 体检 ───
async function handleHealthCheck() {
  if (!props.activeCampaign?.id) return
  healthLoading.value = true
  error.value = ''
  try {
    const result = await metaHealthCheck(props.activeCampaign.id)
    healthIssues.value = sortHealthIssues(result)
    healthRan.value = true
  } catch (e) {
    error.value = '体检失败: ' + e
    emit('error', error.value)
  } finally {
    healthLoading.value = false
  }
}

// ─── 生成修复方案 ───
async function handleProposeRepairs() {
  if (!props.activeCampaign?.id) return
  patchesLoading.value = true
  error.value = ''
  try {
    typedPatches.value = await proposeRepairsFlow({
      campaignId: props.activeCampaign.id,
      proposeCampaignRepairs: metaProposeCampaignRepairs,
      previewTypedPatch: metaPreviewTypedPatch,
    })
  } catch (e) {
    error.value = '生成修复方案失败: ' + e
    emit('error', error.value)
  } finally {
    patchesLoading.value = false
  }
}

// 父组件可在 chat 触发 new_typed_patches 后调用刷新
async function refreshTypedPatches() {
  try {
    typedPatches.value = await refreshTypedPatchesFlow({
      campaignId: props.activeCampaign?.id,
      listTypedPatches: metaListTypedPatches,
      previewTypedPatch: metaPreviewTypedPatch,
    })
  } catch (e) {
    // 静默失败，不影响主流程
    console.warn('刷新 typed patches 失败:', e)
  }
}

async function handleAcceptTypedPatch(patchId) {
  if (!props.activeCampaign?.id) return
  error.value = ''
  try {
    const result = await acceptTypedPatchFlow({
      campaignId: props.activeCampaign.id,
      patchId,
      patches: typedPatches.value,
      acceptTypedPatch: metaAcceptTypedPatch,
      refreshHealth: async () => {
        const issues = await metaHealthCheck(props.activeCampaign.id)
        return sortHealthIssues(issues)
      },
    })
    typedPatches.value = result.patches
    if (result.healthIssues) {
      healthIssues.value = result.healthIssues
      healthRan.value = true
    }
    if (result.healthError) {
      error.value = '体检失败: ' + result.healthError
      emit('error', error.value)
    }
  } catch (e) {
    error.value = '接受修复失败: ' + e
    emit('error', error.value)
  }
}

async function handleDismissTypedPatch(patchId) {
  try {
    typedPatches.value = await dismissTypedPatchFlow({
      patchId,
      patches: typedPatches.value,
      dismissTypedPatch: metaDismissTypedPatch,
    })
  } catch (e) {
    error.value = '忽略修复失败: ' + e
    emit('error', error.value)
  }
}

function togglePatch(patchId) {
  expandedPatchId.value = expandedPatchId.value === patchId ? null : patchId
}

defineExpose({ refreshTypedPatches })
</script>

<template>
  <div class="space-y-5">
    <!-- 🩺 体检区 -->
    <section class="space-y-2">
      <div class="flex items-center justify-between">
        <h3 class="text-sm font-semibold text-ink">Campaign 体检</h3>
        <Button
          variant="primary"
          size="sm"
          :loading="healthLoading"
          :disabled="!activeCampaign || healthLoading"
          @click="handleHealthCheck"
        >{{ healthLoading ? '检查中…' : '运行体检' }}</Button>
      </div>

      <div v-if="error" class="text-xs text-err">{{ error }}</div>

      <EmptyState
        v-if="!activeCampaign"
        title="无活跃 Campaign"
        description="请先在 Campaign 面板选择一个游玩档"
      />
      <div v-else-if="!healthRan && !healthLoading" class="text-xs text-ink-soft">
        点击「运行体检」检查数据完整性。
      </div>
      <LoadingState v-else-if="healthLoading" />
      <EmptyState
        v-else-if="healthRan && healthIssues.length === 0"
        title="未发现问题"
        description="数据完整性校验通过"
      />
      <div v-else class="space-y-1.5">
        <div
          v-for="(issue, i) in healthIssues"
          :key="issue.category + '-' + i"
          class="rounded-lg border p-2.5 text-xs"
          :class="issue.severity === 'error'
            ? 'border-err/30 bg-err/5'
            : 'border-warn/30 bg-warn/5'"
        >
          <div class="flex items-center gap-1.5 mb-1">
            <Badge :variant="issue.severity === 'error' ? 'err' : 'warn'" size="sm">
              {{ issue.severity === 'error' ? 'Error' : 'Warning' }}
            </Badge>
            <span class="text-ink-soft text-[11px]">{{ issue.category }}</span>
          </div>
          <div class="text-ink">{{ issue.message }}</div>
          <div v-if="issue.affected_id" class="text-ink-soft mt-1 break-all text-[11px]">
            ID: {{ issue.affected_id }}
          </div>
        </div>
      </div>
    </section>

    <!-- 🔧 修复建议区 -->
    <section class="space-y-2">
      <div class="flex items-center justify-between">
        <h3 class="text-sm font-semibold text-ink">修复建议</h3>
        <Button
          v-if="healthRan && healthIssues.length > 0"
          variant="primary"
          size="sm"
          :loading="patchesLoading"
          :disabled="!activeCampaign || patchesLoading"
          @click="handleProposeRepairs"
        >{{ patchesLoading ? '生成中…' : '生成修复方案' }}</Button>
      </div>

      <div v-if="!healthRan || healthIssues.length === 0" class="text-xs text-ink-soft">
        先运行体检，有问题时可生成修复方案。
      </div>
      <div v-else-if="typedPatches.length === 0 && !patchesLoading" class="text-xs text-ink-soft">
        点击「生成修复方案」获取类型化 patch。
      </div>

      <div v-else class="space-y-2">
        <div
          v-for="patch in typedPatches"
          :key="patch.id"
          class="bg-surface rounded-lg border p-2.5 text-xs"
          :class="patch._stale ? 'border-warn/30 opacity-70' : 'border-line'"
        >
          <div class="flex items-center gap-1.5 mb-1.5 flex-wrap">
            <Badge variant="accent" size="sm">{{ patch.source_issue_category }}</Badge>
            <Badge v-if="patch._stale" variant="warn" size="sm">已过期</Badge>
          </div>
          <div class="text-ink font-medium mb-1">{{ patch.description }}</div>
          <div v-if="patch.affected_id" class="text-ink-soft mb-1 break-all text-[11px]">
            ID: {{ patch.affected_id }}
          </div>

          <!-- diff 展开区 -->
          <button
            type="button"
            class="text-ink-soft underline hover:text-ink text-[11px]"
            @click="togglePatch(patch.id)"
          >{{ expandedPatchId === patch.id ? '收起 diff' : '查看 diff' }}</button>
          <div
            v-if="expandedPatchId === patch.id && patch.diff && patch.diff.length > 0"
            class="mt-1.5 space-y-1.5"
          >
            <div
              v-for="(d, di) in patch.diff"
              :key="di"
              class="bg-surface-2 rounded px-2 py-1.5 border border-line"
            >
              <div class="text-ink-soft font-medium mb-1 text-[11px]">{{ d.path }}</div>
              <div class="grid grid-cols-[1fr_auto_1fr] gap-1 items-start">
                <div class="min-w-0">
                  <div class="text-[10px] text-ink-soft mb-0.5">Before</div>
                  <pre class="text-[10px] overflow-x-auto text-err/80 whitespace-pre-wrap break-all">{{ formatDiffValue(d.before) }}</pre>
                </div>
                <div class="text-ink-soft px-1 pt-3">→</div>
                <div class="min-w-0">
                  <div class="text-[10px] text-ink-soft mb-0.5">After</div>
                  <pre class="text-[10px] overflow-x-auto text-ok/80 whitespace-pre-wrap break-all">{{ formatDiffValue(d.after) }}</pre>
                </div>
              </div>
            </div>
          </div>

          <!-- 操作 -->
          <div class="flex gap-1.5 mt-2">
            <Button
              variant="default"
              size="sm"
              class="flex-1"
              :disabled="patch._stale"
              @click="handleAcceptTypedPatch(patch.id)"
            >接受</Button>
            <Button
              variant="ghost"
              size="sm"
              class="flex-1"
              @click="handleDismissTypedPatch(patch.id)"
            >忽略</Button>
          </div>
        </div>
      </div>
    </section>
  </div>
</template>
