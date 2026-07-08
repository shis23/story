<script setup>
import { ref, computed, onMounted } from 'vue'
import { alertDialog } from '../../components/base/BaseDialog.js'
import {
  getCard,
  listCampaigns, createCampaign, setActiveCampaign, getActiveCampaign,
  exportCampaignStCards, exportCampaignBundle, importCampaignBundle
} from '../../tauri-api.js'
import CampaignInstancesTab from './CampaignInstancesTab.vue'
import CampaignKnowledgeTab from './CampaignKnowledgeTab.vue'
import CampaignTasksTab from './CampaignTasksTab.vue'
import CampaignSummariesTab from './CampaignSummariesTab.vue'
import CardLibrary from './CardLibrary.vue'
import { buildGreetingOptionsFromDetail } from '../../utils/campaignGreetingOptions.js'
import { refreshSubTab, subTabRefKey } from '../../utils/campaignTabRefresh.js'
import { useCampaignStore } from '../../stores/campaign.js'
import PanelHost from '../shell/PanelHost.vue'
import Tabs from '../ui/Tabs.vue'
import SegmentedControl from '../ui/SegmentedControl.vue'
import Button from '../ui/Button.vue'
import Input from '../ui/Input.vue'
import Select from '../ui/Select.vue'
import Badge from '../ui/Badge.vue'
import EmptyState from '../ui/EmptyState.vue'

const emit = defineEmits(['close', 'campaign-changed'])

const campaignStore = useCampaignStore()

// ─── Tab 控制 ───
const activeTab = ref('cards') // 'cards' | 'campaigns' | 'detail'
const topTabs = [
  { key: 'cards', label: '角色卡' },
  { key: 'campaigns', label: '游玩档' },
  { key: 'detail', label: '档详情' },
]

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
const detailSubTab = ref('instances') // 'instances' | 'knowledge' | 'tasks' | 'summaries'
const detailSubTabs = [
  { label: '角色实例', value: 'instances' },
  { label: '知识', value: 'knowledge' },
  { label: '任务', value: 'tasks' },
  { label: '摘要', value: 'summaries' },
]

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
  // Cards 由 CardLibrary 自管(其 setup 内自动加载);此处只取活跃 Campaign
  activeCampaign.value = await getActiveCampaign()
})

// ─── Cards 操作(委托给 CardLibrary,导入后刷新) ───
async function refreshCards() {
  await cardLibraryRef.value?.refresh?.()
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
  // 同步到 store
  campaignStore.activeCampaign = activeCampaign.value
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

// ─── 暴露 refresh 给父组件(MetaPanel apply 后触发刷新) ───
defineExpose({ refreshActiveDetailTab })
</script>

<template>
  <PanelHost :show="true" title="Campaign 管理" side="left" @close="emit('close')">
    <!-- Tab 切换(PanelHost 已提供滚动容器) -->
    <div class="px-3 pt-3">
      <Tabs v-model="activeTab" :tabs="topTabs">
          <!-- 内容区 -->
          <div class="space-y-3">

            <!-- ═══ Tab 1: 角色卡 ═══ -->
            <template v-if="activeTab === 'cards'">
              <!-- 导入 Bundle 入口 -->
              <div class="flex items-center justify-end gap-2">
                <Button
                  variant="default"
                  size="sm"
                  :disabled="importingBundle"
                  @click="handleImportBundle"
                >{{ importingBundle ? '导入中…' : '导入 Bundle' }}</Button>
              </div>
              <div v-if="importStatus" class="text-xs text-ink-soft">{{ importStatus }}</div>

              <!-- CardLibrary 自管 loading/empty/列表/提取 -->
              <CardLibrary
                ref="cardLibraryRef"
                @open-campaigns="openCampaignsForCard"
              />
            </template>

            <!-- ═══ Tab 2: 游玩档 ═══ -->
            <template v-if="activeTab === 'campaigns'">
              <!-- 未选卡时提示选卡 -->
              <EmptyState
                v-if="!selectedCardId"
                title="请先选择角色卡"
                description="在「角色卡」tab 选择一张卡"
              />

              <template v-else>
                <LoadingState v-if="loadingCampaigns" />

                <EmptyState
                  v-else-if="campaigns.length === 0 && !showNewCampaign"
                  title="还没有游玩档"
                >
                  <template #action>
                    <Button variant="primary" size="md" @click="openNewCampaignForm">新建游玩档</Button>
                  </template>
                </EmptyState>

                <!-- 新建表单 -->
                <div v-if="showNewCampaign" class="bg-surface rounded-lg border border-line p-3 space-y-2">
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

                <!-- Campaign 列表 -->
                <div
                  v-for="camp in campaigns"
                  :key="camp.id"
                  class="bg-surface rounded-lg border overflow-hidden cursor-pointer transition-colors"
                  :class="activeCampaign?.id === camp.id ? 'border-accent' : 'border-line hover:border-accent-border'"
                  @click="openCampaignDetail(camp.id)"
                >
                  <div class="flex items-center gap-3 px-3 py-2.5">
                    <div class="flex-1 min-w-0">
                      <div class="flex items-center gap-2">
                        <span class="text-sm font-medium text-ink">{{ camp.name }}</span>
                        <Badge v-if="activeCampaign?.id === camp.id" variant="accent" size="sm">活跃</Badge>
                      </div>
                      <div class="text-xs text-ink-soft">{{ camp.instance_count }} 个角色实例</div>
                    </div>
                    <Button
                      v-if="activeCampaign?.id !== camp.id"
                      variant="default"
                      size="sm"
                      @click.stop="handleSetActive(camp.id)"
                    >设为活跃</Button>
                    <span class="text-ink-soft text-xs">→</span>
                  </div>
                </div>

                <!-- 新建按钮(有档时显示) -->
                <Button
                  v-if="campaigns.length > 0 && !showNewCampaign"
                  variant="default"
                  size="md"
                  class="w-full border-dashed"
                  @click="openNewCampaignForm"
                >+ 新建游玩档</Button>
              </template>
            </template>

            <!-- ═══ Tab 3: 档详情 ═══ -->
            <template v-if="activeTab === 'detail'">
              <!-- 未选档提示 -->
              <EmptyState
                v-if="!selectedCampaignId"
                title="请先选择游玩档"
                description="在「游玩档」tab 点击一个档"
              />

              <template v-else>
                <!-- 导出按钮组 -->
                <div class="bg-surface rounded-lg border border-line p-3 mb-3 space-y-2">
                  <div class="text-xs font-medium text-ink mb-1">导出</div>
                  <div class="flex gap-2">
                    <Button
                      variant="default"
                      size="sm"
                      class="flex-1"
                      :disabled="exporting"
                      @click="handleExportStCards()"
                    >{{ exporting ? '导出中…' : 'ST 卡 PNG' }}</Button>
                    <Button
                      variant="default"
                      size="sm"
                      class="flex-1"
                      :disabled="exporting"
                      @click="handleExportBundle()"
                    >{{ exporting ? '导出中…' : 'JSON Bundle' }}</Button>
                  </div>
                  <div v-if="exportStatus" class="text-xs text-ink-soft">{{ exportStatus }}</div>
                </div>

                <!-- 刷新按钮 -->
                <div class="flex justify-end mb-2">
                  <Button variant="default" size="sm" @click="refreshActiveDetailTab()">刷新</Button>
                </div>

                <!-- 子 Tab 切换条 -->
                <div class="mb-3">
                  <SegmentedControl v-model="detailSubTab" :options="detailSubTabs" />
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
        </Tabs>
      </div>
  </PanelHost>
</template>
