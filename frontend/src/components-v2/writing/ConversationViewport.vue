<script setup>
/**
 * ConversationViewport — 写作消息流视图组装（v2）
 *
 * 组合关系：
 *   - 读 writingStore：messages / isWriting / showPipeline / pipeline /
 *                      writingMode / canChooseGreeting / greetingOptions /
 *                      selectedGreetingIndex / streamingRoleLabel
 *   - 读 campaignStore：currentConversationId / activeCampaign
 *   - 读 uiStore：powerMode（预留，目前未渲染依赖项）
 *
 * 内部组装：
 *   - 空态（messages.length === 0）：EmptyState（文案"描述你要写的场景，开始第一轮"）
 *   - 开场白选择（canChooseGreeting）：GreetingSelector
 *   - 消息列表：v-for ChatMessage（8 个 emit 转发给 useMessageVariants 的 handler，由父级注入）
 *   - 流式：showPipeline 时 StreamingMessage
 *   - ref 消息容器 + scrollToBottom（新消息/写作完成时调用）
 */
import { ref, watch, nextTick, onMounted } from 'vue'
import { useWritingStore, useCampaignStore, useUiStore } from '../../stores/index.js'
import ChatMessage from './ChatMessage.vue'
import StreamingMessage from './StreamingMessage.vue'
import GreetingSelector from './GreetingSelector.vue'
import EmptyState from '../ui/EmptyState.vue'

const writing = useWritingStore()
const campaign = useCampaignStore()
const ui = useUiStore()

/**
 * 变体操作 handler 由 AppV2 注入（来自 useMessageVariants 的 8 个函数）。
 * 未注入时安全降级为 no-op，保证组件可独立渲染（用于测试 / 预览）。
 */
const props = defineProps({
  handlers: {
    type: Object,
    default: () => ({
      handleReroll: () => {},
      handleRerollUser: () => {},
      handleEditVariant: () => {},
      handleAcceptVariant: () => {},
      handleDeleteVariant: () => {},
      handleAddVariant: () => {},
      handleSwitchVariant: () => {},
      handleBranch: () => {},
    }),
  },
  /** 开场白选择回调（来自 useGreeting.selectGreeting） */
  onSelectGreeting: { type: Function, default: () => {} },
})

// 消息列表容器 ref（自动滚动用）
const messagesContainer = ref(null)

function scrollToBottom() {
  nextTick(() => {
    if (messagesContainer.value) {
      messagesContainer.value.scrollTop = messagesContainer.value.scrollHeight
    }
  })
}

// 暴露给父级（AppV2 的 useWriting / usePipeline 的 scrollToBottom 注入需要）
defineExpose({ scrollToBottom })

// canBranch：Campaign 模式 + 有活跃 Campaign + 有当前对话
const canBranch = (m) =>
  writing.writingMode === 'campaign' &&
  !!campaign.activeCampaign &&
  !!campaign.currentConversationId

// 新消息追加 / 写作完成 / 流式开始时滚动到底部
watch(
  () => writing.messages.length,
  () => scrollToBottom(),
)
watch(
  () => writing.isWriting,
  (now, prev) => {
    // 写作完成（running → false）时滚动
    if (prev && !now) scrollToBottom()
  },
)
watch(
  () => writing.showPipeline,
  () => scrollToBottom(),
)

onMounted(() => scrollToBottom())
</script>

<template>
  <div ref="messagesContainer" class="mx-auto max-w-2xl px-4 sm:px-6">
    <!-- 开场白选择条 -->
    <GreetingSelector
      v-if="writing.canChooseGreeting"
      :options="writing.greetingOptions"
      :selected-index="writing.selectedGreetingIndex"
      @select="onSelectGreeting"
    />

    <!-- 空态 -->
    <EmptyState
      v-if="writing.messages.length === 0"
      description="描述你要写的场景，开始第一轮"
    >
      <template #icon>
        <div class="text-4xl opacity-40">✦</div>
      </template>
    </EmptyState>

    <!-- 消息列表 -->
    <ChatMessage
      v-for="m in writing.messages"
      :key="m.id"
      :message="m"
      :conversation-id="campaign.currentConversationId"
      :busy="writing.isWriting"
      :can-branch="canBranch(m)"
      @reroll="props.handlers.handleReroll"
      @reroll-user="props.handlers.handleRerollUser"
      @edit-variant="props.handlers.handleEditVariant"
      @accept-variant="props.handlers.handleAcceptVariant"
      @delete-variant="props.handlers.handleDeleteVariant"
      @branch="props.handlers.handleBranch"
      @add-variant="props.handlers.handleAddVariant"
      @switch-variant="props.handlers.handleSwitchVariant"
    />

    <!-- 写作进行时：过程流式（Director/子Agent折叠 + Editor逐字），全在最后一条消息 -->
    <StreamingMessage
      v-if="writing.showPipeline"
      :pipeline="writing.pipeline"
      :role-label="writing.streamingRoleLabel"
    />
    <div class="h-4"></div>
  </div>
</template>
