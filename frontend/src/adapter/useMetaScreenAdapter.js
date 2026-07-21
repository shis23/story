/**
 * useMetaScreenAdapter — design/meta 壳 props 装配。
 * 业务子组件（chat/patch/health/mvu/explain）仍由 MetaPanel 托管以保留
 * mvu-applied / lastConversationNode 红线；本 adapter 供后续整壳切换。
 */
import { computed, ref } from 'vue'

export function useMetaScreenAdapter(options = {}) {
  const activeTab = ref(options.initialTab || 'chat')
  const pendingPatchCount = ref(0)
  const globalError = ref('')

  const tabs = [
    { key: 'chat', label: '对话' },
    { key: 'patches', label: 'Patch' },
    { key: 'health', label: '健康检查' },
    { key: 'mvu', label: 'MVU' },
    { key: 'explain', label: '生成解释' },
  ]

  const screenProps = computed(() => ({
    activeTab: activeTab.value,
    tabs,
    pendingPatchCount: pendingPatchCount.value,
    globalError: globalError.value,
    campaignName: options.getCampaignName?.() || '',
  }))

  const screenEvents = {
    close: () => options.onClose?.(),
    'change-tab': (tab) => {
      activeTab.value = tab
    },
  }

  return {
    screenProps,
    screenEvents,
    activeTab,
    pendingPatchCount,
    globalError,
  }
}
