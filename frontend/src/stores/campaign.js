import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { findLastAssistantConversationNode } from '../utils/conversationNodes.js'
import { useWritingStore } from './writing.js'

// Campaign / 角色卡 / 会话历史状态。
// 来源 App.vue:31-41, 322-325, 335。
export const useCampaignStore = defineStore('campaign', () => {
  const activeChar = ref(null) // App.vue:31 — 当前活跃角色卡(legacy 模式)
  const activeCharDetail = ref(null) // App.vue:32 — 角色卡详情(含 alternate_greetings 等)
  const activeCampaign = ref(null) // App.vue:37 — 当前活跃 Campaign
  const instanceNameMap = ref({}) // App.vue:41 — instance_id → display_name
  const conversationHistory = ref([]) // App.vue:335 — 会话历史列表
  const currentConversationId = ref(null) // App.vue:322 — 当前对话 ID(重 roll 需要)

  // getter(App.vue:323-325)
  // useWritingStore() 在 getter 内部调用——此时 pinia 已激活。模块层 import 虽与
  // writing.js 形成循环,但只要不在 defineStore setup 顶层同步调用就不会触发死锁。
  const lastConversationNode = computed(() => {
    const writing = useWritingStore()
    return findLastAssistantConversationNode(
      writing.messages,
      currentConversationId.value,
    )
  })

  return {
    activeChar,
    activeCharDetail,
    activeCampaign,
    instanceNameMap,
    conversationHistory,
    currentConversationId,
    lastConversationNode,
  }
})
