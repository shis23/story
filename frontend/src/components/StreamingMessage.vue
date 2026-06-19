<script setup>
/**
 * StreamingMessage — 写作过程流式消息
 *
 * 在对话区最后一条消息位置，承载整个写作过程：
 *   - Director：折叠态（可展开看流式 delta）
 *   - 各子 Agent：折叠态（进度 + done 后可展开看 full_text）
 *   - Editor：底部逐字流式（衬线正文，pipeline.editor.output）
 *
 * 全程读 App.vue 的 pipeline 状态机，自身无独立状态。
 * draft_ready 后由 App.vue applyConversation 推入正式成文，本组件随 showPipeline=false 消失。
 */
import { ref, computed } from 'vue'
import { formatContent } from '../utils/formatContent.js'

const props = defineProps({
  pipeline: { type: Object, required: true },
  roleLabel: { type: String, default: 'AI' },
})

const expandedDirector = ref(false)
const expandedSubagents = ref(false)
const expandedSubagentIndices = ref(new Set())

function toggleSubagent(i) {
  if (expandedSubagentIndices.value.has(i)) expandedSubagentIndices.value.delete(i)
  else expandedSubagentIndices.value.add(i)
}

const statusDot = (status) => ({
  running: 'bg-running animate-pulse',
  done: 'bg-ok',
  error: 'bg-err',
  cancelled: 'bg-warn',
  pending: 'bg-wait',
  idle: 'bg-ink-faint',
}[status] || 'bg-ink-faint')

// Editor 逐字内容（衬线渲染）
const editorContent = computed(() => props.pipeline.editor?.output || '')
const editorRunning = computed(() => props.pipeline.editor?.status === 'running')
const editorDone = computed(() => props.pipeline.editor?.status === 'done')

const directorActive = computed(() => props.pipeline.director?.status === 'running')
const directorOutput = computed(() => props.pipeline.director?.output || '')

const subagents = computed(() => props.pipeline.subagents || [])
const activeSubagents = computed(() => subagents.value.filter(s => s && s.status && s.status !== 'pending' && s.status !== 'idle'))
</script>

<template>
  <div class="px-4 sm:px-6 py-4 rounded-xl">
    <!-- 角色标签 -->
    <div class="flex items-center gap-2 mb-3">
      <span class="text-xs font-medium text-accent">{{ roleLabel }}</span>
      <span class="text-[10px] text-ink-faint">· 生成中</span>
    </div>

    <!-- ── Director 折叠块 ── -->
    <div v-if="directorOutput || directorActive" class="mb-2 rounded-lg bg-surface/60 overflow-hidden">
      <button
        @click="expandedDirector = !expandedDirector"
        class="w-full flex items-center gap-2 px-3 min-h-[40px] text-xs text-ink-soft hover:bg-surface-2 transition-colors"
      >
        <span class="w-1.5 h-1.5 rounded-full" :class="statusDot(pipeline.director?.status)"></span>
        <span>🎬 导演规划</span>
        <span class="text-ink-faint">{{ pipeline.director?.detail || '' }}</span>
        <span class="ml-auto text-[10px]">{{ expandedDirector ? '▾' : '▸' }}</span>
      </button>
      <div v-if="expandedDirector && directorOutput" class="px-3 pb-2.5 text-[11px] text-ink-soft font-mono whitespace-pre-wrap break-words max-h-48 overflow-y-auto leading-relaxed">
        {{ directorOutput }}
      </div>
    </div>

    <!-- ── 子 Agent 折叠块 ── -->
    <div v-if="activeSubagents.length" class="mb-2 rounded-lg bg-surface/60 overflow-hidden">
      <button
        @click="expandedSubagents = !expandedSubagents"
        class="w-full flex items-center gap-2 px-3 min-h-[40px] text-xs text-ink-soft hover:bg-surface-2 transition-colors"
      >
        <span>🎭 子 Agent · {{ activeSubagents.filter(s => s.status === 'done').length }}/{{ activeSubagents.length }}</span>
        <span class="ml-auto flex items-center gap-1">
          <span v-for="(s, i) in activeSubagents" :key="i" class="w-1.5 h-1.5 rounded-full" :class="statusDot(s.status)"></span>
        </span>
        <span class="text-[10px]">{{ expandedSubagents ? '▾' : '▸' }}</span>
      </button>
      <div v-if="expandedSubagents" class="px-3 pb-2 space-y-1.5">
        <div v-for="(s, i) in activeSubagents" :key="i" class="rounded-md bg-surface-2/60 overflow-hidden">
          <button
            @click="toggleSubagent(i)"
            class="w-full flex items-center gap-2 px-2.5 min-h-[36px] text-xs hover:bg-surface-2 transition-colors"
          >
            <span class="w-1.5 h-1.5 rounded-full shrink-0" :class="statusDot(s.status)"></span>
            <span class="text-base shrink-0">{{ s.emoji || '🎭' }}</span>
            <span class="text-ink truncate flex-1 text-left">{{ s.name || s.id || '子 Agent' }}</span>
            <span v-if="s.status === 'running'" class="text-[10px] text-running">{{ s.progress || 0 }}%</span>
            <span class="text-[10px] text-ink-faint">{{ expandedSubagentIndices.has(i) ? '▾' : '▸' }}</span>
          </button>
          <!-- 进度条（运行中） -->
          <div v-if="s.status === 'running'" class="px-2.5 pb-1.5">
            <div class="h-1 bg-surface-2 rounded-full overflow-hidden">
              <div class="h-full bg-running rounded-full transition-all duration-500" :style="{ width: (s.progress || 0) + '%' }"></div>
            </div>
          </div>
          <!-- 展开看 full_text（done 后） -->
          <div v-if="expandedSubagentIndices.has(i) && s.output" class="px-2.5 pb-2 text-[11px] text-ink-soft font-mono whitespace-pre-wrap break-words max-h-40 overflow-y-auto leading-relaxed">
            {{ s.output }}
          </div>
        </div>
      </div>
    </div>

    <!-- ── Editor 逐字流式（衬线正文） ── -->
    <div v-if="editorContent || editorRunning" class="text-ink prose-fiction text-[15px] leading-loose">
      <span v-html="formatContent(editorContent)"></span>
      <!-- 打字光标 -->
      <span v-if="editorRunning" class="inline-block w-0.5 h-4 bg-accent align-middle animate-pulse ml-0.5"></span>
    </div>
    <div v-else-if="editorDone" class="text-[11px] text-ink-faint italic">成文完成，等待落盘…</div>
  </div>
</template>
