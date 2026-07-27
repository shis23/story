import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { useCampaignStore } from './campaign.js'
import { useWritingStore } from './writing.js'
import { DETAIL_SUB_TABS } from '../utils/campaignTabRefresh.js'

// UI 状态:面板开关、视图路由、全局开关。
// 来源 App.vue:25-28, 33-36, 63-66, 70-83, 92-94, 336-340。
export const useUiStore = defineStore('ui', () => {
  const powerMode = ref(false) // App.vue:25 — 高玩模式总闸
  const appVersion = ref('...') // App.vue:28
  const importError = ref('') // App.vue:33 — 导入错误临时提示
  const extracting = ref(null) // 导入后自动抽取的进度状态：{ id, name } | null

  // 面板开关(全 v-if)
  const showCharList = ref(false) // App.vue:34
  const showCampaignPanel = ref(false) // App.vue:35
  const campaignPanelTab = ref('instances')
  const showMetaPanel = ref(false) // App.vue:36
  const showPresetPanel = ref(false) // App.vue:63
  const showPluginPanel = ref(false) // App.vue:64
  const showDebugDrawer = ref(false) // App.vue:65
  const showSidebar = ref(false) // App.vue:66 — 移动端左导航抽屉
  const showConnConfig = ref(false) // App.vue:340
  const showAgentProfile = ref(false) // AgentProfile 管理面板（Phase 8 补挂,P3-5）
  const showHistory = ref(true) // App.vue:336
  const activeCampaignOverview = ref(true) // App.vue:338

  // getter:当前中间栏视图(App.vue:70-74)
  const currentView = computed(() => {
    const campaign = useCampaignStore()
    if (showHistory.value && campaign.activeCampaign && activeCampaignOverview.value)
      return 'overview'
    if (showHistory.value) return 'history'
    return 'write'
  })

  // getter:中间栏标题(App.vue:77-83)
  const pageTitle = computed(() => {
    const campaign = useCampaignStore()
    const writing = useWritingStore()
    if (currentView.value === 'overview')
      return campaign.activeCampaign?.name || 'Campaign'
    if (currentView.value === 'history') return '会话历史'
    if (writing.writingMode === 'campaign')
      return campaign.activeCampaign?.name || 'Campaign 写作'
    if (writing.writingMode === 'legacy') return campaign.activeChar?.name || '写作'
    return 'StoryForge'
  })

  // actions:视图切换(App.vue:92-94)
  // viewWrite(94)是死代码(声明后未被调用),不迁移。
  function viewHistory() {
    showHistory.value = true
    activeCampaignOverview.value = false
  }
  function viewOverview() {
    showHistory.value = true
    activeCampaignOverview.value = true
  }
  function viewWrite() {
    showHistory.value = false
  }

  function openCampaignPanel(tab = 'instances') {
    campaignPanelTab.value = DETAIL_SUB_TABS.includes(tab) ? tab : 'instances'
    showCampaignPanel.value = true
  }

  // action:高玩模式切换(App.vue:97-100)
  function togglePower() {
    powerMode.value = !powerMode.value
    if (powerMode.value) showDebugDrawer.value = true
  }

  return {
    // state
    powerMode,
    appVersion,
    importError,
    extracting,
    showCharList,
    showCampaignPanel,
    campaignPanelTab,
    showMetaPanel,
    showPresetPanel,
    showPluginPanel,
    showDebugDrawer,
    showSidebar,
    showConnConfig,
    showAgentProfile,
    showHistory,
    activeCampaignOverview,
    // getters
    currentView,
    pageTitle,
    // actions
    viewHistory,
    viewOverview,
    viewWrite,
    openCampaignPanel,
    togglePower,
  }
})
