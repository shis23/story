<script setup>
import { ref } from 'vue'

defineProps({
  pipeline: { type: Object, required: true }, // { state, director, subagents, editor }
})

// 状态 → 颜色/图标
const statusMeta = {
  done: { dot: 'bg-ok', text: 'text-ok', label: '完成' },
  running: { dot: 'bg-running', text: 'text-running', label: '生成中' },
  waiting: { dot: 'bg-wait', text: 'text-wait', label: '等待' },
  idle: { dot: 'bg-ink-soft/40', text: 'text-ink-soft', label: '' },
}

function meta(s) { return statusMeta[s] || statusMeta.idle }

// 导演/编剧输出折叠状态
const showDirectorOutput = ref(true)
const showEditorOutput = ref(true)
</script>

<template>
  <div class="mx-4 my-3 bg-surface rounded-2xl border border-line overflow-hidden">
    <!-- 标题栏 -->
    <div class="px-4 py-2.5 bg-accent-soft/50 flex items-center gap-2">
      <div class="w-2 h-2 rounded-full" :class="pipeline.state === 'running' ? 'bg-running animate-pulse' : 'bg-ok'"></div>
      <span class="text-sm font-medium text-ink">写作流水线</span>
      <span class="text-xs text-ink-soft ml-auto">{{ pipeline.stateLabel }}</span>
    </div>

    <div class="px-4 py-3 space-y-3">
      <!-- 导演 -->
      <div>
        <div class="flex items-center gap-2.5">
          <span class="text-base">🎬</span>
          <div class="flex-1">
            <div class="text-sm font-medium">导演</div>
            <div class="text-xs text-ink-soft">{{ pipeline.director.detail }}</div>
          </div>
          <div class="flex items-center gap-1.5" :class="meta(pipeline.director.status).text">
            <div class="w-1.5 h-1.5 rounded-full" :class="[meta(pipeline.director.status).dot, pipeline.director.status==='running' ? 'animate-pulse' : '']"></div>
            <span class="text-xs">{{ meta(pipeline.director.status).label }}</span>
          </div>
        </div>
        <!-- 导演实时输出 -->
        <div v-if="pipeline.director.output" class="mt-1.5 ml-7">
          <button
            @click="showDirectorOutput = !showDirectorOutput"
            class="text-[10px] text-ink-soft hover:text-ink"
          >{{ showDirectorOutput ? '▾ 隐藏输出' : '▸ 查看输出' }}</button>
          <pre
            v-if="showDirectorOutput"
            class="mt-1 p-2 bg-bg rounded-lg text-[11px] text-ink-soft font-mono whitespace-pre-wrap break-words max-h-48 overflow-y-auto leading-relaxed"
          >{{ pipeline.director.output }}</pre>
        </div>
      </div>

      <!-- 子 Agent（并行） -->
      <div class="pl-6 border-l-2 border-dashed border-line space-y-2">
        <div class="text-[11px] text-ink-soft -ml-6 mb-1">子 Agent · 并行</div>
        <div v-for="sub in pipeline.subagents" :key="sub.id">
          <div class="flex items-center gap-2.5">
            <span class="text-sm">{{ sub.emoji }}</span>
            <div class="flex-1 min-w-0">
              <div class="text-sm truncate">{{ sub.name }}</div>
              <!-- 流式进度条 -->
              <div v-if="sub.status === 'running'" class="h-1 bg-bg rounded-full mt-1 overflow-hidden">
                <div class="h-full bg-running rounded-full transition-all duration-500" :style="{ width: sub.progress + '%' }"></div>
              </div>
            </div>
            <div class="flex items-center gap-1.5 shrink-0" :class="meta(sub.status).text">
              <div class="w-1.5 h-1.5 rounded-full" :class="[meta(sub.status).dot, sub.status==='running' ? 'animate-pulse' : '']"></div>
              <span class="text-xs">{{ meta(sub.status).label }}{{ sub.status === 'running' ? ' ' + sub.progress + '%' : '' }}</span>
            </div>
          </div>
          <!-- 子 Agent 产出文本（完成后展示，可折叠） -->
          <div v-if="sub.output" class="mt-1">
            <button
              @click="sub._expanded = !sub._expanded"
              class="text-[10px] text-ink-soft hover:text-ink"
            >{{ sub._expanded ? '▾ 隐藏表演' : '▸ 查看表演' }}</button>
            <pre
              v-if="sub._expanded"
              class="mt-1 p-2 bg-bg rounded-lg text-[11px] text-ink-soft font-mono whitespace-pre-wrap break-words max-h-48 overflow-y-auto leading-relaxed"
            >{{ sub.output }}</pre>
          </div>
        </div>
      </div>

      <!-- 编剧 -->
      <div>
        <div class="flex items-center gap-2.5">
          <span class="text-base">✍️</span>
          <div class="flex-1">
            <div class="text-sm font-medium">编剧</div>
            <div class="text-xs text-ink-soft">{{ pipeline.editor.detail }}</div>
          </div>
          <div class="flex items-center gap-1.5" :class="meta(pipeline.editor.status).text">
            <div class="w-1.5 h-1.5 rounded-full" :class="[meta(pipeline.editor.status).dot, pipeline.editor.status==='running' ? 'animate-pulse' : '']"></div>
            <span class="text-xs">{{ meta(pipeline.editor.status).label }}</span>
          </div>
        </div>
        <!-- 编剧实时输出 -->
        <div v-if="pipeline.editor.output" class="mt-1.5 ml-7">
          <button
            @click="showEditorOutput = !showEditorOutput"
            class="text-[10px] text-ink-soft hover:text-ink"
          >{{ showEditorOutput ? '▾ 隐藏成文' : '▸ 查看成文' }}</button>
          <pre
            v-if="showEditorOutput"
            class="mt-1 p-2 bg-bg rounded-lg text-[11px] text-ink-soft font-mono whitespace-pre-wrap break-words max-h-64 overflow-y-auto leading-relaxed"
          >{{ pipeline.editor.output }}</pre>
        </div>
      </div>
    </div>
  </div>
</template>
