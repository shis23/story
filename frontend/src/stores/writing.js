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

  // pipeline 状态机(App.vue:285-292)— 结构冻结,StreamingMessage 直接消费
  const pipeline = reactive({
    state: 'idle',
    stateLabel: '',
    director: { status: 'idle', detail: '', output: '' },
    subagents: [],
    editor: { status: 'idle', detail: '', output: '' },
    postprocess: { status: 'idle', detail: '', knowledge: 0, variable: 0, task: 0, reason: '' },
  })

  // 写作模式(App.vue:307-311)
  // useCampaignStore() 在 getter 内部调用——此时 pinia 已激活。
  const writingMode = computed(() => {
    const campaign = useCampaignStore()
    if (campaign.activeCampaign) return 'campaign'
    if (campaign.activeChar) return 'legacy'
    return 'none'
  })

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

  return {
    messages,
    isWriting,
    showPipeline,
    activeConnection,
    selectedGreetingIndex,
    pipeline,
    writingMode,
    streamingRoleLabel,
    greetingOptions,
    selectedGreeting,
    canChooseGreeting,
  }
})
