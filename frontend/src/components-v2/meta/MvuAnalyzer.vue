<script setup>
import { ref, onMounted, computed } from 'vue'
import {
  listCards,
  metaAnalyzeMvuCard,
  metaListMvuTranslations,
  metaPreviewMvuApply,
  metaApplyMvuSchema,
} from '../../tauri-api.js'
import { routingText } from '../../utils/campaignDisplay.js'
import Button from '../ui/Button.vue'
import Badge from '../ui/Badge.vue'
import Select from '../ui/Select.vue'
import EmptyState from '../ui/EmptyState.vue'
import LoadingState from '../ui/LoadingState.vue'
import Overlay from '../ui/Overlay.vue'

// MVU 卡片分析：选卡 → meta_analyze_mvu_card → 显示翻译列表 → 预览/应用 schema。
// 成功 apply 后 emit('mvu-applied')，由 MetaPanel 冒泡到 AppV2 触发 CampaignPanel 刷新。

const emit = defineEmits(['error', 'mvu-applied'])

// ─── 状态 ───
// 用 list_cards（Campaign 卡库）而不是 list_characters（导入层 CharacterStore）：
// 后者可能含已删卡/重复导入残留，且 id 语义与 meta_analyze 的 source_character_id 不一致。
const cards = ref([])
const selectedCardId = ref(null)
const analyzingCardId = ref(null) // 正在分析的 source_character_id

const mvuTranslations = ref([]) // 已分析的 MVU 翻译列表
const activeDetail = ref(null) // 展开的 MVU 分析详情

// Apply 预览流程
const applyPreviewLoading = ref(false)
const applyPreviews = ref([]) // MvuApplyPreview[]
const applyPreviewSource = ref(null) // { id, name } — 正在预览的 source character
const applyingDefId = ref(null) // 正在 apply 的 definition_id

const error = ref('')

// ─── 角色卡 Select：value = source_character_id（MVU 分析入参），按 source 去重 ───
const cardOptions = computed(() => {
  const seen = new Set()
  const opts = []
  for (const c of cards.value) {
    const sourceId = c.source_character_id || c.id
    if (!sourceId || seen.has(sourceId)) continue
    seen.add(sourceId)
    opts.push({
      value: sourceId,
      label: c.name || sourceId.slice(0, 8),
    })
  }
  return opts
})

// ─── 初始化 ───
onMounted(async () => {
  try {
    cards.value = await listCards()
  } catch (e) {
    console.error('加载角色卡失败:', e)
  }
  await refreshMvuList()
})

async function refreshMvuList() {
  try {
    mvuTranslations.value = await metaListMvuTranslations()
  } catch (e) {
    console.error('加载 MVU 列表失败:', e)
  }
}

// ─── 选卡触发分析 ───
async function onCardSelected(cardId) {
  selectedCardId.value = cardId
  if (cardId) await handleAnalyze(cardId)
}

// ─── 触发 MVU 分析 ───
async function handleAnalyze(cardId) {
  if (!cardId) return
  analyzingCardId.value = cardId
  error.value = ''
  try {
    const detail = await metaAnalyzeMvuCard(cardId)
    activeDetail.value = detail
    await refreshMvuList()
  } catch (e) {
    error.value = 'MVU 分析失败: ' + e
    emit('error', error.value)
  } finally {
    analyzingCardId.value = null
    selectedCardId.value = null
  }
}

// ─── MVU Apply 预览 ───
async function handlePreviewApply(sourceId, sourceName) {
  applyPreviewLoading.value = true
  applyPreviews.value = []
  applyPreviewSource.value = { id: sourceId, name: sourceName }
  error.value = ''
  try {
    const previews = await metaPreviewMvuApply(sourceId)
    applyPreviews.value = previews || []
  } catch (e) {
    error.value = 'MVU apply 预览失败: ' + e
    emit('error', error.value)
    applyPreviewSource.value = null
  } finally {
    applyPreviewLoading.value = false
  }
}

// ─── MVU Apply 执行（成功后 emit mvu-applied）───
async function handleApplySchema(definitionId) {
  if (!applyPreviewSource.value) return
  applyingDefId.value = definitionId
  error.value = ''
  try {
    await metaApplyMvuSchema(applyPreviewSource.value.id, definitionId)
    // 从预览列表中移除已应用的 definition
    applyPreviews.value = applyPreviews.value.filter(p => p.definition_id !== definitionId)
    // 通知父组件刷新 Campaign 变量 —— 触发 refreshActiveDetailTab 链
    emit('mvu-applied')
  } catch (e) {
    error.value = '应用 schema 失败: ' + e
    emit('error', error.value)
  } finally {
    applyingDefId.value = null
  }
}

function closeApplyPreview() {
  applyPreviews.value = []
  applyPreviewSource.value = null
}

