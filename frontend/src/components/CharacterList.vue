<script setup>
import { onMounted, ref } from 'vue'
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
    cards.value = await listCards()
  } catch (e) {
    console.error('加载角色卡列表失败:', e)
  }
  loading.value = false
}

async function handleSelect(card) {
  try {
    const detail = await getCard(card.id)
    const mainDef = detail.character_definitions?.[0] || {}
    emit('select', {
      id: card.source_character_id,
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
      _card: detail,
    })
  } catch (e) {
    console.error('获取角色卡详情失败:', e)
  }
}

async function handleDelete(card) {
  const ok = await confirmDialog(`确定删除角色卡「${card.name}」？`, { title: '删除确认' })
  if (!ok) return
  try {
    await deleteCard(card.id)
    await refresh()
    if (props.activeId === card.id || props.activeId === card.source_character_id) {
      emit('select', null)
    }
  } catch (e) {
    console.error('删除失败:', e)
  }
}

function extractionLabel(card) {
  const status = extractionStatus(card)
  if (status === 'extracted' && definitionCount(card) > 0) return '已识别'
  if (status === 'extracted') return '已识别但无定义'
  if (status === 'fallback') return '降级可用'
  if (definitionCount(card) > 0) return '历史数据'
  return '未识别'
}

function extractionClass(card) {
  return campaignCardIsFullyExtracted(card)
    ? 'bg-ok/10 text-ok border-ok/20'
    : 'bg-warn/10 text-warn border-warn/20'
}

function isActive(card) {
  return props.activeId === card.id || props.activeId === card.source_character_id
}

function formatImportedAt(value) {
  const date = new Date(value)
  if (!value || Number.isNaN(date.getTime())) return '导入时间未知'
  return new Intl.DateTimeFormat('zh-CN', {
    year: 'numeric',
    month: 'long',
    day: 'numeric',
  }).format(date)
}
</script>

<template>
  <BaseOverlay
    :model-value="true"
    title="角色卡库"
    size="drawer"
    position="left"
    :body-scroll="false"
    @close="emit('close')"
  >
    <template #header-extra>
      <span class="rounded-full bg-surface-2 px-2.5 py-1 text-xs font-medium text-ink-soft">
        {{ cards.length }} 张
      </span>
    </template>

    <div class="relative flex min-h-0 flex-1 flex-col overflow-hidden bg-bg">
      <div class="pointer-events-none absolute -right-20 top-10 h-56 w-56 rounded-full bg-accent-soft blur-3xl"></div>

      <div class="relative shrink-0 border-b border-line px-5 py-4">
        <div class="text-[10px] font-semibold uppercase tracking-[0.2em] text-accent">Cast library</div>
        <p class="mt-1 text-sm leading-6 text-ink-soft">
          管理已导入的角色卡。点击卡片查看识别结果、原始资料与世界书。
        </p>
      </div>

      <div v-if="loading" class="relative flex flex-1 items-center justify-center px-8 text-center">
        <div>
          <span class="mx-auto block h-6 w-6 animate-spin rounded-full border-2 border-line border-t-accent"></span>
          <div class="mt-3 text-sm text-ink-soft">正在整理角色卡…</div>
        </div>
      </div>

      <div v-else-if="cards.length === 0" class="relative flex flex-1 items-center justify-center px-8 text-center">
        <div class="max-w-xs">
          <div class="mx-auto flex h-14 w-14 items-center justify-center rounded-2xl border border-dashed border-accent-border bg-accent-soft text-accent">
            <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <rect x="4" y="3" width="16" height="18" rx="2" />
              <path d="M8 8h8M8 12h5M8 16h6" />
            </svg>
          </div>
          <h3 class="mt-4 text-base font-semibold text-ink">还没有角色卡</h3>
          <p class="mt-1 text-sm leading-6 text-ink-soft">从左侧导航选择“导入”，添加 PNG 或 JSON 角色卡。</p>
        </div>
      </div>

      <div v-else class="relative min-h-0 flex-1 overflow-y-auto p-4">
        <div class="space-y-3">
          <article
            v-for="card in cards"
            :key="card.id"
            data-testid="character-card-row"
            class="group relative overflow-hidden rounded-2xl border bg-surface shadow-card transition-all duration-200 hover:-translate-y-0.5 hover:border-accent-border hover:shadow-rise"
            :class="isActive(card) ? 'border-accent-border ring-1 ring-accent-border' : 'border-line'"
          >
            <span
              v-if="isActive(card)"
              class="absolute inset-y-3 left-0 w-0.5 rounded-r-full bg-accent"
              aria-hidden="true"
            ></span>

            <button
              type="button"
              class="flex w-full items-start gap-3.5 p-4 pr-14 text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent-border"
              :aria-label="`查看 ${card.name} 详情`"
              @click="handleSelect(card)"
            >
              <div class="flex h-12 w-12 shrink-0 items-center justify-center rounded-2xl border border-accent-border bg-accent-soft text-lg font-semibold text-accent-bright shadow-card">
                {{ card.name?.charAt(0) || '?' }}
              </div>

              <div class="min-w-0 flex-1">
                <div class="truncate text-base font-semibold tracking-tight text-ink">{{ card.name }}</div>

                <div class="mt-2 flex flex-wrap items-center gap-1.5">
                  <span class="rounded-md bg-surface-2 px-2 py-0.5 text-xs font-medium text-ink-soft">
                    {{ definitionCount(card) }} 个角色定义
                  </span>
                  <span
                    class="rounded-md border px-2 py-0.5 text-xs font-medium"
                    :class="extractionClass(card)"
                  >
                    <span class="mr-1 inline-block h-1.5 w-1.5 rounded-full bg-current align-middle"></span>
                    {{ extractionLabel(card) }}
                  </span>
                </div>

                <time
                  :datetime="card.imported_at"
                  :title="card.imported_at"
                  class="mt-2.5 flex items-center gap-1.5 text-[11px] text-ink-faint"
                >
                  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                    <rect x="3" y="5" width="18" height="16" rx="2" />
                    <path d="M16 3v4M8 3v4M3 10h18" />
                  </svg>
                  {{ formatImportedAt(card.imported_at) }}
                </time>
              </div>

              <span class="absolute bottom-4 right-4 text-ink-faint transition-transform group-hover:translate-x-0.5 group-hover:text-accent" aria-hidden="true">
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                  <path d="m9 18 6-6-6-6" />
                </svg>
              </span>
            </button>

            <button
              type="button"
              class="absolute right-3 top-3 flex h-9 w-9 items-center justify-center rounded-xl text-ink-faint opacity-70 transition-colors hover:bg-err/10 hover:text-err focus:opacity-100 focus:outline-none focus-visible:ring-2 focus-visible:ring-err/30 sm:opacity-0 sm:group-hover:opacity-100"
              :aria-label="`删除 ${card.name}`"
              :title="`删除 ${card.name}`"
              @click="handleDelete(card)"
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M3 6h18M8 6V4h8v2M19 6l-1 15H6L5 6M10 11v6M14 11v6" />
              </svg>
            </button>
          </article>
        </div>
      </div>

      <div v-if="cards.length" class="relative shrink-0 border-t border-line bg-surface/80 px-5 py-3 text-xs text-ink-faint backdrop-blur-sm">
        点击角色卡进入详情；删除操作会再次确认。
      </div>
    </div>
  </BaseOverlay>
</template>
