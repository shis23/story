<script setup>
import { ref, computed } from 'vue'
import PanelHost from '../shell/PanelHost.vue'
import MetaChat from './MetaChat.vue'
import PatchPreview from './PatchPreview.vue'
import HealthCheckPanel from './HealthCheckPanel.vue'
import MvuAnalyzer from './MvuAnalyzer.vue'
import GenerationExplanation from './GenerationExplanation.vue'
import MetaScreen from '../../design/meta/MetaScreen.vue'

// 契约红线：
//   props:  activeCampaign / lastConversationNode
//   emits:  close / mvu-applied
//
// 对话 tab：fillContent → MetaChat 消息区 flex 占满、输入条贴抽屉底。
// 其它 tab：MetaScreen 内容井滚动 + 底部留白。

const props = defineProps({
  activeCampaign: { type: Object, default: null },
  lastConversationNode: { type: Object, default: null },
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
const isChat = computed(() => activeTab.value === 'chat')

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
  <PanelHost
    :show="true"
    title=""
    side="left"
    :show-chrome="false"
    :body-scroll="false"
    @close="emit('close')"
  >
    <MetaScreen
      class="h-full w-full"
      :active-tab="activeTab"
      :tabs="tabs"
      :pending-patch-count="pendingPatchCount"
      :global-error="globalError"
      :campaign-name="campaignName"
      :fill-content="isChat"
      @close="emit('close')"
      @change-tab="activeTab = $event"
    >
      <!-- 对话：占满剩余高度，输入贴底 -->
      <div
        v-show="activeTab === 'chat'"
        class="h-full min-h-0 w-full flex flex-col"
      >
        <MetaChat
          ref="chatRef"
          class="h-full min-h-0"
          @error="onError"
          @new-patch="onNewPatch"
          @new-typed-patches="onNewTypedPatches"
        />
      </div>

      <div v-show="activeTab === 'patches'" class="w-full min-w-0">
        <PatchPreview
          ref="patchPreviewRef"
          @error="onError"
          @patch-count-change="onPatchCountChange"
        />
      </div>

      <div v-show="activeTab === 'health'" class="w-full min-w-0">
        <HealthCheckPanel
          ref="healthCheckRef"
          :active-campaign="activeCampaign"
          @error="onError"
        />
      </div>

      <div v-show="activeTab === 'mvu'" class="w-full min-w-0">
        <MvuAnalyzer
          ref="mvuRef"
          @error="onError"
          @mvu-applied="onMvuApplied"
        />
      </div>

      <div v-show="activeTab === 'explain'" class="w-full min-w-0">
        <GenerationExplanation
          :last-conversation-node="lastConversationNode"
          @error="onError"
        />
      </div>
    </MetaScreen>
  </PanelHost>
</template>
