<script setup>
import { ref, computed, onMounted } from 'vue'
import { alertDialog } from './base/BaseDialog.js'
import BaseOverlay from './base/BaseOverlay.vue'
import {
  listCards, getCard, extractCharacters,
  listCampaigns, createCampaign, setActiveCampaign, getActiveCampaign,
  exportCampaignStCards, exportCampaignBundle, importCampaignBundle
} from '../tauri-api.js'
import CampaignInstancesTab from './CampaignInstancesTab.vue'
import CampaignKnowledgeTab from './CampaignKnowledgeTab.vue'
import CampaignTasksTab from './CampaignTasksTab.vue'
import CampaignSummariesTab from './CampaignSummariesTab.vue'
import {
  campaignCardDefinitionCount as definitionCount,
  campaignCardExtractionLabel as extractionLabel,
  campaignCardExtractionStatus as extractionStatus,
  campaignCardIsFullyExtracted,
  campaignCardShouldShowExtractButton as shouldShowExtractButton,
} from '../utils/campaignCardStatus.js'
import { buildGreetingOptionsFromDetail } from '../utils/campaignGreetingOptions.js'
import { refreshSubTab, subTabRefKey } from '../utils/campaignTabRefresh.js'

const emit = defineEmits(['close', 'campaign-changed'])

// ─── Tab 控制 ───
const activeTab = ref('cards') // 'cards' | 'campaigns' | 'detail'

// ─── Cards 状态 ───
const cards = ref([])
const loadingCards = ref(false)
const extractingCardId = ref(null)
const expandedCardId = ref(null)
const cardDetail = ref(null)

// ─── Campaigns 状态 ───
const selectedCardId = ref(null)
const selectedCampaignCardDetail = ref(null)
const campaigns = ref([])
const loadingCampaigns = ref(false)
const activeCampaign = ref(null)
const showNewCampaign = ref(false)
const newCampaignName = ref('')
const newCampaignGreetingIndex = ref(0)
const creatingCampaign = ref(false)

const newCampaignGreetingOptions = computed(() => buildGreetingOptionsFromDetail(selectedCampaignCardDetail.value))
const selectedNewCampaignGreeting = computed(() => newCampaignGreetingOptions.value[newCampaignGreetingIndex.value] || null)

function normalizeNewCampaignGreetingSelection() {
  if (newCampaignGreetingIndex.value >= newCampaignGreetingOptions.value.length) {
    newCampaignGreetingIndex.value = 0
  }
}

async function loadSelectedCampaignCardDetail() {
  selectedCampaignCardDetail.value = null
  newCampaignGreetingIndex.value = 0
  if (!selectedCardId.value) return
  selectedCampaignCardDetail.value = await getCard(selectedCardId.value)
  normalizeNewCampaignGreetingSelection()
}

// ─── Detail 状态 ───
const selectedCampaignId = ref(null)
const detailSubTab = ref('instances') // 'instances' | 'knowledge' | 'tasks' | 'summaries'

// ─── 子组件 template refs ───
const instancesTabRef = ref(null)
const knowledgeTabRef = ref(null)
const tasksTabRef = ref(null)
const summariesTabRef = ref(null)

// ─── 导出状态 ───
const exporting = ref(false)
const exportStatus = ref('')
const importingBundle = ref(false)
const importStatus = ref('')

// ─── 初始化 ───
onMounted(async () => {
  await refreshCards()
  activeCampaign.value = await getActiveCampaign()
})

// ─── Cards 操作 ───
async function refreshCards() {
  loadingCards.value = true
  try {
    cards.value = await listCards()
  } finally {
    loadingCards.value = false
  }
}

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

// ─── Campaigns 操作 ───
async function refreshCampaigns() {
  if (!selectedCardId.value) return
  loadingCampaigns.value = true
  try {
    campaigns.value = await listCampaigns(selectedCardId.value)
  } finally {
    loadingCampaigns.value = false
  }
}

async function openCampaignsForCard(card) {
  selectedCardId.value = card.id
  activeTab.value = 'campaigns'
  showNewCampaign.value = false
  await loadSelectedCampaignCardDetail()
  await refreshCampaigns()
}

async function openNewCampaignForm() {
  showNewCampaign.value = true
  newCampaignGreetingIndex.value = 0
  if (selectedCardId.value && !selectedCampaignCardDetail.value) {
    await loadSelectedCampaignCardDetail()
  }
}

