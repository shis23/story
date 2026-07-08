<script setup>
import { ref } from 'vue'
import PanelHost from '../shell/PanelHost.vue'
import Tabs from '../ui/Tabs.vue'
import Badge from '../ui/Badge.vue'
import MetaChat from './MetaChat.vue'
import PatchPreview from './PatchPreview.vue'
import HealthCheckPanel from './HealthCheckPanel.vue'
import MvuAnalyzer from './MvuAnalyzer.vue'
import GenerationExplanation from './GenerationExplanation.vue'

// MetaPanel 容器（v2 重构，源自 src/components/MetaPanel.vue 826 行）。
//
// 契约红线（不可变）：
//   props:  activeCampaign(Object) / lastConversationNode({conversation_id, node_id})
//   emits:  close / mvu-applied
//   mvu-applied 由 MvuAnalyzer 触发后冒泡，最终调用 CampaignPanel.refreshActiveDetailTab。

const props = defineProps({
  activeCampaign: { type: Object, default: null },
  lastConversationNode: { type: Object, default: null }, // { conversation_id, node_id }
})

const emit = defineEmits(['close', 'mvu-applied'])

// ─── Tab 控制 ───
const activeTab = ref('chat')
const tabs = [
  { key: 'chat', label: '对话' },
  { key: 'patches', label: 'Patch' },
  { key: 'health', label: '健康检查' },
  { key: 'mvu', label: 'MVU' },
  { key: 'explain', label: '生成解释' },
]

// ─── 全局错误展示（子组件冒泡） ───
const globalError = ref('')

// ─── Patch 计数（tab badge）───
const pendingPatchCount = ref(0)

// ─── 子组件 refs（跨 tab 刷新） ───
const chatRef = ref(null)
const patchPreviewRef = ref(null)
const healthCheckRef = ref(null)
const mvuRef = ref(null)

// ─── 子 → 父 冒泡 ───
function onError(msg) {
  globalError.value = msg
}

function onPatchCountChange(count) {
  pendingPatchCount.value = count
}

// 对话产生新 patch → 刷新 PatchPreview
function onNewPatch() {
  patchPreviewRef.value?.refresh?.()
}

// 对话产生新 typed patch → 刷新 HealthCheckPanel 的修复建议列表
function onNewTypedPatches() {
  healthCheckRef.value?.refreshTypedPatches?.()
}

// MvuAnalyzer apply 成功 → 冒泡 mvu-applied 给 AppV2（触发 CampaignPanel.refreshActiveDetailTab）
function onMvuApplied() {
  emit('mvu-applied')
}
</script>

<template>
  <PanelHost :show="true" title="🔧 Meta 配置助手" side="left" @close="emit('close')">
    <div class="flex flex-col h-full">
      <!-- Tab 切换 + 全局错误/Patch 计数 -->
      <div class="shrink-0 px-3 pt-3">
        <Tabs v-model="activeTab" :tabs="tabs">
          <div class="flex items-center gap-2 mb-2 min-h-[16px]">
            <Badge v-if="pendingPatchCount > 0" variant="warn" size="sm">
              {{ pendingPatchCount }} 个 Patch 待采纳
            </Badge>
            <span v-if="globalError" class="text-xs text-err truncate flex-1">{{ globalError }}</span>
          </div>

          <!-- ═══ Tab: 对话 ═══ -->
          <!-- chat 区需占满高度（消息滚动 + 输入栏），用 viewport 高度减去标题栏 + tabs 行 + badge 行 -->
          <div v-show="activeTab === 'chat'" class="h-[calc(100vh-13rem)]">
            <MetaChat
              ref="chatRef"
              @error="onError"
              @new-patch="onNewPatch"
              @new-typed-patches="onNewTypedPatches"
            />
          </div>

          <!-- ═══ Tab: 待采纳 Patch ═══ -->
          <div v-show="activeTab === 'patches'">
            <PatchPreview
              ref="patchPreviewRef"
              @error="onError"
              @patch-count-change="onPatchCountChange"
            />
          </div>

          <!-- ═══ Tab: 健康检查 + 类型化修复 ═══ -->
          <div v-show="activeTab === 'health'">
            <HealthCheckPanel
              ref="healthCheckRef"
              :active-campaign="activeCampaign"
              @error="onError"
            />
          </div>

          <!-- ═══ Tab: MVU 分析 ═══ -->
          <div v-show="activeTab === 'mvu'">
            <MvuAnalyzer
              ref="mvuRef"
              @error="onError"
              @mvu-applied="onMvuApplied"
            />
          </div>

          <!-- ═══ Tab: 生成溯源 ═══ -->
          <div v-show="activeTab === 'explain'">
            <GenerationExplanation
              :last-conversation-node="lastConversationNode"
              @error="onError"
            />
          </div>
        </Tabs>
      </div>
    </div>
  </PanelHost>
</template>
