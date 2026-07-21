<script setup>
/**
 * WritingScreen — 写作主屏（重设计 · 纯展示 · 零 store 依赖）。
 *
 * 结构（对齐 selected 图②③）：
 *   - 空态：EmptyHero 英雄区（图①）
 *   - 开场白：GreetingCards
 *   - 写作/成文：StoryPage 稿纸（页头题+元信息+状态、文稿小节、页脚生成状态）
 *     + ProcessTimeline 回顾（稿纸下方）
 *   - 底部：ComposerBar 指令条
 *
 * 全部数据经 props 注入，全部意图经 events 抛出（契约见 CONTRACT.md）。
 */
import { computed } from 'vue'
import StoryPage from './StoryPage.vue'
import ProcessTimeline from './ProcessTimeline.vue'
import ComposerBar from './ComposerBar.vue'
import EmptyHero from './EmptyHero.vue'
import GreetingCards from './GreetingCards.vue'

const props = defineProps({
  /** 页题（接线：Campaign 名 / 会话派生题） */
  title: { type: String, default: '' },
  /** 用时等补充元信息（可选） */
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

const isEmpty = computed(
  () => props.messages.length === 0 && !props.isWriting && props.greetingOptions.length === 0,
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
    <!-- 滚动区 -->
    <div class="flex-1 overflow-y-auto">
      <!-- 空态英雄区（图①） -->
      <EmptyHero
        v-if="isEmpty"
        class="mx-auto w-full max-w-[760px] px-4 sm:px-8"
        @import="emit('import')"
        @new-campaign="emit('new-campaign')"
        @view-history="emit('view-history')"
      />

      <!-- 开场白选择 -->
      <div v-else-if="greetingOptions.length" class="mx-auto w-full max-w-[760px] px-4 sm:px-8 py-8">
        <GreetingCards
          :options="greetingOptions"
          :selected-index="selectedGreetingIndex"
          @select="emit('select-greeting', $event)"
        />
      </div>

      <!-- 稿纸 + 回顾 -->
      <template v-else>
        <StoryPage
          :title="title"
          :duration-text="durationText"
          :messages="messages"
          :is-writing="isWriting"
          :pipeline="pipeline"
          :show-pipeline="showPipeline"
          :streaming-role-label="streamingRoleLabel"
          :can-branch="canBranch"
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
    </div>

    <!-- 指令条 -->
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
