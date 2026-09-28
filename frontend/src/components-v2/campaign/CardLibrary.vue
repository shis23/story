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
import { errorText } from '../../utils/errorText.js'

const emit = defineEmits(['open-campaigns', 'open-studio', 'revise-card'])

// ─── Cards 状态 ───
const cards = ref([])
const loadingCards = ref(false)
const cardsError = ref(null)
const extractingCardId = ref(null)
const expandedCardId = ref(null)
const cardDetail = ref(null)
const loadingDetail = ref(false)

// ─── Cards 操作 ───
async function refreshCards() {
  loadingCards.value = true
  cardsError.value = null
  try {
    cards.value = await listCards()
  } catch (e) {
    // F-11：此前是 `refreshCards().catch(() => {})` —— 加载失败渲染成
    // EmptyState「还没有角色卡」，用户把错误看成"没有数据"且无重试入口。
    cardsError.value = errorText(e)
  } finally {
    loadingCards.value = false
  }
}

// 挂载即加载(与原 CampaignPanel onMounted 调 refreshCards 一致)
refreshCards()

// 暴露 refresh 给父组件(CampaignPanel 导入 Bundle 后刷新)
defineExpose({ refresh: refreshCards })

function extractionClass(card) {
  return campaignCardIsFullyExtracted(card) ? 'text-ok' : 'text-warn'
}

// F-29：老数据可能没有 source_character_id（CLAUDE.md：old data falls back to
// StoredCharacter.id）。识别接口必须用回退后的 id，且禁用标记也要用它，
// 否则 `extractCharacters(undefined)` 必失败，并把所有缺该字段的卡一起锁在"识别中…"。
function sourceCharacterId(card) {
  return card?.source_character_id || card?.id
}

function extractButtonText(card) {
  const status = extractionStatus(card)
  if (extractingCardId.value != null && extractingCardId.value === sourceCharacterId(card)) return '识别中…'
  return status === 'unknown' && definitionCount(card) === 0 ? '识别角色' : '重新识别'
}

async function handleExtract(card) {
  const sourceId = sourceCharacterId(card)
  if (!sourceId) {
    await alertDialog('无法识别：这张卡既没有 source_character_id 也没有 id')
    return
  }
  extractingCardId.value = sourceId
  try {
    const result = await extractCharacters(sourceId, { force: true })
    await refreshCards()
    expandedCardId.value = result.id
    cardDetail.value = await getCard(result.id)
  } catch (e) {
    await alertDialog('角色识别失败: ' + errorText(e))
  } finally {
    extractingCardId.value = null
  }
}

async function toggleCard(card) {
  if (expandedCardId.value === card.id) {
    expandedCardId.value = null
    cardDetail.value = null
    return
  }
  expandedCardId.value = card.id
  cardDetail.value = null
  loadingDetail.value = true
  try {
    cardDetail.value = await getCard(card.id)
  } catch (e) {
    // 展开失败必须回滚展开态，且给用户可见提示（此前是未处理的 promise rejection）
    expandedCardId.value = null
    await alertDialog('读取角色卡详情失败: ' + errorText(e))
  } finally {
    loadingDetail.value = false
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
    <div class="flex items-center justify-between gap-2">
      <div class="text-xs text-ink-soft">导入已有卡，写卡工作室从零生成，或在详情中修订补卡（另存）</div>
      <Button variant="primary" size="sm" @click="emit('open-studio')">写卡工作室</Button>
    </div>

    <LoadingState v-if="loadingCards" />

    <!-- F-11：加载失败要能和"没有数据"区分，并给重试入口 -->
    <div v-else-if="cardsError" class="rounded-xl border border-err/40 bg-err/10 px-3 py-3 text-center space-y-2">
      <div class="text-xs text-err">加载角色卡失败: {{ cardsError }}</div>
      <Button variant="default" size="sm" @click="refreshCards">重试</Button>
    </div>

    <EmptyState
      v-else-if="cards.length === 0"
      title="还没有角色卡"
      description="请先导入角色卡，或打开写卡工作室从零创建"
    />

    <template v-else>
      <DataList
        :items="cards"
        active-key="id"
        :active-id="expandedCardId"
        :max-items="60"
        @select="toggleCard"
      >
        <template #item="{ item: card }">
          <!-- 卡头部：整行点击由 DataList 的 select 事件驱动（F-06） -->
          <div class="flex items-center gap-3">
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
              :disabled="extractingCardId === sourceCharacterId(card)"
              @click.stop="handleExtract(card)"
            >{{ extractButtonText(card) }}</Button>
            <span class="text-ink-soft text-xs">{{ expandedCardId === card.id ? '▲' : '▼' }}</span>
          </div>

          <!-- 详情加载中（展开失败会回滚展开态并有提示） -->
          <div v-if="expandedCardId === card.id && loadingDetail" class="border-t border-line mt-2 pt-2 text-xs text-ink-soft" role="status">
            详情加载中…
          </div>

          <!-- 卡展开详情 -->
          <div v-else-if="expandedCardId === card.id && cardDetail" class="border-t border-line mt-2 pt-2 space-y-2" @click.stop>
            <div v-if="cardDetail.extraction_message" class="text-xs text-warn bg-warn/10 rounded-lg px-3 py-2">
              {{ cardDetail.extraction_message }}
            </div>
            <div v-if="!cardDetail.character_definitions?.length" class="text-xs text-ink-faint py-2">
              暂无角色定义，请点击「识别角色」
            </div>
            <div
              v-for="def in cardDetail.character_definitions || []"
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
              <WorldInfoReadonlyPanel :character-id="sourceCharacterId(card) || cardDetail.source_character_id" />
            </div>

            <!-- 操作 -->
            <div class="pt-1 grid gap-2" :class="cardDetail.character_definitions?.length > 0 ? 'sm:grid-cols-2' : ''">
              <Button
                variant="default"
                size="md"
                class="w-full"
                @click="emit('revise-card', card)"
              >
                写卡工作室修订
              </Button>
              <Button
                v-if="cardDetail.character_definitions?.length > 0"
                variant="primary"
                size="md"
                class="w-full"
                @click="emit('open-campaigns', card)"
              >
                管理游玩档 →
              </Button>
            </div>
          </div>
        </template>
      </DataList>
    </template>
  </div>
</template>
