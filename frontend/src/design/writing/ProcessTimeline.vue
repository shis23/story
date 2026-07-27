<script setup>
/**
 * ProcessTimeline — 创作过程回顾（重设计：横向步骤条，对齐选定图③）。
 *
 * 折叠态：一行摘要（步骤数 + 质量状态）。
 * 展开态：横向步骤条（图标圈 + 连接线 + 标签 + 状态），点步骤看 mono 输出。
 */
import { ref, computed } from 'vue'

const props = defineProps({
  pipeline: { type: Object, required: true },
  generationMode: { type: String, default: 'continuation' },
})

const expanded = ref(false)
const openKey = ref(null)

const visibleStage = (stage) => stage && !['idle', 'pending'].includes(stage.status)

const steps = computed(() => {
  const list = []
  const d = props.pipeline.director
  if (visibleStage(d)) list.push({ key: 'director', label: '场景规划', kind: 'model', ...d })
  for (const s of props.pipeline.subagents || []) {
    if (visibleStage(s)) {
      list.push({ key: s.id, label: s.name || '角色表演', kind: 'model', ...s })
    }
  }
  const e = props.pipeline.editor
  if (visibleStage(e)) {
    const writer = e.role ? e.role === 'writer' : props.generationMode === 'continuation'
    list.push({
      key: 'editor',
      label: writer ? '正文续写' : '成文合编',
      kind: 'model',
      ...e,
    })
  }
  const q = props.pipeline.quality
  if (q) {
    list.push({
      key: 'quality',
      label: '规则检查',
      kind: 'rule',
      status: q.passed ? 'done' : 'error',
      detail: q.passed ? '通过' : `警告 ${q.warningCount || (q.warnings || []).length}`,
      output: (q.warnings || []).join('\n') || '未发现确定性质量问题',
    })
  }
  const summary = props.pipeline.summary
  if (visibleStage(summary)) {
    list.push({ key: 'summary', label: '剧情摘要', kind: 'model', ...summary })
  }
  const postprocess = props.pipeline.postprocess
  if (visibleStage(postprocess)) {
    list.push({ key: 'postprocess', label: '状态记账', kind: 'model', ...postprocess })
  }
  return list
})

const completedCount = computed(() =>
  steps.value.filter((s) => ['done', 'error', 'cancelled'].includes(s.status)).length,
)
const modelStageCount = computed(() => steps.value.filter((s) => s.kind === 'model').length)
const ruleStageCount = computed(() => steps.value.filter((s) => s.kind === 'rule').length)
const qualityOk = computed(() => props.pipeline.quality?.passed !== false)
const openStep = computed(() => steps.value.find((s) => s.key === openKey.value))

function stepCircle(status) {
  return (
    {
      done: 'border-ok text-ok bg-ok/10',
      running: 'border-running text-running bg-running/10',
      error: 'border-warn text-warn bg-warn/10',
      cancelled: 'border-warn text-warn bg-warn/10',
    }[status] || 'border-line text-ink-faint'
  )
}

function toggleOutput(key) {
  openKey.value = openKey.value === key ? null : key
}

function stepInspectable(step) {
  return !!(step.output || step.detail)
}
</script>

<template>
  <section class="rounded-xl border border-line bg-surface shadow-card overflow-hidden">
    <!-- 折叠态：一行摘要 -->
    <button
      @click="expanded = !expanded"
      class="w-full flex items-center gap-2 px-4 min-h-[42px] text-xs text-ink-soft hover:bg-surface-2/60 transition-colors"
    >
      <span class="w-1.5 h-1.5 rounded-full shrink-0" :class="qualityOk ? 'bg-ok' : 'bg-warn'"></span>
      <span class="font-medium text-ink">创作过程回顾</span>
      <span class="text-ink-faint">{{ completedCount }}/{{ steps.length }} 步完成</span>
      <span class="hidden sm:inline text-[10px] text-ink-faint">
        {{ modelStageCount }} 个模型阶段<span v-if="ruleStageCount"> · {{ ruleStageCount }} 次规则检查</span>
      </span>
      <span class="ml-auto text-[11px] text-ink-faint">{{ expanded ? '收起' : '展开' }}</span>
    </button>

    <!-- 展开态：阶段账本。后处理允许并行，因此不再用误导性的单向连接线。 -->
    <div v-if="expanded" class="border-t border-line px-4 sm:px-6 py-5">
      <ol class="grid grid-cols-2 gap-2.5 sm:grid-cols-4">
        <li
          v-for="s in steps"
          :key="s.key"
          class="min-w-0"
        >
          <button
            type="button"
            class="h-full w-full rounded-xl border bg-bg/70 p-3 text-left transition-all"
            :class="[
              openKey === s.key ? 'border-accent-border shadow-card' : 'border-line',
              stepInspectable(s) ? 'hover:border-accent-border hover:bg-surface-2' : 'cursor-default',
            ]"
            :disabled="!stepInspectable(s)"
            :title="stepInspectable(s) ? (openKey === s.key ? '收起详情' : '查看详情') : s.label"
            @click="stepInspectable(s) && toggleOutput(s.key)"
          >
            <span class="flex items-center justify-between gap-2">
              <span class="flex h-6 w-6 items-center justify-center rounded-full border text-[10px]" :class="stepCircle(s.status)">
                <svg v-if="s.status === 'done'" width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12.5l4.5 4.5L19 7"/></svg>
                <span v-else-if="s.status === 'running'" class="h-1.5 w-1.5 rounded-full bg-running animate-pulse"></span>
                <span v-else-if="s.status === 'error'" class="font-bold">!</span>
                <span v-else class="h-1.5 w-1.5 rounded-full bg-ink-faint"></span>
              </span>
              <span class="rounded-full border border-line px-1.5 py-0.5 text-[9px] text-ink-faint">
                {{ s.kind === 'rule' ? '规则' : '模型' }}
              </span>
            </span>
            <span class="mt-2 block truncate text-xs font-medium text-ink">{{ s.label }}</span>
            <span class="mt-1 block truncate text-[10px] text-ink-faint">{{ s.detail || '等待结果' }}</span>
          </button>
        </li>
      </ol>

      <!-- 步骤输出 -->
      <div
        v-if="openStep && (openStep.output || openStep.detail)"
        class="mt-4 max-h-56 overflow-y-auto rounded-xl border border-line bg-bg/70 p-4 text-ink-soft whitespace-pre-wrap break-words"
        :class="openStep.key === 'editor' ? 'font-serif text-sm leading-7' : 'font-mono text-[11px] leading-relaxed'"
      >
        <div class="mb-1 text-[10px] font-medium text-ink-faint uppercase tracking-wider">{{ openStep.label }}</div>
        {{ openStep.output || openStep.detail }}
      </div>
    </div>
  </section>
</template>
