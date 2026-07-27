import { defineStore } from 'pinia'
import { ref, reactive, computed } from 'vue'
import { useCampaignStore } from './campaign.js'
import { buildGreetingOptionsFromDetail } from '../utils/campaignGreetingOptions.js'

// 写作 / 流水线运行时状态。
// 来源 App.vue:26-27, 285-292, 307-311, 314-317, 320-322, 326-333, 342。
export const useWritingStore = defineStore('writing', () => {
  const messages = ref([]) // App.vue:26 — 消息列表
  const isWriting = ref(false) // App.vue:320 — 流水线运行中(禁用重 roll / 显示停止)
  const showPipeline = ref(false) // App.vue:27 — 是否显示流水线过程
  const activeConnection = ref(null) // App.vue:342 — 当前活跃连接(顶栏显示用)
  const selectedGreetingIndex = ref(0) // App.vue:326 — 开场白选择索引
  const generationModeByCampaign = ref({})
  const pendingReceipt = ref(null)
  const validGenerationModes = new Set([
    'continuation',
    'duet',
    'sequential_crew',
    'big_scene',
  ])
  try {
    const saved = globalThis.localStorage?.getItem('storyforge:generation-mode-by-campaign')
    const parsed = saved ? JSON.parse(saved) : null
    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
      generationModeByCampaign.value = Object.fromEntries(
        Object.entries(parsed).filter(([, mode]) => validGenerationModes.has(mode)),
      )
    }
  } catch {
    // localStorage may be unavailable in tests/private contexts; in-memory scoping still works.
  }

  // pipeline 状态机(App.vue:285-292)— 结构冻结,StreamingMessage 直接消费
  const pipeline = reactive({
    state: 'idle',
    stateLabel: '',
    director: { status: 'idle', detail: '', output: '' },
    subagents: [],
    editor: { status: 'idle', detail: '', output: '' },
    postprocess: { status: 'idle', detail: '', knowledge: 0, variable: 0, task: 0, reason: '' },
    // B3 DraftQualityGate 结果（warn-only；null = 本轮尚未检查）
    quality: null,
  })

  // 写作模式(App.vue:307-311)
  // useCampaignStore() 在 getter 内部调用——此时 pinia 已激活。
  const writingMode = computed(() => {
    const campaign = useCampaignStore()
    if (campaign.activeCampaign) return 'campaign'
    if (campaign.activeChar) return 'legacy'
    return 'none'
  })

  // 手动档位只记在当前 Campaign 名下，不形成跨故事的全局偏好。
  const generationMode = computed(() => {
    const campaign = useCampaignStore()
    const campaignId = campaign.activeCampaign?.id
    if (!campaignId) return 'continuation'
    return generationModeByCampaign.value[campaignId] || 'continuation'
  })

  function setGenerationMode(mode) {
    const campaign = useCampaignStore()
    const campaignId = campaign.activeCampaign?.id
    if (!campaignId || !validGenerationModes.has(mode)) return
    generationModeByCampaign.value = {
      ...generationModeByCampaign.value,
      [campaignId]: mode,
    }
    try {
      globalThis.localStorage?.setItem(
        'storyforge:generation-mode-by-campaign',
        JSON.stringify(generationModeByCampaign.value),
      )
    } catch {
      // Preference persistence is best-effort and must never block writing.
    }
  }

  const selectedReceiptMutationIndices = computed(() => {
    const items = pendingReceipt.value?.items
    if (!Array.isArray(items)) return []
    return items
      .filter((item) => item.selected)
      .map((item) => item.mutation_index)
  })

  function openTurnReceipt(nodeId, dto) {
    if (!nodeId || !dto) return
    pendingReceipt.value = {
      ...dto,
      nodeId,
      items: Array.isArray(dto.items)
        ? dto.items.map((item) => ({
          ...item,
          selected: item.selected_by_default !== false,
        }))
        : [],
    }
  }

  function setReceiptItemSelected(mutationIndex, selected) {
    const item = pendingReceipt.value?.items?.find(
      (candidate) => candidate.mutation_index === mutationIndex,
    )
    if (item) item.selected = !!selected
  }

  function clearTurnReceipt() {
    pendingReceipt.value = null
  }

  // 流式消息角色标签(App.vue:86-90,合并 314-317 的重复逻辑)
  const streamingRoleLabel = computed(() => {
    const campaign = useCampaignStore()
    return writingMode.value === 'campaign'
      ? campaign.activeCampaign?.name || 'AI'
      : campaign.activeChar?.name || 'AI'
  })

  // 开场白选项(App.vue:327-328)
  const greetingOptions = computed(() => {
    const campaign = useCampaignStore()
    return buildGreetingOptionsFromDetail(campaign.activeCharDetail)
  })
  const selectedGreeting = computed(
    () => greetingOptions.value[selectedGreetingIndex.value] || null,
  )

  // 是否可选择开场白(App.vue:329-333)
  const canChooseGreeting = computed(() => {
    const campaign = useCampaignStore()
    return (
      writingMode.value === 'legacy' &&
      !campaign.currentConversationId &&
      greetingOptions.value.length > 1
    )
  })

  /** 用后端活动 Turn 的质量报告回填 pipeline.quality（刷新/重进后）。 */
  function applyQualityFromTurn(dto) {
    if (!dto) {
      pipeline.quality = null
      return
    }
    const warningCount = dto.warning_count ?? (dto.warnings?.length || 0)
    const errorCount = dto.error_count ?? 0
    const warnings = Array.isArray(dto.warnings) ? dto.warnings : []
    const passed = !!dto.passed
    pipeline.quality = {
      passed,
      warningCount,
      errorCount,
      warnings,
      status: passed ? 'ok' : errorCount > 0 ? 'error' : 'warn',
      source: 'turn',
    }
    if (!passed && warningCount > 0 && pipeline.state === 'done') {
      pipeline.stateLabel = `已产出 · 质量警告 ${warningCount}`
    }
  }

  return {
    messages,
    isWriting,
    showPipeline,
    activeConnection,
    selectedGreetingIndex,
    pipeline,
    writingMode,
    generationMode,
    setGenerationMode,
    pendingReceipt,
    selectedReceiptMutationIndices,
    openTurnReceipt,
    setReceiptItemSelected,
    clearTurnReceipt,
    streamingRoleLabel,
    greetingOptions,
    selectedGreeting,
    canChooseGreeting,
    applyQualityFromTurn,
  }
})
