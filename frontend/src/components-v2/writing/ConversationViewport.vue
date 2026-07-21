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
import ProcessReview from './ProcessReview.vue'
import GreetingSelector from './GreetingSelector.vue'

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
  /** 空态三入口（可选，默认 no-op；由 AppV2 接线到既有 composable / store） */
  onNewCampaign: { type: Function, default: () => {} },
  onImport: { type: Function, default: () => {} },
  onViewHistory: { type: Function, default: () => {} },
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
  <div ref="messagesContainer" class="mx-auto w-full max-w-[720px] px-4 sm:px-8 py-6">
    <!-- 开场白选择条 -->
    <GreetingSelector
      v-if="writing.canChooseGreeting"
      :options="writing.greetingOptions"
      :selected-index="writing.selectedGreetingIndex"
      @select="onSelectGreeting"
    />

    <!-- 空态英雄区（纸上编辑部：衬线大标题 + 三入口） -->
    <section v-if="writing.messages.length === 0" class="pt-12 pb-10 text-center">
      <div class="flex items-center justify-center gap-3 text-accent/70 mb-7" aria-hidden="true">
        <span class="h-px w-12 bg-accent-border"></span>
        <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M19.7 12.7a5.5 5.5 0 0 0-7.78-7.78L5.5 10.34V18.5h8.16z"/><path d="M15.5 8.5L3 21"/><path d="M17 14.5H9.5"/></svg>
        <span class="h-px w-12 bg-accent-border"></span>
      </div>
      <h1 class="font-semibold text-[26px] sm:text-[28px] leading-snug text-ink">开始你的故事</h1>
      <p class="mt-2.5 text-sm text-ink-soft">故事不是被发现的，而是被锻造的。</p>

      <div class="mt-9 grid sm:grid-cols-3 gap-3 text-left">
        <button
          class="group rounded-lg border border-line bg-surface p-4 shadow-card transition-all hover:border-accent-border hover:shadow-rise"
          @click="onImport"
        >
          <span class="text-ink-soft group-hover:text-accent transition-colors" aria-hidden="true">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M20 15v3.5a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V15"/><path d="M7.5 10.5L12 15l4.5-4.5"/><path d="M12 15V3.5"/></svg>
          </span>
          <div class="mt-2.5 text-sm font-medium text-ink">导入角色卡</div>
          <div class="mt-1 text-xs leading-relaxed text-ink-soft">从 PNG / JSON 卡开始一段新故事</div>
        </button>
        <button
          class="group rounded-lg border border-line bg-surface p-4 shadow-card transition-all hover:border-accent-border hover:shadow-rise"
          @click="onNewCampaign"
        >
          <span class="text-ink-soft group-hover:text-accent transition-colors" aria-hidden="true">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>
          </span>
          <div class="mt-2.5 text-sm font-medium text-ink">新建 Campaign</div>
          <div class="mt-1 text-xs leading-relaxed text-ink-soft">选卡与开场白，开启长篇连载</div>
        </button>
        <button
          class="group rounded-lg border border-line bg-surface p-4 shadow-card transition-all hover:border-accent-border hover:shadow-rise"
          @click="onViewHistory"
        >
          <span class="text-ink-soft group-hover:text-accent transition-colors" aria-hidden="true">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/></svg>
          </span>
          <div class="mt-2.5 text-sm font-medium text-ink">继续上次</div>
          <div class="mt-1 text-xs leading-relaxed text-ink-soft">打开会话历史，回到未完的情节</div>
        </button>
      </div>

      <p class="mt-8 text-xs text-ink-faint">在下方描述你要写的场景，或直接回车开始第一轮</p>
    </section>

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
    <!-- 写作完成后：过程回顾（P2-1 修复，独立于 showPipeline 挂载） -->
    <ProcessReview />
    <div class="h-4"></div>
  </div>
</template>
