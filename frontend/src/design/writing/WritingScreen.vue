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

const props = defineProps({
  title: { type: String, default: '' },
  durationText: { type: String, default: '' },
  messages: { type: Array, default: () => [] },
  isWriting: { type: Boolean, default: false },
  pipeline: { type: Object, default: () => ({}) },
  showPipeline: { type: Boolean, default: false },
  streamingRoleLabel: { type: String, default: 'AI' },
  canBranch: { type: Boolean, default: false },
  greetingOptions: { type: Array, default: () => [] },
  selectedGreetingIndex: { type: Number, default: 0 },
  composerDisabled: { type: Boolean, default: false },
  composerPlaceholder: { type: String, default: '' },
  qualityAcceptHint: { type: String, default: null },
  contentComponent: { type: [Object, Function, String], default: null },
  subagentRolesByMessage: { type: Object, default: () => ({}) },
})

const emit = defineEmits([
  'start-writing',
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

const isEmpty = computed(
  () => props.messages.length === 0 && !props.isWriting && props.greetingOptions.length === 0,
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
            :quality-accept-hint="qualityAcceptHint"
            :content-component="contentComponent"
            :subagent-roles-by-message="subagentRolesByMessage"
            @cancel="emit('cancel')"
            @reroll="forward('reroll')"
            @reroll-user="forward('reroll-user')"
            @switch-variant="forward('switch-variant')"
            @edit-variant="forward('edit-variant')"
            @accept-variant="forward('accept-variant')"
            @delete-variant="forward('delete-variant')"
            @add-variant="forward('add-variant')"
            @branch="forward('branch')"
          />
          <div class="mx-auto w-full max-w-[760px] px-4 sm:px-8 pb-6">
            <ProcessTimeline v-if="hasReview" :pipeline="pipeline" />
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
          @start-writing="emit('start-writing', $event)"
          @cancel="emit('cancel')"
        />
      </div>
    </div>
  </div>
</template>
