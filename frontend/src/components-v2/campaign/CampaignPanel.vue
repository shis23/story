<script setup>
import { ref, computed, onMounted } from 'vue'
import { alertDialog, confirmDialog } from '../../components/base/BaseDialog.js'
import {
  getCard,
  listCampaigns, createCampaign, deleteCampaign, setActiveCampaign, getActiveCampaign, getActiveTurnQuality,
  exportCampaignStCards, exportCampaignBundle, importCampaignBundle
} from '../../tauri-api.js'
import { useWritingStore } from '../../stores/writing.js'
import CampaignInstancesTab from './CampaignInstancesTab.vue'
import CampaignKnowledgeTab from './CampaignKnowledgeTab.vue'
import CampaignWorldInfoTab from './CampaignWorldInfoTab.vue'
import CampaignTasksTab from './CampaignTasksTab.vue'
import CampaignSummariesTab from './CampaignSummariesTab.vue'
import CardLibrary from './CardLibrary.vue'
import CardStudio from './CardStudio.vue'
import { buildGreetingOptionsFromDetail } from '../../utils/campaignGreetingOptions.js'
import { refreshSubTab, subTabRefKey } from '../../utils/campaignTabRefresh.js'
import { useCampaignStore } from '../../stores/campaign.js'
import PanelHost from '../shell/PanelHost.vue'
import Button from '../ui/Button.vue'
import Input from '../ui/Input.vue'
import Select from '../ui/Select.vue'
import EmptyState from '../ui/EmptyState.vue'
import CampaignScreen from '../../design/campaign/CampaignScreen.vue'

const emit = defineEmits(['close', 'campaign-changed'])

const campaignStore = useCampaignStore()
const writingStore = useWritingStore()

// ─── 壳模式：manage（双栏活动）| cards（角色卡库）───
const shellMode = ref('manage') // 'manage' | 'cards'
// cards 模式下的子视图：library | studio
const cardsView = ref('library')
const studioSeed = ref(null) // { characterId, brief? }
// 兼容旧 activeTab 语义：cards | campaigns | detail
const activeTab = ref('detail')

// ─── Cards 状态(CardLibrary 自管;此处仅持有 ref 用于导入后刷新) ───
const cardLibraryRef = ref(null)

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

// greeting 选项(用 index 作 value)
const greetingOptions = computed(() =>
  newCampaignGreetingOptions.value.map((o, i) => ({ value: i, label: o.label }))
)

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
const detailSubTab = ref('instances') // 'instances' | 'knowledge' | 'worldinfo' | 'tasks' | 'summaries'

const selectedCampaign = computed(
  () => campaigns.value.find((c) => c.id === selectedCampaignId.value) || activeCampaign.value,
)

// ─── 子组件 template refs ───
const instancesTabRef = ref(null)
const knowledgeTabRef = ref(null)
const worldInfoTabRef = ref(null)
const tasksTabRef = ref(null)
const summariesTabRef = ref(null)

// ─── 导出状态 ───
const exporting = ref(false)
const exportStatus = ref('')
const importingBundle = ref(false)
const importStatus = ref('')

// ─── 初始化 ───
onMounted(async () => {
  // Cards 由 CardLibrary 自管(其 setup 内自动加载);此处只取活跃 Campaign
  activeCampaign.value = await getActiveCampaign()
  // 若已有活跃 Campaign,根据其 card_id 回填 selectedCardId 并预加载游玩档,
  // 避免用户进入「游玩档」tab 看到空白(P1-4);且默认直入档详情(P2-5 入口扁平化,
  // 让用户可直接看到知识/任务/摘要后处理结果,无需 5 步嵌套导航)。
  if (activeCampaign.value?.card_id) {
    selectedCardId.value = activeCampaign.value.card_id
    selectedCampaignId.value = activeCampaign.value.id
    await loadSelectedCampaignCardDetail()
    activeTab.value = 'detail'
    shellMode.value = 'manage'
  }
  // 双栏列表展示全部活动（创建时仍用 selectedCardId 限定角色卡）
  const savedCardId = selectedCardId.value
  selectedCardId.value = null
  await refreshCampaigns()
  selectedCardId.value = savedCardId
  // 若按卡过滤列表为空但已有活跃档，至少保证选中项可见
  if (
    selectedCampaignId.value &&
    !campaigns.value.some((c) => c.id === selectedCampaignId.value) &&
    activeCampaign.value
  ) {
    campaigns.value = [activeCampaign.value, ...campaigns.value]
  }
})

