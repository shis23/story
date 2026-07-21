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
})

const expanded = ref(false)
const openKey = ref(null)

const steps = computed(() => {
  const list = []
  const d = props.pipeline.director
  if (d && (d.output || d.detail)) list.push({ key: 'director', label: '构思规划', ...d })
  for (const s of props.pipeline.subagents || []) {
    if (s && s.status && s.status !== 'pending' && s.status !== 'idle')
      list.push({ key: s.id, label: s.name || '子 Agent', ...s })
  }
  const e = props.pipeline.editor
  if (e && (e.output || e.detail)) list.push({ key: 'editor', label: '内容生成', ...e })
  const q = props.pipeline.quality
  if (q) {
    list.push({
      key: 'quality',
      label: '质量校对',
      status: q.passed ? 'done' : 'error',
      detail: q.passed ? '通过' : `警告 ${q.warningCount || (q.warnings || []).length}`,
      output: (q.warnings || []).join('\n'),
    })
  }
  return list
})

const doneCount = computed(() => steps.value.filter((s) => s.status === 'done').length)
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
      <span class="text-ink-faint">{{ doneCount }}/{{ steps.length }} 步完成</span>
      <span class="ml-auto text-[11px] text-ink-faint">{{ expanded ? '收起' : '展开' }}</span>
    </button>

    <!-- 展开态：横向步骤条 -->
    <div v-if="expanded" class="border-t border-line px-4 sm:px-6 py-5">
      <ol class="flex items-start">
        <li
          v-for="(s, i) in steps"
          :key="s.key"
          class="flex-1 flex flex-col items-center text-center relative"
        >
          <!-- 连接线（左半） -->
          <span
            v-if="i > 0"
            class="absolute top-[13px] right-1/2 w-full h-px bg-line -z-0"
            aria-hidden="true"
          ></span>
          <button
            @click="s.output && toggleOutput(s.key)"
            class="relative z-10 w-7 h-7 rounded-full border-2 flex items-center justify-center bg-surface transition-colors"
            :class="[stepCircle(s.status), s.output ? 'cursor-pointer' : 'cursor-default']"
            :title="s.output ? (openKey === s.key ? '收起输出' : '查看输出') : s.label"
          >
            <!-- done:对勾 / running:脉冲点 / error:叹号 / 其他:圆点 -->
            <svg v-if="s.status === 'done'" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12.5l4.5 4.5L19 7"/></svg>
            <span v-else-if="s.status === 'running'" class="w-2 h-2 rounded-full bg-running animate-pulse"></span>
            <span v-else-if="s.status === 'error'" class="text-[10px] font-bold leading-none">!</span>
            <span v-else class="w-1.5 h-1.5 rounded-full bg-ink-faint"></span>
          </button>
          <div class="mt-2 text-xs font-medium text-ink leading-tight px-1">{{ s.label }}</div>
          <div class="mt-0.5 text-[10px] text-ink-faint leading-tight px-1 truncate max-w-full">{{ s.detail }}</div>
        </li>
      </ol>

      <!-- 步骤输出 -->
      <div
        v-if="openStep && openStep.output"
        class="mt-4 rounded-lg bg-surface-2/60 border border-line p-3 text-[11px] text-ink-soft font-mono whitespace-pre-wrap break-words max-h-44 overflow-y-auto leading-relaxed"
      >
        <div class="mb-1 text-[10px] font-medium text-ink-faint uppercase tracking-wider">{{ openStep.label }}</div>
        {{ openStep.output }}
      </div>
    </div>
  </section>
</template>
