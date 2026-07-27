<script setup>
/**
 * WritingScreen — 写作主屏（重设计 · 纯展示 · 零 store 依赖）。
 *
 * 结构（对齐 selected 图②③）：
 *   - 空态：EmptyHero
 *   - 开场白：GreetingCards
 *   - 写作/成文：StoryPage 稿纸 + ProcessTimeline
 *   - 底部：ComposerBar
 *
 * 全部数据经 props 注入，全部意图经 events 抛出（契约见 CONTRACT.md）。
 * 暴露 scrollToBottom() 给 adapter / 生产注入链（签名与旧 ConversationViewport 一致）。
 */
import { ref, computed, nextTick } from 'vue'
import StoryPage from './StoryPage.vue'
import ProcessTimeline from './ProcessTimeline.vue'
import ComposerBar from './ComposerBar.vue'
import EmptyHero from './EmptyHero.vue'
import GreetingCards from './GreetingCards.vue'
import { shouldShowEmptyWritingState } from '../../utils/cardShellPresentation.js'

const props = defineProps({
  title: { type: String, default: '' },
  durationText: { type: String, default: '' },
  messages: { type: Array, default: () => [] },
  isWriting: { type: Boolean, default: false },
  pipeline: { type: Object, default: () => ({}) },
  showPipeline: { type: Boolean, default: false },
  streamingRoleLabel: { type: String, default: 'AI' },
  canBranch: { type: Boolean, default: false },
  allowPartialReroll: { type: Boolean, default: true },
  greetingOptions: { type: Array, default: () => [] },
  selectedGreetingIndex: { type: Number, default: 0 },
  composerDisabled: { type: Boolean, default: false },
  composerPlaceholder: { type: String, default: '' },
  generationMode: { type: String, default: 'continuation' },
  showGenerationModes: { type: Boolean, default: false },
  pendingReceipt: { type: Object, default: null },
  qualityAcceptHint: { type: String, default: null },
  contentComponent: { type: [Object, Function, String], default: null },
  subagentRolesByMessage: { type: Object, default: () => ({}) },
  /** Whether the caller has an active setup-only opening shell to render. */
  showOpening: { type: Boolean, default: false },
})

const emit = defineEmits([
  'start-writing',
  'update-generation-mode',
  'cancel',
  'import',
  'new-campaign',
  'view-history',
  'select-greeting',
  'reroll',
  'reroll-user',
  'switch-variant',
  'edit-variant',
  'accept-variant',
  'retry-postprocess',
  'dismiss-receipt',
  'delete-variant',
  'add-variant',
  'branch',
])

const scrollEl = ref(null)

function scrollToBottom() {
  nextTick(() => {
    if (scrollEl.value) {
      scrollEl.value.scrollTop = scrollEl.value.scrollHeight
    }
  })
}

defineExpose({ scrollToBottom })

const isEmpty = computed(() =>
  shouldShowEmptyWritingState({
    showOpening: props.showOpening,
    messageCount: props.messages.length,
    isWriting: props.isWriting,
    greetingCount: props.greetingOptions.length,
  }),
)

const showStory = computed(
  () => props.messages.length > 0 || props.isWriting || props.greetingOptions.length === 0,
)

const hasReview = computed(() => {
  if (props.isWriting) return false
  const p = props.pipeline
  return (
    p?.state === 'done' &&
    (p.director?.output || p.editor?.output || (p.subagents || []).length || p.quality)
  )
})

const forward = (name) => (p) => emit(name, p)
</script>

<template>
  <div class="h-full flex flex-col bg-bg">
    <div ref="scrollEl" class="flex-1 overflow-y-auto">
      <EmptyHero
        v-if="isEmpty"
        class="mx-auto w-full max-w-[760px] px-4 sm:px-8"
        @import="emit('import')"
        @new-campaign="emit('new-campaign')"
        @view-history="emit('view-history')"
      />

      <template v-else>
        <!-- 开场白可与消息并存（legacy 未建会话时） -->
        <div v-if="greetingOptions.length" class="mx-auto w-full max-w-[760px] px-4 sm:px-8 pt-6 pb-2">
          <GreetingCards
            :options="greetingOptions"
            :selected-index="selectedGreetingIndex"
            @select="emit('select-greeting', $event)"
          />
        </div>

        <!-- 完整开场独立成章，置于故事稿纸之前；design 层不 import 功能组件。 -->
        <div v-if="showOpening && $slots.opening" class="mx-auto w-full max-w-[980px] px-4 sm:px-8 pt-6 pb-2">
          <slot name="opening" />
        </div>

        <template v-if="showStory">
          <StoryPage
            :title="title"
            :duration-text="durationText"
            :messages="messages"
            :is-writing="isWriting"
            :pipeline="pipeline"
            :show-pipeline="showPipeline"
            :streaming-role-label="streamingRoleLabel"
            :can-branch="canBranch"
            :allow-partial-reroll="allowPartialReroll"
            :generation-mode="generationMode"
            :quality-accept-hint="qualityAcceptHint"
            :pending-receipt="pendingReceipt"
            :content-component="contentComponent"
            :subagent-roles-by-message="subagentRolesByMessage"
            @cancel="emit('cancel')"
            @reroll="forward('reroll')"
            @reroll-user="forward('reroll-user')"
            @switch-variant="forward('switch-variant')"
            @edit-variant="forward('edit-variant')"
            @accept-variant="forward('accept-variant')"
            @retry-postprocess="forward('retry-postprocess')"
            @dismiss-receipt="forward('dismiss-receipt')"
            @delete-variant="forward('delete-variant')"
            @add-variant="forward('add-variant')"
            @branch="forward('branch')"
          >
            <template #after-messages>
              <slot name="after-messages" />
            </template>
          </StoryPage>
          <div class="mx-auto w-full max-w-[760px] px-4 sm:px-8 pb-6">
            <ProcessTimeline v-if="hasReview" :pipeline="pipeline" :generation-mode="generationMode" />
          </div>
        </template>
      </template>
    </div>

    <div class="shrink-0">
      <div class="mx-auto w-full max-w-[760px] px-4 sm:px-8 pb-4">
        <ComposerBar
          :writing="isWriting"
          :disabled="composerDisabled"
          :placeholder="composerPlaceholder"
          :generation-mode="generationMode"
          :show-generation-modes="showGenerationModes"
          @start-writing="emit('start-writing', $event)"
          @update-generation-mode="emit('update-generation-mode', $event)"
          @cancel="emit('cancel')"
        />
      </div>
    </div>
  </div>
</template>