async function handleCreateCampaign() {
  if (!newCampaignName.value.trim() || !selectedCardId.value) return
  creatingCampaign.value = true
  try {
    const result = await createCampaign(
      selectedCardId.value,
      newCampaignName.value.trim(),
      selectedNewCampaignGreeting.value?.content || null,
    )
    newCampaignName.value = ''
    newCampaignGreetingIndex.value = 0
    showNewCampaign.value = false
    await refreshCampaigns()
    await handleSetActive(result.id)
  } catch (e) {
    await alertDialog('创建失败: ' + e)
  } finally {
    creatingCampaign.value = false
  }
}

async function handleSetActive(campaignId) {
  await setActiveCampaign(campaignId)
  activeCampaign.value = await getActiveCampaign()
  emit('campaign-changed', activeCampaign.value)
}

async function openCampaignDetail(campaignId) {
  selectedCampaignId.value = campaignId
  activeTab.value = 'detail'
  // 子组件各自 onMounted 加载，不需要 refreshDetail 全拉
}

// ─── 刷新当前活跃的 detail 子 tab ───
function refreshActiveDetailTab() {
  const refMap = {
    instances: instancesTabRef,
    knowledge: knowledgeTabRef,
    tasks: tasksTabRef,
    summaries: summariesTabRef,
  }
  // 把 Vue template ref 包裹对象转成纯 { refresh } 映射
  const tabRefs = {}
  for (const key of Object.keys(refMap)) {
    const refKey = subTabRefKey(key)
    if (refKey && refMap[key].value) {
      tabRefs[refKey] = refMap[key].value
    }
  }
  refreshSubTab(detailSubTab.value, tabRefs)
}

// ─── 导出操作 ───

async function saveFileViaDialog(filename, data, mimeType = 'application/octet-stream') {
  try {
    const { save } = await import('@tauri-apps/plugin-dialog')
    const filePath = await save({
      defaultPath: filename,
      filters: [{ name: 'Files', extensions: [filename.split('.').pop() || '*'] }],
    })
    if (filePath) {
      const { writeBinaryFile } = await import('@tauri-apps/plugin-fs')
      await writeBinaryFile(filePath, data)
      return true
    }
    return false
  } catch (e) {
    console.error('保存文件失败:', e)
    throw e
  }
}

async function handleExportStCards() {
  if (!selectedCampaignId.value) return
  exporting.value = true
  exportStatus.value = '正在导出 ST 卡…'
  try {
    const result = await exportCampaignStCards(selectedCampaignId.value)
    if (!result || !result.cards || result.cards.length === 0) {
      exportStatus.value = '无可导出的角色'
      return
    }

    // 逐个保存 PNG 文件
    let saved = 0
    for (const card of result.cards) {
      const ok = await saveFileViaDialog(card.filename, new Uint8Array(card.data))
      if (ok) saved++
    }

    // 保存共享 lorebook
    if (result.lorebook_json && result.lorebook_json !== '{}') {
      const lorebookData = new TextEncoder().encode(result.lorebook_json)
      const ok = await saveFileViaDialog('lorebook.json', lorebookData)
      if (ok) saved++
    }

    exportStatus.value = `已保存 ${saved} 个文件`
  } catch (e) {
    exportStatus.value = '导出失败: ' + e
  } finally {
    exporting.value = false
  }
}

async function handleExportBundle() {
  if (!selectedCampaignId.value) return
  exporting.value = true
  exportStatus.value = '正在导出 Bundle…'
  try {
    const json = await exportCampaignBundle(selectedCampaignId.value)
    if (!json) {
      exportStatus.value = '导出失败：无数据'
      return
    }
    const data = new TextEncoder().encode(json)
    const ok = await saveFileViaDialog('campaign-bundle.json', data)
    exportStatus.value = ok ? 'Bundle 导出完成' : '已取消'
  } catch (e) {
    exportStatus.value = '导出失败: ' + e
  } finally {
    exporting.value = false
  }
}

