<script setup>
import { ref, computed } from 'vue'
import PanelHost from '../shell/PanelHost.vue'
import MetaChat from './MetaChat.vue'
import PatchPreview from './PatchPreview.vue'
import HealthCheckPanel from './HealthCheckPanel.vue'
import MvuAnalyzer from './MvuAnalyzer.vue'
import GenerationExplanation from './GenerationExplanation.vue'
import MetaScreen from '../../design/meta/MetaScreen.vue'

// MetaPanel 容器
//
// 契约红线（不可变）：
//   props:  activeCampaign(Object) / lastConversationNode({conversation_id, node_id})
//   emits:  close / mvu-applied
//
// 2026-07-22：
//   - 外壳 design/meta/MetaScreen
//   - 固定抽屉宽 panelWidthClass，各 tab 内容长短不再跳宽
//   - showChrome=false，避免 PanelHost 空标题栏 + MetaScreen 顶栏双重 chrome

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
  <!-- 宽度走全局 --layout-drawer（Overlay 默认），与连接/预设/插件等侧栏抽屉一致 -->
  <PanelHost
    :show="true"
    title=""
    side="left"
    :show-chrome="false"
    @close="emit('close')"
  >
    <MetaScreen
      class="h-full w-full"
      :active-tab="activeTab"
      :tabs="tabs"
      :pending-patch-count="pendingPatchCount"
      :global-error="globalError"
      :campaign-name="campaignName"
      @close="emit('close')"
      @change-tab="activeTab = $event"
    >
      <!-- 每个 tab 同一容器约束：min-w-0 + 满宽，禁止 EmptyState 文案把壳撑开 -->
      <div
        v-show="activeTab === 'chat'"
        class="w-full min-w-0 h-[min(70vh,520px)] flex flex-col"
      >
        <MetaChat
          ref="chatRef"
          class="min-h-0 flex-1"
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
