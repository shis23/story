<script setup>
/**
 * ProcessReview — 写作完成后的过程回顾（v2, 修复 P2-1）
 *
 * 背景：StreamingMessage 挂载依赖 writing.showPipeline，而 useWriting.js 在完成
 * 回调里把 showPipeline 置 false，导致 StreamingMessage 整体卸载，导演规划/子 Agent
 * 折叠块随之消失，用户完成后看不到导演输出。
 *
 * 本组件独立于 showPipeline：只要上一轮写作完成（pipeline.state==='done' 且
 * 非正在写作）且有导演/子 Agent 输出，就以折叠块形式展示过程回顾，挂载条件只看
 * 数据是否存在，不看 showPipeline。
 *
 * 数据源：writingStore.pipeline（完成后不清空，stateLabel/状态/输出均保留）。
 */
import { ref, computed } from 'vue'
import { useWritingStore } from '../../stores/index.js'

const writing = useWritingStore()

// 折叠态（独立于 StreamingMessage，互不影响）
const expanded = ref(false)

const directorOutput = computed(() => writing.pipeline.director?.output || '')
const directorStatus = computed(() => writing.pipeline.director?.status)
const editorOutput = computed(() => writing.pipeline.editor?.output || '')
const editorStatus = computed(() => writing.pipeline.editor?.status)
const subagents = computed(() =>
  (writing.pipeline.subagents || []).filter(
    (s) => s && s.status && s.status !== 'pending' && s.status !== 'idle',
  ),
)

// 有过程数据可回顾（已完成态，且至少导演/编剧/子 Agent 有输出）
const hasReview = computed(
  () =>
    !writing.isWriting &&
    writing.pipeline.state === 'done' &&
    (directorOutput.value || editorOutput.value || subagents.value.length),
)

const directorDetail = computed(() => writing.pipeline.director?.detail || '')
const editorDetail = computed(() => writing.pipeline.editor?.detail || '')
const quality = computed(() => writing.pipeline.quality)
const qualityWarnings = computed(() => quality.value?.warnings || [])
</script>

<template>
  <div
    v-if="hasReview"
    class="mx-auto max-w-2xl px-4 sm:px-6"
  >
    <div class="mb-2 rounded-lg border border-line/60 overflow-hidden">
      <button
        @click="expanded = !expanded"
        class="w-full flex items-center gap-2 px-3 min-h-[36px] text-xs text-ink-soft hover:bg-surface-2 transition-colors"
      >
        <span class="text-ink-faint">📋</span>
        <span>上一轮过程回顾</span>
        <span class="text-ink-faint">{{ directorDetail }}</span>
        <span class="ml-auto text-[10px] text-ink-faint">{{ expanded ? '收起' : '展开' }}</span>
      </button>

      <div v-if="expanded" class="px-3 pb-3 space-y-2 bg-surface/40">
        <!-- 导演规划 -->
        <div v-if="directorOutput">
          <div class="text-[10px] font-medium text-ink-faint uppercase tracking-wide pt-2 pb-1">🎬 导演规划</div>
          <div class="text-[11px] text-ink-soft font-mono whitespace-pre-wrap break-words max-h-48 overflow-y-auto leading-relaxed">
            {{ directorOutput }}
          </div>
        </div>

        <!-- 子 Agent 产出 -->
        <div v-if="subagents.length">
          <div class="text-[10px] font-medium text-ink-faint uppercase tracking-wide pt-2 pb-1">
            🎭 子 Agent · {{ subagents.filter(s => s.status === 'done').length }}/{{ subagents.length }}
          </div>
          <div
            v-for="(s, i) in subagents"
            :key="s.id || i"
            class="rounded-md bg-surface-2/50 p-2"
          >
            <div class="flex items-center gap-2 text-xs">
              <span>{{ s.emoji || '🎭' }}</span>
              <span class="text-ink truncate">{{ s.name || s.id || '子 Agent' }}</span>
            </div>
            <div v-if="s.output" class="mt-1 text-[11px] text-ink-soft font-mono whitespace-pre-wrap break-words max-h-40 overflow-y-auto leading-relaxed">
              {{ s.output }}
            </div>
          </div>
        </div>

        <!-- 编剧成文 -->
        <div v-if="editorOutput">
          <div class="text-[10px] font-medium text-ink-faint uppercase tracking-wide pt-2 pb-1">
            ✍️ 编剧成文
          </div>
          <div class="text-[11px] text-ink-soft font-mono whitespace-pre-wrap break-words max-h-48 overflow-y-auto leading-relaxed">
            {{ editorOutput }}
          </div>
        </div>

        <!-- B3 质量门禁（warn-only） -->
        <div v-if="quality">
          <div class="text-[10px] font-medium text-ink-faint uppercase tracking-wide pt-2 pb-1">
            🧪 质量检查
            <span
              class="ml-1 normal-case tracking-normal"
              :class="quality.passed ? 'text-ok' : 'text-warn'"
            >
              {{ quality.passed ? '通过' : `警告 ${quality.warningCount || qualityWarnings.length}` }}
            </span>
          </div>
          <ul
            v-if="qualityWarnings.length"
            class="text-[11px] text-ink-soft space-y-1 list-disc pl-4"
          >
            <li v-for="(msg, i) in qualityWarnings" :key="i">{{ msg }}</li>
          </ul>
          <div v-else class="text-[11px] text-ink-faint">未发现确定性质量问题</div>
        </div>
      </div>
    </div>
  </div>
</template>
