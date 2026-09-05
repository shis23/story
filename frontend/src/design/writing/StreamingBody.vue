<script setup>
/**
 * StreamingBody — 写作进行中的流式区（重设计，纯展示）。
 *
 * 过程（导演/子Agent）默认收成一行状态条；成文正文是唯一主角。
 * contentComponent 由 adapter 注入（生产 = RichContent）。
 */
import { ref, computed, useId } from 'vue'
import { ChevronDown } from '@lucide/vue'

const props = defineProps({
  pipeline: { type: Object, required: true },
  roleLabel: { type: String, default: 'AI' },
  generationMode: { type: String, default: 'continuation' },
  contentComponent: { type: [Object, Function, String], default: null },
})

const showProcess = ref(false)
const detailId = useId()

const editorOutput = computed(() => props.pipeline.editor?.output || '')
const visibleStage = (stage) => stage && !['idle', 'pending'].includes(stage.status)
const editorLabel = computed(() =>
  (props.pipeline.editor?.role
    ? props.pipeline.editor.role === 'writer'
    : props.generationMode === 'continuation')
    ? '执笔者'
    : '编剧',
)
const steps = computed(() => {
  const list = []
  if (visibleStage(props.pipeline.director)) {
    list.push({ key: 'director', label: '导演', ...props.pipeline.director })
  }
  for (const s of props.pipeline.subagents || []) {
    if (visibleStage(s))
      list.push({ key: s.id, label: s.name || s.id, ...s })
  }
  if (visibleStage(props.pipeline.editor)) {
    list.push({ key: 'editor', label: editorLabel.value, ...props.pipeline.editor })
  }
  if (visibleStage(props.pipeline.summary)) {
    list.push({ key: 'summary', label: '剧情摘要', ...props.pipeline.summary })
  }
  if (visibleStage(props.pipeline.postprocess)) {
    list.push({ key: 'postprocess', label: '状态记账', ...props.pipeline.postprocess })
  }
  return list
})

const dotClass = (status) =>
  ({
    running: 'bg-running animate-pulse',
    done: 'bg-ok',
    error: 'bg-err',
    cancelled: 'bg-warn',
  })[status] || 'bg-wait'

function paragraphs(text) {
  return String(text || '').split(/\n{2,}/).filter(Boolean)
}
</script>

<template>
  <article class="py-5">
    <header class="flex items-center gap-2 mb-2">
      <span class="min-w-0 truncate text-xs font-medium text-accent">{{ roleLabel }}</span>
      <span class="flex items-center gap-1.5 text-[11px] text-running">
        <span class="w-1.5 h-1.5 rounded-full bg-running animate-pulse"></span>生成中
      </span>

      <button
        type="button"
        @click="showProcess = !showProcess"
        :aria-expanded="showProcess"
        :aria-controls="detailId"
        class="ml-auto flex items-center gap-1.5 min-h-7 px-2 rounded-md text-[11px] text-ink-faint hover:text-ink-soft hover:bg-surface-2 transition-colors"
      >
        <span class="flex items-center gap-1">
          <span v-for="s in steps" :key="s.key" class="w-1.5 h-1.5 rounded-full" :class="dotClass(s.status)"></span>
        </span>
        过程 {{ steps.filter((s) => s.status === 'done').length }}/{{ steps.length }}
        <ChevronDown
          :size="12" aria-hidden="true"
          class="transition-transform duration-150" :class="showProcess ? 'rotate-180' : ''"
        />
      </button>
    </header>

    <Transition name="sf-disclosure">
    <div v-if="showProcess" :id="detailId">
    <div class="sf-disclosure-body">
    <div class="mb-3 rounded-lg border border-line bg-surface/70 divide-y divide-line overflow-hidden">
      <div v-for="s in steps" :key="s.key" class="flex items-center gap-2 px-3 py-2 text-xs">
        <span class="w-1.5 h-1.5 rounded-full shrink-0" :class="dotClass(s.status)"></span>
        <span class="text-ink">{{ s.label }}</span>
        <span v-if="s.progress != null && s.status === 'running'" class="text-running tabular-nums">{{ s.progress }}%</span>
        <span class="text-ink-faint truncate">{{ s.detail || '' }}</span>
      </div>
    </div>
    </div>
    </div>
    </Transition>

    <div class="prose-fiction text-[15.5px] text-ink">
      <component
        :is="contentComponent"
        v-if="contentComponent"
        :content="editorOutput"
        :source-content="editorOutput"
      />
      <template v-else>
        <p v-for="(p, i) in paragraphs(editorOutput)" :key="i" class="mb-4 last:mb-0">{{ p }}</p>
      </template>
      <span class="inline-block w-[2px] h-[1.05em] bg-accent align-text-bottom animate-pulse"></span>
    </div>
  </article>
</template>
