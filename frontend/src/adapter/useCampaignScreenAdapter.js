/**
 * useCampaignScreenAdapter — design/campaign 数据装配脚手架。
 *
 * 当前生产仍挂 `components-v2/campaign/CampaignPanel.vue`（含 MVU 实例编辑、
 * extract、refreshActiveDetailTab 红线）。本 adapter 供预览与后续整屏切换；
 * 切换时必须保留 Meta mvu-applied → refreshActiveDetailTab 链。
 */
import { ref, computed, watch } from 'vue'
import {
  listCampaigns,
  getActiveCampaign,
  setActiveCampaign,
  listInstances,
  getCampaign,
} from '../tauri-api.js'

export function useCampaignScreenAdapter(handlers = {}) {
  const campaigns = ref([])
  const selectedCampaignId = ref(null)
  const selectedCampaign = ref(null)
  const loadingCampaigns = ref(false)
  const loadingDetail = ref(false)
  const detailTab = ref('instances')
  const instances = ref([])
  const knowledge = ref([])
  const tasks = ref([])
  const summaries = ref([])
  const exportStatus = ref('')
  const importStatus = ref('')
  const exporting = ref(false)
  const importing = ref(false)

  async function refreshCampaigns() {
    loadingCampaigns.value = true
    try {
      campaigns.value = (await listCampaigns()) || []
      if (!selectedCampaignId.value) {
        const active = await getActiveCampaign()
        if (active?.id) {
          selectedCampaignId.value = active.id
          selectedCampaign.value = active
        }
      }
    } finally {
      loadingCampaigns.value = false
    }
  }

  async function loadDetail() {
    if (!selectedCampaignId.value) {
      selectedCampaign.value = null
      instances.value = []
      knowledge.value = []
      tasks.value = []
      summaries.value = []
      return
    }
    loadingDetail.value = true
    try {
      const full = await getCampaign(selectedCampaignId.value).catch(() => null)
      selectedCampaign.value =
        full || campaigns.value.find((c) => c.id === selectedCampaignId.value) || selectedCampaign.value
      instances.value = (await listInstances(selectedCampaignId.value).catch(() => [])) || []
      // 知识/任务/摘要在完整切换前由旧 Tab 组件权威加载；此处保持空数组避免伪 API。
      knowledge.value = selectedCampaign.value?.knowledge || []
      tasks.value = selectedCampaign.value?.tasks || []
      summaries.value = selectedCampaign.value?.summaries || []
    } finally {
      loadingDetail.value = false
    }
  }

  watch(selectedCampaignId, () => {
    loadDetail().catch(() => {})
  })

  const screenProps = computed(() => ({
    campaigns: campaigns.value,
    selectedCampaignId: selectedCampaignId.value,
    selectedCampaign: selectedCampaign.value,
    loadingCampaigns: loadingCampaigns.value,
    detailTab: detailTab.value,
    instances: instances.value,
    knowledge: knowledge.value,
    tasks: tasks.value,
    summaries: summaries.value,
    loadingDetail: loadingDetail.value,
    exportStatus: exportStatus.value,
    importStatus: importStatus.value,
    exporting: exporting.value,
    importing: importing.value,
  }))

  const screenEvents = {
    close: () => handlers.onClose?.(),
    'select-campaign': (c) => {
      selectedCampaignId.value = c?.id || null
      selectedCampaign.value = c || null
    },
    'set-active': async (id) => {
      await setActiveCampaign(id)
      handlers.onCampaignChanged?.(await getActiveCampaign())
      await refreshCampaigns()
    },
    'change-tab': (tab) => {
      detailTab.value = tab
    },
    'new-campaign': () => handlers.onNewCampaign?.(),
    'export-st': () => handlers.onExportSt?.(selectedCampaignId.value),
    'export-bundle': () => handlers.onExportBundle?.(selectedCampaignId.value),
    'import-bundle': () => handlers.onImportBundle?.(),
    refresh: async () => {
      await refreshCampaigns()
      await loadDetail()
    },
  }

  /** Meta mvu-applied 兼容入口（整屏切换时挂到 ref） */
  function refreshActiveDetailTab() {
    return loadDetail()
  }

  return {
    screenProps,
    screenEvents,
    refreshCampaigns,
    refreshActiveDetailTab,
  }
}
