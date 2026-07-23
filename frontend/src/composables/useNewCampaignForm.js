// 迁移自 App.vue:530-609 的新建 Campaign 表单。
// 表单专属局部 ref（newCampaignCards / newCampaignCardId / newCampaignCardDetail /
// newCampaignName / newCampaignGreetingIndex / creatingCampaign / showNewCampaignForm）
// 仅此表单使用，放 composable 内部，不进 store。
//
// 消费 campaignStore: activeCampaign（role_label 覆盖用）。
// 消费 writingStore: messages（applyConversation 后覆盖 assistant role_label）。
// 消费 uiStore: showHistory（建完后切到写作视图）。
//
// 范围外依赖（注入）：
// - loadInstanceNameMap()：刷新实例名映射（App.vue:42-54，未列入迁移清单）
// - applyConversation(conv)：把后端 Conversation 应用到前端 messages（App.vue:416-438）
// - broadcastPluginEvent(event, data)：插件事件广播
// - loadConversationHistory()：刷新会话历史列表（App.vue:441-453）
// - alertDialog(message)：错误弹窗（来自 components/base/BaseDialog.js）
// 未传时安全降级为 no-op（alertDialog 降级为 console.error）。

import { ref, computed } from 'vue'
import { useCampaignStore } from '../stores/campaign.js'
import { useWritingStore } from '../stores/writing.js'
import { useUiStore } from '../stores/ui.js'
import {
  listCards,
  getCard,
  createCampaign,
  setActiveCampaign,
  getActiveCampaign,
  getConversation,
} from '../tauri-api.js'
import { ST_EVENT_TYPES } from '../plugin-bridge.js'
import { buildGreetingOptionsFromDetail } from '../utils/campaignGreetingOptions.js'
import { preferredCampaignCard } from '../utils/campaignCardStatus.js'
import { assistantRoleLabel } from '../utils/roleLabel.js'

/**
 * @param {{
 *   loadInstanceNameMap?: () => Promise<void> | void,
 *   applyConversation?: (conv: object) => void,
 *   broadcastPluginEvent?: (event: string, data?: object) => void,
 *   loadConversationHistory?: () => Promise<void> | void,
 *   openingShellStarted?: (conversationId: string | null) => void,
 *   alertDialog?: (message: string) => Promise<void> | void,
 * }} [options]
 */
export function useNewCampaignForm(options = {}) {
  const campaignStore = useCampaignStore()
  const writingStore = useWritingStore()
  const uiStore = useUiStore()

  const loadInstanceNameMap = options.loadInstanceNameMap || (() => {})
  const applyConversation = options.applyConversation || (() => {})
  const broadcastPluginEvent = options.broadcastPluginEvent || (() => {})
  const loadConversationHistory = options.loadConversationHistory || (() => {})
  const openingShellStarted = options.openingShellStarted || (() => {})
  const alertDialog = options.alertDialog || ((msg) => { console.error('alertDialog(未注入):', msg) })

  // 来源 App.vue:530-538
  const showNewCampaignForm = ref(false)
  const newCampaignCards = ref([])
  const newCampaignCardId = ref(null)
  const newCampaignCardDetail = ref(null)
  const newCampaignName = ref('')
  const newCampaignGreetingIndex = ref(0)
  const creatingCampaign = ref(false)
  const newCampaignGreetingOptions = computed(() => buildGreetingOptionsFromDetail(newCampaignCardDetail.value))
  const selectedNewCampaignGreeting = computed(() => newCampaignGreetingOptions.value[newCampaignGreetingIndex.value] || null)

  // 来源 App.vue:540-544
  function normalizeNewCampaignGreetingSelection() {
    if (newCampaignGreetingIndex.value >= newCampaignGreetingOptions.value.length) {
      newCampaignGreetingIndex.value = 0
    }
  }

  // 来源 App.vue:546-556
  async function loadNewCampaignCardDetail() {
    newCampaignCardDetail.value = null
    newCampaignGreetingIndex.value = 0
    if (!newCampaignCardId.value) return
    try {
      newCampaignCardDetail.value = await getCard(newCampaignCardId.value)
      normalizeNewCampaignGreetingSelection()
    } catch (e) {
      console.error('加载 Campaign 开场白失败:', e)
    }
  }

  // 来源 App.vue:558-570
  async function openNewCampaignDialog() {
    showNewCampaignForm.value = true
    newCampaignName.value = ''
    newCampaignCardDetail.value = null
    newCampaignGreetingIndex.value = 0
    try {
      newCampaignCards.value = await listCards()
      // 默认优先选已完整识别的卡，其次选已有可用 definitions 的降级/历史卡。
      const selectedCard = preferredCampaignCard(newCampaignCards.value)
      newCampaignCardId.value = selectedCard?.id || null
      await loadNewCampaignCardDetail()
    } catch (e) {
      console.error('加载角色卡列表失败:', e)
    }
  }

  // 来源 App.vue:572-609
  async function handleCreateCampaign() {
    if (!newCampaignCardId.value || !newCampaignName.value.trim()) return
    creatingCampaign.value = true
    try {
      const result = await createCampaign(
        newCampaignCardId.value,
        newCampaignName.value.trim(),
        selectedNewCampaignGreeting.value?.content || null,
      )
      await setActiveCampaign(result.id)
      campaignStore.activeCampaign = await getActiveCampaign()
      await loadInstanceNameMap()
      showNewCampaignForm.value = false
      // 加载该 Campaign 绑定的对话（create_campaign 已自动建+开场白）
      if (result.conversation_id) {
        const conv = await getConversation(result.conversation_id)
        if (conv) {
          applyConversation(conv)
          // role_label 用 Campaign 名（原 App.vue:591-593 messages.value.forEach）
          const campaignName = campaignStore.activeCampaign?.name
          writingStore.messages.forEach((m) => {
            if (m.role === 'assistant') {
              m.role_label = assistantRoleLabel('campaign', campaignName)
            }
          })
          broadcastPluginEvent(ST_EVENT_TYPES.CHAT_LOADED, {
            conversationId: result.conversation_id,
            campaignId: result.id,
            // 原载荷含 reason: 'campaign_created'，由 chatEventPayload 在 App.vue 组装；
            // 此处仅传语义字段，调用方如需完整 chatEventPayload 可在注入的 broadcastPluginEvent 内补齐。
            reason: 'campaign_created',
          })
        }
      }
      openingShellStarted(result.conversation_id || null)
      uiStore.showHistory = false // 切到写作视图
      await loadConversationHistory()
    } catch (e) {
      console.error('创建 Campaign 失败:', e)
      await alertDialog('创建 Campaign 失败: ' + e)
    } finally {
      creatingCampaign.value = false
    }
  }

  return {
    // state（模板绑定用）
    showNewCampaignForm,
    newCampaignCards,
    newCampaignCardId,
    newCampaignCardDetail,
    newCampaignName,
    newCampaignGreetingIndex,
    creatingCampaign,
    newCampaignGreetingOptions,
    selectedNewCampaignGreeting,
    // functions
    normalizeNewCampaignGreetingSelection,
    loadNewCampaignCardDetail,
    openNewCampaignDialog,
    handleCreateCampaign,
  }
}