async function handleImportBundle() {
  importingBundle.value = true
  importStatus.value = '正在导入 Bundle…'
  exportStatus.value = ''
  try {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const filePath = await open({
      multiple: false,
      filters: [{ name: 'StoryForge Campaign Bundle', extensions: ['json'] }],
    })
    if (!filePath) {
      importStatus.value = '已取消'
      return
    }

    const { readTextFile } = await import('@tauri-apps/plugin-fs')
    const bundleJson = await readTextFile(filePath)
    const result = await importCampaignBundle(bundleJson)
    if (!result?.campaign_id || !result?.card_id) {
      importStatus.value = '导入失败：无返回数据'
      return
    }

    await refreshCards()
    selectedCardId.value = result.card_id
    await refreshCampaigns()
    selectedCampaignId.value = result.campaign_id
    await handleSetActive(result.campaign_id)
    activeTab.value = 'detail'
    const message = `导入完成：${result.instance_count} 个角色，${result.knowledge_count} 条知识`
    importStatus.value = message
    exportStatus.value = message
  } catch (e) {
    importStatus.value = '导入失败: ' + e
  } finally {
    importingBundle.value = false
  }
}

// ─── 暴露 refresh 给父组件（MetaPanel apply 后触发刷新） ───
defineExpose({ refreshActiveDetailTab })
</script>

