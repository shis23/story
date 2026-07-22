<script setup>
import { ref, onMounted } from 'vue'
import { listCards, getCard, deleteCard } from '../tauri-api.js'
import { confirmDialog } from './base/BaseDialog.js'
import BaseOverlay from './base/BaseOverlay.vue'
import {
  campaignCardDefinitionCount as definitionCount,
  campaignCardExtractionStatus as extractionStatus,
  campaignCardIsFullyExtracted,
} from '../utils/campaignCardStatus.js'

const props = defineProps({
  activeId: { type: String, default: null },
})
const emit = defineEmits(['select', 'close'])

const cards = ref([])
const loading = ref(false)

onMounted(async () => {
  await refresh()
})

async function refresh() {
  loading.value = true
  try {
    // 统一读 cards.json（CharacterCard，与 Campaign 管理同源）
    cards.value = await listCards()
  } catch (e) {
    console.error('加载角色卡列表失败:', e)
  }
  loading.value = false
}

async function handleSelect(card) {
  try {
    const detail = await getCard(card.id)
    // 适配为 CharacterDetail 可用的 character 对象
    // CharacterCard 没有扁平字段（first_mes 等），从 character_definitions 拼基础信息
    const mainDef = detail.character_definitions?.[0] || {}
    emit('select', {
      id: card.source_character_id, // 扁平 Character id（CharacterDetail 用）
      name: card.name,
      description: mainDef.persona_prompt || '',
      personality: '',
      scenario: '',
      first_mes: '',
      system_prompt: '',
      spec_version: '',
      tags: [],
      world_info_count: 0,
      world_info_entries: [],
      has_renderable_assets: false,
      creator: '',
      _card: detail, // 保留完整 CharacterCard 详情供扩展
    })
  } catch (e) {
    console.error('获取角色卡详情失败:', e)
  }
}

async function handleDelete(card, event) {
  event.stopPropagation()
  const ok = await confirmDialog(`确定删除角色卡「${card.name}」？`, { title: '删除确认' })
  if (!ok) return
  try {
    // 按 CharacterCard.id 删（级联删 campaign/instances/mvu）
    await deleteCard(card.id)
    await refresh()
    if (card.id === props.activeId) {
      emit('select', null)
    }
  } catch (e) {
    console.error('删除失败:', e)
  }
}

function extractionLabel(card) {
  const status = extractionStatus(card)
  if (status === 'extracted' && definitionCount(card) > 0) return '已识别'
  if (status === 'extracted') return '已识别但无角色定义'
  if (status === 'fallback') return '降级可用'
  if (definitionCount(card) > 0) return '历史状态未知'
  return '未识别'
}

function extractionClass(card) {
  return campaignCardIsFullyExtracted(card) ? 'text-ok' : 'text-warn'
}
</script>

<template>
  <!-- size=drawer 对齐 --layout-drawer，与 v2 侧滑功能抽屉同宽 -->
  <BaseOverlay :model-value="true" title="角色卡列表" size="drawer" position="left" @close="emit('close')">
    <template #header-extra>
      <span class="text-xs text-ink-soft">{{ cards.length }} 张</span>
    </template>

    <div v-if="loading" class="p-8 text-center text-ink-soft text-sm">加载中…</div>

    <div v-else-if="cards.length === 0" class="p-8 text-center text-ink-soft text-sm">
      还没有导入角色卡<br>
      <span class="text-xs mt-1 block">点左导航「导入」开始</span>
    </div>

    <div v-else class="divide-y divide-line">
      <div
        v-for="card in cards"
        :key="card.id"
        @click="handleSelect(card)"
        class="px-4 py-3 cursor-pointer hover:bg-accent-soft/50 transition-colors flex items-start gap-3"
      >
        <div class="w-10 h-10 rounded-full bg-surface-2 flex items-center justify-center text-lg shrink-0">
          {{ card.name?.charAt(0) || '?' }}
        </div>

        <div class="flex-1 min-w-0">
          <div class="font-medium text-ink text-sm truncate">{{ card.name }}</div>
          <div class="text-xs text-ink-soft mt-0.5">
            {{ definitionCount(card) }} 个角色定义
            <span class="ml-1" :class="extractionClass(card)">{{ extractionLabel(card) }}</span>
          </div>
          <div class="text-[10px] text-ink-faint mt-0.5">{{ card.imported_at }}</div>
        </div>

        <button
          @click="handleDelete(card, $event)"
          class="w-9 h-9 flex items-center justify-center text-ink-faint hover:text-err hover:bg-err/10 rounded-lg shrink-0 transition-colors"
          title="删除"
        >
          ✕
        </button>
      </div>
    </div>
  </BaseOverlay>
</template>
