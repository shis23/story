<script setup>
import { ref } from 'vue'
import { alertDialog } from '../../components/base/BaseDialog.js'
import {
  listCards, getCard, extractCharacters,
} from '../../tauri-api.js'
import {
  campaignCardDefinitionCount as definitionCount,
  campaignCardExtractionLabel as extractionLabel,
  campaignCardExtractionStatus as extractionStatus,
  campaignCardIsFullyExtracted,
  campaignCardShouldShowExtractButton as shouldShowExtractButton,
} from '../../utils/campaignCardStatus.js'
import DataList from '../ui/DataList.vue'
import Badge from '../ui/Badge.vue'
import Button from '../ui/Button.vue'
import EmptyState from '../ui/EmptyState.vue'
import LoadingState from '../ui/LoadingState.vue'
import WorldInfoReadonlyPanel from './WorldInfoReadonlyPanel.vue'

const emit = defineEmits(['open-campaigns'])

// ─── Cards 状态 ───
const cards = ref([])
const loadingCards = ref(false)
const extractingCardId = ref(null)
const expandedCardId = ref(null)
const cardDetail = ref(null)

// ─── Cards 操作 ───
async function refreshCards() {
  loadingCards.value = true
  try {
    cards.value = await listCards()
  } finally {
    loadingCards.value = false
  }
}

// 挂载即加载(与原 CampaignPanel onMounted 调 refreshCards 一致)
refreshCards().catch(() => {})

// 暴露 refresh 给父组件(CampaignPanel 导入 Bundle 后刷新)
defineExpose({ refresh: refreshCards })

function extractionClass(card) {
  return campaignCardIsFullyExtracted(card) ? 'text-ok' : 'text-warn'
}

function extractButtonText(card) {
  const status = extractionStatus(card)
  if (extractingCardId.value === card.source_character_id) return '识别中…'
  return status === 'unknown' && definitionCount(card) === 0 ? '识别角色' : '重新识别'
}

async function handleExtract(card) {
  extractingCardId.value = card.source_character_id
  try {
    const result = await extractCharacters(card.source_character_id, { force: true })
    await refreshCards()
    expandedCardId.value = result.id
    cardDetail.value = await getCard(result.id)
  } catch (e) {
    await alertDialog('角色识别失败: ' + e)
  } finally {
    extractingCardId.value = null
  }
}

async function toggleCard(card) {
  if (expandedCardId.value === card.id) {
    expandedCardId.value = null
    cardDetail.value = null
  } else {
    expandedCardId.value = card.id
    cardDetail.value = await getCard(card.id)
  }
}

// 角色类型 → Badge variant
function roleVariant(roleType) {
  if (roleType === 'Protagonist') return 'accent'
  if (roleType === 'Supporting') return 'ok'
  return 'neutral'
}
</script>

<template>
  <div class="space-y-3">
    <LoadingState v-if="loadingCards" />

    <EmptyState
      v-else-if="cards.length === 0"
      title="还没有角色卡"
      description="请先导入角色卡"
    />

    <template v-else>
      <DataList :items="cards" active-key="id">
        <template #item="{ item: card }">
          <!-- 卡头部 -->
          <div class="flex items-center gap-3" @click.stop="toggleCard(card)">
            <div class="flex-1 min-w-0">
              <div class="text-sm font-medium text-ink truncate">{{ card.name }}</div>
              <div class="text-xs text-ink-soft">
                {{ definitionCount(card) }} 个角色定义
                <span class="ml-1" :class="extractionClass(card)">{{ extractionLabel(card) }}</span>
              </div>
            </div>
            <Button
              v-if="shouldShowExtractButton(card)"
              variant="primary"
              size="sm"
              :disabled="extractingCardId === card.source_character_id"
              @click.stop="handleExtract(card)"
            >{{ extractButtonText(card) }}</Button>
            <span class="text-ink-soft text-xs">{{ expandedCardId === card.id ? '▲' : '▼' }}</span>
          </div>

          <!-- 卡展开详情 -->
          <div v-if="expandedCardId === card.id && cardDetail" class="border-t border-line mt-2 pt-2 space-y-2" @click.stop>
            <div v-if="cardDetail.extraction_message" class="text-xs text-warn bg-warn/10 rounded-lg px-3 py-2">
              {{ cardDetail.extraction_message }}
            </div>
            <div v-if="cardDetail.character_definitions.length === 0" class="text-xs text-ink-faint py-2">
              暂无角色定义，请点击「识别角色」
            </div>
            <div
              v-for="def in cardDetail.character_definitions"
              :key="def.id"
              class="bg-surface-2 rounded-lg px-3 py-2 text-xs"
            >
              <div class="flex items-center gap-2 mb-1">
                <span class="font-medium text-ink">{{ def.name }}</span>
                <Badge v-if="def.role_type" :variant="roleVariant(def.role_type)" size="sm">{{ def.role_type }}</Badge>
                <span v-if="def.group" class="text-ink-soft">{{ def.group }}</span>
              </div>
              <div class="text-ink-soft line-clamp-2">{{ def.persona_prompt }}</div>
            </div>

            <!-- 卡模板世界书（只读） -->
            <div class="pt-2 border-t border-line">
              <WorldInfoReadonlyPanel :character-id="card.source_character_id || cardDetail.source_character_id" />
            </div>

            <!-- 开档按钮 -->
            <div v-if="cardDetail.character_definitions.length > 0" class="pt-1">
              <Button variant="primary" size="md" class="w-full" @click="emit('open-campaigns', card)">
                管理游玩档 →
              </Button>
            </div>
          </div>
        </template>
      </DataList>
    </template>
  </div>
</template>