<template>
  <BaseOverlay :model-value="true" title="Campaign 管理" size="md" position="left" @close="emit('close')">
    <!-- Tab 切换（sticky 在内容区顶部） -->
    <div class="sticky top-0 z-10 bg-bg border-b border-line shrink-0">
      <div class="flex">
        <button
          v-for="tab in [{key:'cards',label:'角色卡'},{key:'campaigns',label:'游玩档'},{key:'detail',label:'档详情'}]"
          :key="tab.key"
          @click="activeTab = tab.key"
          class="flex-1 min-h-[44px] text-xs font-medium transition-colors"
          :class="activeTab === tab.key ? 'text-accent border-b-2 border-accent' : 'text-ink-soft'"
        >{{ tab.label }}</button>
      </div>
    </div>

    <!-- 内容区 -->
    <div class="p-4 space-y-3">

      <!-- ═══ Tab 1: 角色卡 ═══ -->
      <template v-if="activeTab === 'cards'">
        <div class="flex items-center justify-end gap-2">
          <button
            @click="handleImportBundle"
            :disabled="importingBundle"
            class="min-h-[36px] px-3 rounded-lg text-xs font-medium bg-bg text-ink-soft hover:bg-line border border-line disabled:opacity-50 transition-colors"
          >{{ importingBundle ? '导入中…' : '导入 Bundle' }}</button>
        </div>
        <div v-if="importStatus" class="text-xs text-ink-soft">{{ importStatus }}</div>

        <div v-if="loadingCards" class="text-center text-ink-soft text-sm py-8">加载中…</div>

        <div v-else-if="cards.length === 0" class="text-center text-ink-soft text-sm py-8">
          还没有角色卡，请先导入
        </div>

        <div v-for="card in cards" :key="card.id" class="bg-surface rounded-xl border border-line overflow-hidden">
          <!-- 卡头部 -->
          <div
            class="flex items-center gap-3 px-3 py-2.5 cursor-pointer hover:bg-bg transition-colors"
            @click="toggleCard(card)"
          >
            <div class="flex-1 min-w-0">
              <div class="text-sm font-medium text-ink truncate">{{ card.name }}</div>
              <div class="text-xs text-ink-soft">
                {{ definitionCount(card) }} 个角色定义
                <span class="ml-1" :class="extractionClass(card)">{{ extractionLabel(card) }}</span>
              </div>
            </div>
            <button
              v-if="shouldShowExtractButton(card)"
              @click.stop="handleExtract(card)"
              :disabled="extractingCardId === card.source_character_id"
              class="min-h-[36px] px-3 rounded-full text-xs font-medium bg-accent text-white disabled:opacity-50 transition-colors"
            >
              {{ extractButtonText(card) }}
            </button>
            <span class="text-ink-soft text-xs">{{ expandedCardId === card.id ? '▲' : '▼' }}</span>
          </div>

          <!-- 卡展开详情 -->
          <div v-if="expandedCardId === card.id && cardDetail" class="border-t border-line px-3 py-2 space-y-2">
            <div v-if="cardDetail.extraction_message" class="text-xs text-warn bg-warn/10 rounded-lg px-3 py-2">
              {{ cardDetail.extraction_message }}
            </div>
            <div v-if="cardDetail.character_definitions.length === 0" class="text-xs text-ink-soft py-2">
              暂无角色定义，请点击「识别角色」
            </div>
            <div
              v-for="def in cardDetail.character_definitions"
              :key="def.id"
              class="bg-bg rounded-lg px-3 py-2 text-xs"
            >
              <div class="flex items-center gap-2 mb-1">
                <span class="font-medium text-ink">{{ def.name }}</span>
                <span class="px-1.5 py-0.5 rounded text-[10px]"
                  :class="{
                    'bg-accent/10 text-accent': def.role_type === 'Protagonist',
                    'bg-ok/10 text-ok': def.role_type === 'Supporting',
                    'bg-ink-soft/10 text-ink-soft': def.role_type === 'Extra',
                  }"
                >{{ def.role_type }}</span>
                <span v-if="def.group" class="text-ink-soft">{{ def.group }}</span>
              </div>
              <div class="text-ink-soft line-clamp-2">{{ def.persona_prompt }}</div>
            </div>

            <!-- 开档按钮 -->
            <div v-if="cardDetail.character_definitions.length > 0" class="pt-1">
              <button
                @click="openCampaignsForCard(card)"
                class="w-full min-h-[44px] rounded-lg text-sm font-medium bg-accent text-white hover:opacity-90 transition-colors"
              >管理游玩档 →</button>
            </div>
          </div>
        </div>
      </template>

      <!-- ═══ Tab 2: 游玩档 ═══ -->
      <template v-if="activeTab === 'campaigns'">
        <!-- 未选卡时提示选卡 -->
        <div v-if="!selectedCardId" class="text-center text-ink-soft text-sm py-8">
          请先在「角色卡」tab 选择一张卡
        </div>

        <template v-else>
          <div v-if="loadingCampaigns" class="text-center text-ink-soft text-sm py-8">加载中…</div>

          <div v-else-if="campaigns.length === 0 && !showNewCampaign" class="text-center py-8">
            <div class="text-ink-soft text-sm mb-3">还没有游玩档</div>
            <button @click="openNewCampaignForm" class="min-h-[44px] px-4 rounded-lg text-xs font-medium bg-accent text-white">
              新建游玩档
            </button>
          </div>

          <!-- 新建表单 -->
          <div v-if="showNewCampaign" class="bg-surface rounded-xl border border-line p-3 space-y-2">
            <div class="text-xs font-medium text-ink">新建游玩档</div>
            <div v-if="newCampaignGreetingOptions.length > 1">
              <label class="text-xs text-ink-soft mb-1 block">开场白</label>
              <select
                v-model="newCampaignGreetingIndex"
                class="w-full min-h-[44px] px-3 text-sm rounded-lg border border-line bg-bg focus:outline-none focus:border-accent"
              >
                <option v-for="(option, i) in newCampaignGreetingOptions" :key="i" :value="i">{{ option.label }}</option>
              </select>
            </div>
            <input
              v-model="newCampaignName"
              placeholder="输入档名（如：第一周目）"
              class="w-full min-h-[44px] px-3 text-sm rounded-lg border border-line bg-bg focus:outline-none focus:border-accent"
              @keyup.enter="handleCreateCampaign"
            />
            <div class="flex gap-2">
              <button @click="showNewCampaign = false" class="flex-1 min-h-[44px] rounded-lg text-sm bg-bg text-ink-soft">取消</button>
              <button
                @click="handleCreateCampaign"
                :disabled="creatingCampaign || !newCampaignName.trim()"
                class="flex-1 min-h-[44px] rounded-lg text-sm font-medium bg-accent text-white disabled:opacity-50"
              >{{ creatingCampaign ? '创建中…' : '创建' }}</button>
            </div>
          </div>

          <!-- Campaign 列表 -->
          <div
            v-for="camp in campaigns"
            :key="camp.id"
            class="bg-surface rounded-xl border overflow-hidden cursor-pointer transition-colors"
            :class="activeCampaign?.id === camp.id ? 'border-accent' : 'border-line hover:border-accent-border'"
            @click="openCampaignDetail(camp.id)"
          >
            <div class="flex items-center gap-3 px-3 py-2.5">
              <div class="flex-1 min-w-0">
                <div class="flex items-center gap-2">
                  <span class="text-sm font-medium text-ink">{{ camp.name }}</span>
                  <span v-if="activeCampaign?.id === camp.id" class="px-1.5 py-0.5 rounded text-[10px] bg-accent/10 text-accent">活跃</span>
                </div>
                <div class="text-xs text-ink-soft">{{ camp.instance_count }} 个角色实例</div>
              </div>
              <button
                v-if="activeCampaign?.id !== camp.id"
                @click.stop="handleSetActive(camp.id)"
                class="min-h-[36px] px-3 rounded-full text-xs bg-bg text-ink-soft hover:bg-line transition-colors"
              >设为活跃</button>
              <span class="text-ink-soft text-xs">→</span>
            </div>
          </div>

          <!-- 新建按钮（有档时显示） -->
          <button
            v-if="campaigns.length > 0 && !showNewCampaign"
            @click="openNewCampaignForm"
            class="w-full min-h-[44px] rounded-lg text-xs font-medium bg-bg text-ink-soft hover:bg-line border border-dashed border-line transition-colors"
          >+ 新建游玩档</button>
        </template>
      </template>

      <!-- ═══ Tab 3: 档详情 ═══ -->
      <template v-if="activeTab === 'detail'">
        <!-- 未选档提示 -->
        <div v-if="!selectedCampaignId" class="text-center text-ink-soft text-sm py-8">
          请先在「游玩档」tab 点击一个档
        </div>

        <template v-else>
          <!-- 导出按钮组 -->
          <div class="bg-surface rounded-xl border border-line p-3 mb-3 space-y-2">
            <div class="text-xs font-medium text-ink mb-1">导出</div>
            <div class="flex gap-2">
              <button
                @click="handleExportStCards()"
                :disabled="exporting"
                class="flex-1 min-h-[44px] rounded-lg text-xs font-medium bg-bg text-ink-soft hover:bg-line border border-line disabled:opacity-50 transition-colors"
              >
                {{ exporting ? '导出中…' : 'ST 卡 PNG' }}
              </button>
              <button
                @click="handleExportBundle()"
                :disabled="exporting"
                class="flex-1 min-h-[44px] rounded-lg text-xs font-medium bg-bg text-ink-soft hover:bg-line border border-line disabled:opacity-50 transition-colors"
              >
                {{ exporting ? '导出中…' : 'JSON Bundle' }}
              </button>
            </div>
            <div v-if="exportStatus" class="text-xs text-ink-soft">{{ exportStatus }}</div>
          </div>

          <!-- 刷新按钮 -->
          <div class="flex justify-end mb-1">
            <button @click="refreshActiveDetailTab()" class="min-h-[36px] px-3 rounded-full text-xs font-medium bg-bg text-ink-soft hover:bg-line border border-line transition-colors">刷新</button>
          </div>

          <!-- 子 Tab 切换条 -->
          <div class="flex gap-1 bg-surface rounded-xl p-1 mb-3">
            <button
              v-for="st in [{key:'instances',label:'角色实例'},{key:'knowledge',label:'知识'},{key:'tasks',label:'任务'},{key:'summaries',label:'摘要'}]"
              :key="st.key"
              @click="detailSubTab = st.key"
              class="flex-1 min-h-[44px] text-xs font-medium rounded-lg transition-colors"
              :class="detailSubTab === st.key ? 'bg-bg text-accent shadow-sm' : 'text-ink-soft'"
            >{{ st.label }}</button>
          </div>

          <!-- ▸ 子 Tab: 角色实例 -->
          <CampaignInstancesTab
            v-if="detailSubTab === 'instances'"
            ref="instancesTabRef"
            :campaign-id="selectedCampaignId"
          />

          <!-- ▸ 子 Tab: 知识 -->
          <CampaignKnowledgeTab
            v-if="detailSubTab === 'knowledge'"
            ref="knowledgeTabRef"
            :campaign-id="selectedCampaignId"
          />

          <!-- ▸ 子 Tab: 任务 -->
          <CampaignTasksTab
            v-if="detailSubTab === 'tasks'"
            ref="tasksTabRef"
            :campaign-id="selectedCampaignId"
          />

          <!-- ▸ 子 Tab: 摘要 -->
          <CampaignSummariesTab
            v-if="detailSubTab === 'summaries'"
            ref="summariesTabRef"
            :campaign-id="selectedCampaignId"
          />
        </template>
      </template>
    </div>
  </BaseOverlay>
</template>