// 工具：格式化 VariableField 摘要
function fieldSummary(field) {
  const type = field.value_type ? (typeof field.value_type === 'string' ? field.value_type : JSON.stringify(field.value_type)) : '?'
  const def = field.default !== undefined && field.default !== null ? JSON.stringify(field.default) : ''
  return `${field.key} (${field.label}) — ${type}${def ? ' = ' + def : ''}`
}

defineExpose({ refreshMvuList })
</script>

<template>
  <div class="space-y-4">
    <!-- 选卡分析入口 -->
    <section class="bg-surface rounded-lg border border-line p-3 space-y-2">
      <div class="text-sm font-semibold text-ink">分析状态栏 (MVU)</div>
      <div class="text-xs text-ink-soft">选择一张角色卡触发五合一分析。</div>
      <Select
        v-model="selectedCardId"
        :options="cardOptions"
        placeholder="选择角色卡…"
        :loading="analyzingCardId !== null"
        @update:model-value="onCardSelected"
      />
      <div v-if="analyzingCardId" class="text-xs text-ink-soft">
        <LoadingState compact label="分析中…" />
      </div>
      <div v-if="error" class="text-xs text-err">{{ error }}</div>
    </section>

    <!-- 已分析的 MVU 翻译列表 -->
    <section class="space-y-2">
      <h3 class="text-sm font-semibold text-ink">已分析的 MVU</h3>

      <EmptyState
        v-if="mvuTranslations.length === 0"
        title="尚无分析结果"
        description="选择上方角色卡触发分析"
      />

      <div v-else class="space-y-2">
        <div
          v-for="m in mvuTranslations"
          :key="m.source_character_id"
          class="bg-surface rounded-lg border border-line p-3"
        >
          <div class="flex items-center justify-between gap-2 mb-1">
            <div class="text-sm font-medium text-ink truncate">{{ m.character_name }}</div>
            <Badge variant="neutral" size="sm">
              置信度 {{ Math.round(m.analysis_confidence * 100) }}%
            </Badge>
          </div>
          <div class="text-xs text-ink-soft mb-2">
            {{ routingText(m.routing) }} · {{ m.ui_binding_count }} 绑定 · {{ m.fallback_count }} 兜底
          </div>
          <div class="flex gap-1.5">
            <Button
              variant="default"
              size="sm"
              :disabled="analyzingCardId === m.source_character_id"
              :loading="analyzingCardId === m.source_character_id"
              @click="handleAnalyze(m.source_character_id)"
            >重新分析</Button>
            <Button
              variant="primary"
              size="sm"
              :disabled="applyPreviewLoading"
              :loading="applyPreviewLoading && applyPreviewSource?.id === m.source_character_id"
              @click="handlePreviewApply(m.source_character_id, m.character_name)"
            >应用 Schema</Button>
          </div>
        </div>
      </div>
    </section>

    <!-- MVU 分析详情浮层 -->
    <Overlay
      :show="!!activeDetail"
      side="center"
      :title="activeDetail ? (activeDetail.character_name + ' · MVU 分析结果') : ''"
      @update:show="activeDetail = $event ? activeDetail : null"
    >
      <div v-if="activeDetail" class="p-4 space-y-3 text-xs">
        <!-- 路由 + 置信度 -->
        <div class="flex gap-1.5 flex-wrap">
          <Badge
            :variant="activeDetail.translation.routing.kind === 'hybrid' ? 'warn' : 'ok'"
            size="sm"
          >{{ routingText(activeDetail.translation.routing) }}</Badge>
          <Badge variant="neutral" size="sm">
            置信度 {{ Math.round(activeDetail.analysis_confidence * 100) }}%
          </Badge>
        </div>

        <!-- 启发式打分 -->
        <div v-if="activeDetail.complexity && activeDetail.complexity.classification" class="text-ink-soft">
          启发式分类：{{ activeDetail.complexity.classification }} · {{ activeDetail.complexity.reasoning }}
        </div>

        <!-- 统计 -->
        <div class="grid grid-cols-2 gap-2 text-ink-soft">
          <div>变量字段：{{ activeDetail.translation.variable_schema.length }}</div>
          <div>UI 绑定：{{ activeDetail.translation.ui_bindings.length }}</div>
          <div>更新规则：{{ activeDetail.translation.update_rules.length }}</div>
          <div>交互映射：{{ activeDetail.translation.interactions.length }}</div>
          <div>兜底 JS：{{ activeDetail.translation.fallback_fragments.length }}</div>
        </div>

        <!-- UI 绑定列表 -->
        <div v-if="activeDetail.translation.ui_bindings.length > 0">
          <div class="font-medium text-ink mb-1">UI 绑定</div>
          <div
            v-for="b in activeDetail.translation.ui_bindings"
            :key="b.element"
            class="bg-surface-2 rounded px-2 py-1.5 mb-1 flex justify-between gap-2"
          >
            <span class="text-ink">{{ b.element }}</span>
            <span class="text-ink-soft">{{ b.variable_key }} ({{ b.display.kind }})</span>
          </div>
        </div>

        <!-- 更新规则 -->
        <div v-if="activeDetail.translation.update_rules.length > 0">
          <div class="font-medium text-ink mb-1">更新规则（注入后处理 Agent）</div>
          <div
            v-for="(r, i) in activeDetail.translation.update_rules"
            :key="i"
            class="bg-surface-2 rounded px-2 py-1.5 mb-1 text-ink-soft"
          >{{ i + 1 }}. {{ r }}</div>
        </div>

        <!-- 兜底片段 -->
        <div v-if="activeDetail.translation.fallback_fragments.length > 0">
          <div class="font-medium text-warn mb-1">⚠ 兜底 JS（需共享 WebView，下一轮实现）</div>
          <div
            v-for="(f, i) in activeDetail.translation.fallback_fragments"
            :key="i"
            class="bg-warn/5 border border-warn/20 rounded px-2 py-1.5 mb-1"
          >
            <div class="text-ink">{{ f.description }}</div>
            <div class="text-ink-soft text-[10px]">原因：{{ f.reason }}</div>
          </div>
        </div>

        <!-- 备注 -->
        <div v-if="activeDetail.translation.notes.length > 0">
          <div class="font-medium text-ink mb-1">备注</div>
          <div
            v-for="(n, i) in activeDetail.translation.notes"
            :key="i"
            class="text-ink-soft text-[10px]"
          >• {{ n }}</div>
        </div>
      </div>
    </Overlay>

    <!-- MVU Apply 预览浮层 -->
    <Overlay
      :show="!!applyPreviewSource"
      side="center"
      :title="applyPreviewSource ? (applyPreviewSource.name + ' · Schema 合并预览') : ''"
      @update:show="(v) => { if (!v) closeApplyPreview() }"
    >
      <div v-if="applyPreviewLoading" class="p-6">
        <LoadingState label="加载预览中…" />
      </div>
      <div v-else-if="applyPreviews.length === 0" class="p-6">
        <EmptyState title="无可用 definition" />
      </div>
      <div v-else class="p-4 space-y-3">
        <div
          v-for="p in applyPreviews"
          :key="p.definition_id"
          class="bg-surface rounded-lg border border-line p-3 text-xs"
          :class="p.has_changes ? '' : 'opacity-50'"
        >
          <!-- header -->
          <div class="flex items-center justify-between mb-2 gap-2">
            <div class="font-medium text-ink">
              {{ p.character_name }}
              <span class="text-ink-soft font-normal ml-1 text-[10px]">({{ p.definition_id.slice(0, 8) }}…)</span>
            </div>
            <Badge :variant="p.has_changes ? 'accent' : 'ok'" size="sm">
              {{ p.has_changes ? '有变更' : '无变化' }}
            </Badge>
          </div>

          <!-- diff 详情 -->
          <div v-if="p.has_changes" class="space-y-2 mb-2">
            <!-- 新增字段 -->
            <div v-if="p.added_fields.length > 0">
              <div class="text-accent font-medium mb-1">+ 新增 {{ p.added_fields.length }} 个字段</div>
              <div
                v-for="(f, i) in p.added_fields"
                :key="'a'+i"
                class="bg-surface-2 rounded px-1.5 py-1 border border-line text-[10px] text-ink-soft break-all"
              >{{ fieldSummary(f) }}</div>
            </div>
            <!-- 覆盖字段 -->
            <div v-if="p.overwritten_fields.length > 0">
              <div class="text-warn font-medium mb-1">↻ 覆盖 {{ p.overwritten_fields.length }} 个字段</div>
              <div
                v-for="(f, i) in p.overwritten_fields"
                :key="'o'+i"
                class="bg-surface-2 rounded px-1.5 py-1 border border-line text-[10px] text-ink-soft break-all"
              >{{ fieldSummary(f) }}</div>
            </div>
          </div>
          <div class="text-ink-soft text-[10px]">
            合并后共 {{ p.merged_schema.length }} 个字段 · {{ p.unchanged_count }} 个不变
          </div>

          <!-- 操作按钮 -->
          <div class="flex justify-end mt-2.5">
            <Button
              variant="primary"
              size="sm"
              :disabled="!p.has_changes || applyingDefId === p.definition_id"
              :loading="applyingDefId === p.definition_id"
              @click="handleApplySchema(p.definition_id)"
            >{{ applyingDefId === p.definition_id ? '应用中…' : '应用' }}</Button>
          </div>
        </div>
      </div>
    </Overlay>
  </div>
</template>