// ─── Cards 操作(委托给 CardLibrary,导入后刷新) ───
async function refreshCards() {
  await cardLibraryRef.value?.refresh?.()
}

// ─── Campaigns 操作 ───
async function refreshCampaigns() {
  loadingCampaigns.value = true
  try {
    // 有选中角色卡时按卡过滤；否则列出全部（对齐 design 双栏「我的活动」）
    campaigns.value = await listCampaigns(selectedCardId.value || null)
  } finally {
    loadingCampaigns.value = false
  }
}

async function openCampaignsForCard(card) {
  selectedCardId.value = card.id
  shellMode.value = 'manage'
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
  // 同步到 store
  campaignStore.activeCampaign = activeCampaign.value
  // 回填活动 Turn 质量报告（若有）
  try {
    if (activeCampaign.value?.id) {
      const dto = await getActiveTurnQuality(activeCampaign.value.id)
      writingStore.applyQualityFromTurn(dto)
    }
  } catch (e) {
    console.error('getActiveTurnQuality:', e)
  }
  emit('campaign-changed', activeCampaign.value)
}

/** 删除整局活动（一活动一对话：级联会话 + 实例/知识/任务/总结） */
async function handleDeleteCampaign(camp) {
  const id = camp?.id || selectedCampaignId.value
  if (!id) return
  const label = camp?.name || selectedCampaign.value?.name || id.slice(0, 8)
  const ok = await confirmDialog(
    `确定删除「${label}」整局活动？\n\n将同时删除：对话正文、角色实例、知识、任务与总结。此操作不可恢复。`,
    { title: '删除整局活动' },
  )
  if (!ok) return
  try {
    await deleteCampaign(id)
    const wasSelected = selectedCampaignId.value === id
    const wasActive = activeCampaign.value?.id === id
      || campaignStore.activeCampaign?.id === id
    if (wasSelected) {
      selectedCampaignId.value = null
    }
    if (wasActive) {
      activeCampaign.value = null
      campaignStore.activeCampaign = null
      // 当前写作若绑在这局会话上，一并清空
      if (
        camp?.conversation_id
        && campaignStore.currentConversationId === camp.conversation_id
      ) {
        writingStore.messages = []
        campaignStore.currentConversationId = null
      }
    }
    await refreshCampaigns()
    activeCampaign.value = await getActiveCampaign()
    campaignStore.activeCampaign = activeCampaign.value
    if (!selectedCampaignId.value && activeCampaign.value?.id) {
      selectedCampaignId.value = activeCampaign.value.id
    }
    emit('campaign-changed', activeCampaign.value)
  } catch (e) {
    await alertDialog('删除活动失败: ' + e)
  }
}

async function openCampaignDetail(campaignId) {
  selectedCampaignId.value = campaignId
  shellMode.value = 'manage'
  activeTab.value = 'detail'
  // 子组件各自 onMounted 加载，不需要 refreshDetail 全拉
}

function onSelectCampaign(camp) {
  if (!camp?.id) return
  openCampaignDetail(camp.id)
}

function onChangeMode(mode) {
  shellMode.value = mode
  activeTab.value = mode === 'cards' ? 'cards' : (selectedCampaignId.value ? 'detail' : 'campaigns')
  if (mode !== 'cards') {
    cardsView.value = 'library'
    studioSeed.value = null
  }
}

