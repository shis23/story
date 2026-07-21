<script setup>
import { ref, computed } from 'vue'
import PanelHost from '../shell/PanelHost.vue'
import MetaChat from './MetaChat.vue'
import PatchPreview from './PatchPreview.vue'
import HealthCheckPanel from './HealthCheckPanel.vue'
import MvuAnalyzer from './MvuAnalyzer.vue'
import GenerationExplanation from './GenerationExplanation.vue'
import MetaScreen from '../../design/meta/MetaScreen.vue'

// MetaPanel 容器（v2 重构，源自 src/components/MetaPanel.vue 826 行）。
//
// 契约红线（不可变）：
//   props:  activeCampaign(Object) / lastConversationNode({conversation_id, node_id})
//   emits:  close / mvu-applied
//   mvu-applied 由 MvuAnalyzer 触发后冒泡，最终调用 CampaignPanel.refreshActiveDetailTab。
//
// 2026-07-22：外壳换为 design/meta/MetaScreen（纸面 tab 壳）；业务子组件不变。

const props = defineProps({
  activeCampaign: { type: Object, default: null },
  lastConversationNode: { type: Object, default: null }, // { conversation_id, node_id }
})

const emit = defineEmits(['close', 'mvu-applied'])

const activeTab = ref('chat')
const tabs = [
  { key: 'chat', label: '对话' },
  { key: 'patches', label: 'Patch' },
  { key: 'health', label: '健康检查' },
  { key: 'mvu', label: 'MVU' },
  { key: 'explain', label: '生成解释' },
]

const globalError = ref('')
const pendingPatchCount = ref(0)

const chatRef = ref(null)
const patchPreviewRef = ref(null)
const healthCheckRef = ref(null)
const mvuRef = ref(null)

const campaignName = computed(() => props.activeCampaign?.name || '')

function onError(msg) {
  globalError.value = msg
}

function onPatchCountChange(count) {
  pendingPatchCount.value = count
}

function onNewPatch() {
  patchPreviewRef.value?.refresh?.()
}

function onNewTypedPatches() {
  healthCheckRef.value?.refreshTypedPatches?.()
}

function onMvuApplied() {
  emit('mvu-applied')
}
</script>

<template>
  <PanelHost :show="true" title="" side="left" @close="emit('close')">
    <MetaScreen
      class="h-full"
      :active-tab="activeTab"
      :tabs="tabs"
      :pending-patch-count="pendingPatchCount"
      :global-error="globalError"
      :campaign-name="campaignName"
      @close="emit('close')"
      @change-tab="activeTab = $event"
    >
      <div v-show="activeTab === 'chat'" class="h-[calc(100vh-13rem)]">
        <MetaChat
          ref="chatRef"
          @error="onError"
          @new-patch="onNewPatch"
          @new-typed-patches="onNewTypedPatches"
        />
      </div>

      <div v-show="activeTab === 'patches'">
        <PatchPreview
          ref="patchPreviewRef"
          @error="onError"
          @patch-count-change="onPatchCountChange"
        />
      </div>

      <div v-show="activeTab === 'health'">
        <HealthCheckPanel
          ref="healthCheckRef"
          :active-campaign="activeCampaign"
          @error="onError"
        />
      </div>

      <div v-show="activeTab === 'mvu'">
        <MvuAnalyzer
          ref="mvuRef"
          @error="onError"
          @mvu-applied="onMvuApplied"
        />
      </div>

      <div v-show="activeTab === 'explain'">
        <GenerationExplanation
          :last-conversation-node="lastConversationNode"
          @error="onError"
        />
      </div>
    </MetaScreen>
  </PanelHost>
</template>
