<script setup>
/**
 * StoryPage — 稿纸页（重设计核心：写作区=桌面上的一张纸）。
 *
 * 结构（对齐 selected 图②③）：
 *   - 页头：章节题 + 元信息（字数/用时）+ 状态徽章
 *   - 页体：消息以「文稿段落」呈现
 *   - 页脚（流式中）：AI 正在生成… + 停止生成
 *
 * 纯展示；contentComponent / qualityAcceptHint / subagentRoles 由上层透传。
 */
import { computed } from 'vue'
import MessageItem from './MessageItem.vue'
import StreamingBody from './StreamingBody.vue'

const props = defineProps({
  title: { type: String, default: '' },
  durationText: { type: String, default: '' },
  messages: { type: Array, default: () => [] },
  isWriting: { type: Boolean, default: false },
  pipeline: { type: Object, default: () => ({}) },
  showPipeline: { type: Boolean, default: false },
  streamingRoleLabel: { type: String, default: 'AI' },
  canBranch: { type: Boolean, default: false },
  qualityAcceptHint: { type: String, default: null },
  contentComponent: { type: [Object, Function, String], default: null },
  /** messageId -> [{id,label}] */
  subagentRolesByMessage: { type: Object, default: () => ({}) },
})

const emit = defineEmits([
  'cancel',
  'reroll',
  'reroll-user',
  'switch-variant',
  'edit-variant',
  'accept-variant',
  'delete-variant',
  'add-variant',
  'branch',
])

const wordCount = computed(() => {
  const all = props.messages
    .map((m) => {
      const v = m.variants?.[m.active_variant]
      return v?.display_content ?? v?.content ?? ''
    })
    .join('')
  return all.replace(/\s/g, '').length
})

const status = computed(() => {
  if (props.isWriting) return { text: '生成中', cls: 'text-running border-running/30 bg-running/10', dot: 'bg-running animate-pulse' }
  if (props.messages.length) return { text: '已完成', cls: 'text-ok border-ok/30 bg-ok/10', dot: 'bg-ok' }
  return null
})

const messageEvents = {
  'reroll': (p) => emit('reroll', p),
  'reroll-user': (p) => emit('reroll-user', p),
  'switch-variant': (p) => emit('switch-variant', p),
  'edit-variant': (p) => emit('edit-variant', p),
  'accept-variant': (p) => emit('accept-variant', p),
  'delete-variant': (p) => emit('delete-variant', p),
  'add-variant': (p) => emit('add-variant', p),
  'branch': (p) => emit('branch', p),
}

function rolesFor(message) {
  return props.subagentRolesByMessage[message.id] || []
}
</script>

<template>
  <div class="px-4 sm:px-8 py-6 sm:py-8">
    <div class="mx-auto w-full max-w-[760px] rounded-xl border border-line bg-surface shadow-rise">
      <header class="px-6 sm:px-12 pt-8 sm:pt-10 pb-2">
        <div class="flex items-start gap-3">
          <div class="flex-1 min-w-0">
            <h1 class="font-semibold text-[22px] sm:text-[24px] leading-snug text-ink">
              {{ title || '未命名文稿' }}
            </h1>
            <p class="mt-1.5 text-xs text-ink-faint">
              <span>字数 {{ wordCount.toLocaleString() }}</span>
              <template v-if="durationText"><span class="mx-1.5">·</span><span>{{ durationText }}</span></template>
            </p>
          </div>
          <span
            v-if="status"
            class="shrink-0 mt-1 flex items-center gap-1.5 px-2 py-0.5 rounded-full border text-[11px]"
            :class="status.cls"
          >
            <span class="w-1.5 h-1.5 rounded-full" :class="status.dot"></span>{{ status.text }}
          </span>
        </div>
      </header>

      <div class="px-6 sm:px-12 pb-4">
        <MessageItem
          v-for="m in messages"
          :key="m.id"
          :message="m"
          :busy="isWriting"
          :can-branch="canBranch"
          :quality-accept-hint="qualityAcceptHint"
          :content-component="contentComponent"
          :subagent-roles="rolesFor(m)"
          v-on="messageEvents"
        />

        <StreamingBody
          v-if="showPipeline && isWriting"
          :pipeline="pipeline"
          :role-label="streamingRoleLabel"
          :content-component="contentComponent"
        />
      </div>

      <footer
        v-if="isWriting"
        class="flex items-center gap-2 mx-6 sm:mx-12 mb-6 sm:mb-8 mt-2 rounded-lg border border-line bg-surface-2/50 px-4 py-2.5"
      >
        <span class="flex items-center gap-2 text-xs text-ink-soft">
          <span class="w-1.5 h-1.5 rounded-full bg-running animate-pulse"></span>
          AI 正在生成
          <span class="flex gap-0.5" aria-hidden="true">
            <span class="w-1 h-1 rounded-full bg-ink-faint animate-pulse" style="animation-delay:0ms"></span>
            <span class="w-1 h-1 rounded-full bg-ink-faint animate-pulse" style="animation-delay:200ms"></span>
            <span class="w-1 h-1 rounded-full bg-ink-faint animate-pulse" style="animation-delay:400ms"></span>
          </span>
        </span>
        <button
          type="button"
          @click="emit('cancel')"
          class="ml-auto min-h-8 px-3 rounded-md text-xs text-ink-soft border border-line bg-surface hover:border-err/40 hover:text-err transition-colors"
        >停止生成</button>
      </footer>

      <div v-else class="h-6 sm:h-8"></div>
    </div>
  </div>
</template>