function openStudioForRevise(card) {
  const characterId = card?.source_character_id || card?.id
  if (!characterId) return
  studioSeed.value = {
    characterId,
    brief: `修订角色卡：${card?.name || characterId}`,
    // ensure re-opening the same card always seeds a new revise project
    nonce: `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
  }
  shellMode.value = 'cards'
  activeTab.value = 'cards'
  cardsView.value = 'studio'
}

function onChangeDetailTab(tab) {
  detailSubTab.value = tab
  activeTab.value = 'detail'
}

async function onNewCampaignFromShell() {
  if (!selectedCardId.value) {
    shellMode.value = 'cards'
    activeTab.value = 'cards'
    return
  }
  await openNewCampaignForm()
}

// ─── 刷新当前活跃的 detail 子 tab ───
function refreshActiveDetailTab() {
  const refMap = {
    instances: instancesTabRef,
    knowledge: knowledgeTabRef,
    worldinfo: worldInfoTabRef,
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

// ─── 暴露 refresh 给父组件(MetaPanel apply 后触发刷新) ───
defineExpose({ refreshActiveDetailTab })
</script>

<template>
  <!-- 全宽活动管理（对齐 selected 图④）；深度 Tab 经 #detail 注入，保留 MVU/变量编辑 -->
  <PanelHost :show="true" title="" side="full" :show-chrome="false" :body-scroll="false" @close="emit('close')">
    <CampaignScreen
      class="h-full"
      :mode="shellMode"
      :campaigns="campaigns"
      :selected-campaign-id="selectedCampaignId"
      :selected-campaign="selectedCampaign"
      :loading-campaigns="loadingCampaigns"
      :detail-tab="detailSubTab"
      :export-status="exportStatus"
      :import-status="importStatus"
      :exporting="exporting"
      :importing="importingBundle"
      @close="emit('close')"
      @select-campaign="onSelectCampaign"
      @set-active="handleSetActive"
      @delete-campaign="handleDeleteCampaign"
      @change-tab="onChangeDetailTab"
      @change-mode="onChangeMode"
      @new-campaign="onNewCampaignFromShell"
      @export-st="handleExportStCards"
      @export-bundle="handleExportBundle"
      @import-bundle="handleImportBundle"
      @refresh="refreshActiveDetailTab"
    >
      <template #cards>
        <div class="space-y-3 max-w-4xl">
          <div v-if="importStatus" class="text-xs text-ink-soft">{{ importStatus }}</div>
          <CardStudio
            v-if="cardsView === 'studio'"
            :seed="studioSeed"
            @close="cardsView = 'library'; studioSeed = null"
            @imported="async () => { cardsView = 'library'; studioSeed = null; await refreshCards() }"
          />
          <CardLibrary
            v-else
            ref="cardLibraryRef"
            @open-campaigns="openCampaignsForCard"
            @open-studio="studioSeed = null; cardsView = 'studio'"
            @revise-card="openStudioForRevise"
          />
        </div>
      </template>

      <template #detail>
        <!-- 新建游玩档表单（有选中卡时） -->
        <div v-if="showNewCampaign" class="mb-4 rounded-xl border border-line bg-surface p-4 space-y-2 shadow-card">
          <div class="text-xs font-medium text-ink">新建游玩档</div>
          <div v-if="newCampaignGreetingOptions.length > 1">
            <label class="text-xs text-ink-soft mb-1 block">开场白</label>
            <Select v-model="newCampaignGreetingIndex" :options="greetingOptions" />
          </div>
          <Input
            v-model="newCampaignName"
            placeholder="输入档名（如：第一周目）"
            @keyup.enter="handleCreateCampaign"
          />
          <div class="flex gap-2">
            <Button variant="default" size="md" class="flex-1" @click="showNewCampaign = false">取消</Button>
            <Button
              variant="primary"
              size="md"
              class="flex-1"
              :disabled="creatingCampaign || !newCampaignName.trim()"
              :loading="creatingCampaign"
              @click="handleCreateCampaign"
            >{{ creatingCampaign ? '创建中…' : '创建' }}</Button>
          </div>
        </div>

        <EmptyState
          v-if="!selectedCampaignId"
          title="请先选择活动"
          description="从左侧列表选择，或新建一个游玩档"
        />

        <template v-else>
          <CampaignInstancesTab
            v-if="detailSubTab === 'instances'"
            ref="instancesTabRef"
            :campaign-id="selectedCampaignId"
          />
          <CampaignKnowledgeTab
            v-else-if="detailSubTab === 'knowledge'"
            ref="knowledgeTabRef"
            :campaign-id="selectedCampaignId"
          />
          <CampaignWorldInfoTab
            v-else-if="detailSubTab === 'worldinfo'"
            ref="worldInfoTabRef"
            :campaign-id="selectedCampaignId"
          />
          <CampaignTasksTab
            v-else-if="detailSubTab === 'tasks'"
            ref="tasksTabRef"
            :campaign-id="selectedCampaignId"
          />
          <CampaignSummariesTab
            v-else
            ref="summariesTabRef"
            :campaign-id="selectedCampaignId"
          />
        </template>
      </template>
    </CampaignScreen>
  </PanelHost>
</template>
