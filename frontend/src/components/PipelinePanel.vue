<script setup>
import { ref, computed } from 'vue'

defineProps({
  pipeline: { type: Object, required: true },
})

// 状态 → 颜色/图标
const statusMeta = {
  done: { dot: 'bg-ok', text: 'text-ok', label: '完成' },
  running: { dot: 'bg-running', text: 'text-running', label: '运行中' },
  waiting: { dot: 'bg-wait', text: 'text-wait', label: '等待' },
  pending: { dot: 'bg-wait', text: 'text-wait', label: '等待' },
  cancelled: { dot: 'bg-warn', text: 'text-warn', label: '已取消' },
  error: { dot: 'bg-err', text: 'text-err', label: '失败' },
  idle: { dot: 'bg-ink-soft/40', text: 'text-ink-soft', label: '' },
}

function meta(s) { return statusMeta[s] || statusMeta.idle }

// ─── 折叠状态（默认折叠，移动端避免长文本撑开） ──────────────────────────
const expandedDirector = ref(false)
const expandedSubagents = ref(true)
const expandedEditor = ref(false)
const expandedPostprocess = ref(false)

// 子 Agent 展开状态（用 Set 追踪展开的 index）
const expandedSubagentIndices = ref(new Set())
function toggleSubagentExpand(index) {
  if (expandedSubagentIndices.value.has(index)) {
    expandedSubagentIndices.value.delete(index)
  } else {
    expandedSubagentIndices.value.add(index)
  }
}
function isSubagentExpanded(index) {
  return expandedSubagentIndices.value.has(index)
}
</script>

<template>
  <div class="mx-4 my-3 bg-surface rounded-2xl border border-line overflow-hidden">
    <!-- 标题栏 -->
    <div class="px-4 py-2.5 bg-accent-soft/50 flex items-center gap-2">
      <div class="w-2 h-2 rounded-full" :class="pipeline.state === 'running' ? 'bg-running animate-pulse' : 'bg-ok'"></div>
      <span class="text-sm font-medium text-ink">写作流水线</span>
      <span class="text-xs text-ink-soft ml-auto">{{ pipeline.stateLabel }}</span>
    </div>

    <div class="px-4 py-3 space-y-1">
      <!-- ═══ 导演段 ═══ -->
      <div class="rounded-lg overflow-hidden">
        <button
          class="w-full flex items-center gap-2.5 px-2 py-2 hover:bg-line/30 rounded-lg transition-colors"
          @click="expandedDirector = !expandedDirector"
        >
          <span class="text-[10px] text-ink-soft/60 w-3 text-center">{{ expandedDirector ? '▾' : '▸' }}</span>
          <span class="text-base">🎬</span>
          <div class="flex-1 text-left min-w-0">
            <div class="text-sm font-medium">导演</div>
            <div class="text-xs text-ink-soft truncate">{{ pipeline.director.detail }}</div>
          </div>
          <div class="flex items-center gap-1.5 shrink-0" :class="meta(pipeline.director.status).text">
            <div class="w-1.5 h-1.5 rounded-full" :class="[meta(pipeline.director.status).dot, pipeline.director.status === 'running' ? 'animate-pulse' : '']"></div>
            <span class="text-xs">{{ meta(pipeline.director.status).label }}</span>
          </div>
        </button>
        <div v-if="expandedDirector && pipeline.director.output" class="px-2 pb-2 ml-8">
          <pre class="p-2 bg-bg rounded-lg text-[11px] text-ink-soft font-mono whitespace-pre-wrap break-words max-h-48 overflow-y-auto leading-relaxed border border-line/50">{{ pipeline.director.output }}</pre>
        </div>
      </div>

      <!-- 连接线 -->
      <div class="ml-4 h-3 border-l-2 border-dashed border-line"></div>

      <!-- ═══ 子 Agent 段 ═══ -->
      <div class="rounded-lg overflow-hidden">
        <button
          class="w-full flex items-center gap-2.5 px-2 py-2 hover:bg-line/30 rounded-lg transition-colors"
          @click="expandedSubagents = !expandedSubagents"
        >
          <span class="text-[10px] text-ink-soft/60 w-3 text-center">{{ expandedSubagents ? '▾' : '▸' }}</span>
          <span class="text-base">🎭</span>
          <div class="flex-1 text-left min-w-0">
            <div class="text-sm font-medium">子 Agent · 并行</div>
            <div class="text-xs text-ink-soft">
              <template v-if="pipeline.subagents.length === 0">等待分配</template>
              <template v-else>{{ pipeline.subagents.filter(s => s.status === 'done').length }} / {{ pipeline.subagents.length }} 完成</template>
            </div>
          </div>
          <div class="flex items-center gap-1.5 shrink-0">
            <template v-for="(sub, i) in pipeline.subagents" :key="i">
              <div class="w-1.5 h-1.5 rounded-full" :class="meta(sub.status).dot"></div>
            </template>
          </div>
        </button>

        <div v-if="expandedSubagents && pipeline.subagents.length > 0" class="px-2 pb-2 ml-8 space-y-1">
          <div
            v-for="(sub, i) in pipeline.subagents"
            :key="i"
            class="rounded-lg border border-line/50 overflow-hidden"
          >
            <!-- 子 Agent 标题行 -->
            <button
              class="w-full flex items-center gap-2 px-2 py-1.5 hover:bg-line/20 transition-colors"
              @click="toggleSubagentExpand(i)"
            >
              <span class="text-[10px] text-ink-soft/60 w-3 text-center">{{ isSubagentExpanded(i) ? '▾' : '▸' }}</span>
              <span class="text-sm">{{ sub.emoji }}</span>
              <div class="flex-1 text-left min-w-0">
                <div class="text-xs font-medium truncate">{{ sub.name || sub.id || '角色 ' + (i + 1) }}</div>
                <!-- 运行中时显示进度条 -->
                <div v-if="sub.status === 'running'" class="h-1 bg-bg rounded-full mt-1 overflow-hidden">
                  <div class="h-full bg-running rounded-full transition-all duration-500" :style="{ width: sub.progress + '%' }"></div>
                </div>
              </div>
              <div class="flex items-center gap-1.5 shrink-0" :class="meta(sub.status).text">
                <div class="w-1.5 h-1.5 rounded-full" :class="[meta(sub.status).dot, sub.status === 'running' ? 'animate-pulse' : '']"></div>
                <span class="text-[11px]">{{ meta(sub.status).label }}{{ sub.status === 'running' ? ' ' + sub.progress + '%' : '' }}</span>
              </div>
            </button>
            <!-- 子 Agent 展开详情 -->
            <div v-if="isSubagentExpanded(i)" class="px-2 pb-2 space-y-1">
              <!-- 实例 ID（当 name 与 id 不同时显示） -->
              <div v-if="sub.id && sub.name && sub.id !== sub.name" class="text-[10px] text-ink-soft/50 px-2">
                ID: {{ sub.id }}
              </div>
              <!-- 输出 -->
              <pre v-if="sub.output" class="p-2 bg-bg rounded-lg text-[11px] text-ink-soft font-mono whitespace-pre-wrap break-words max-h-48 overflow-y-auto leading-relaxed border border-line/50">{{ sub.output }}</pre>
              <div v-else-if="sub.status !== 'running'" class="text-[11px] text-ink-soft/50 italic px-2">暂无输出</div>
            </div>
          </div>
        </div>
      </div>

      <!-- 连接线 -->
      <div class="ml-4 h-3 border-l-2 border-dashed border-line"></div>

      <!-- ═══ 编剧段 ═══ -->
      <div class="rounded-lg overflow-hidden">
        <button
          class="w-full flex items-center gap-2.5 px-2 py-2 hover:bg-line/30 rounded-lg transition-colors"
          @click="expandedEditor = !expandedEditor"
        >
          <span class="text-[10px] text-ink-soft/60 w-3 text-center">{{ expandedEditor ? '▾' : '▸' }}</span>
          <span class="text-base">✍️</span>
          <div class="flex-1 text-left min-w-0">
            <div class="text-sm font-medium">编剧</div>
            <div class="text-xs text-ink-soft truncate">{{ pipeline.editor.detail }}</div>
          </div>
          <div class="flex items-center gap-1.5 shrink-0" :class="meta(pipeline.editor.status).text">
            <div class="w-1.5 h-1.5 rounded-full" :class="[meta(pipeline.editor.status).dot, pipeline.editor.status === 'running' ? 'animate-pulse' : '']"></div>
            <span class="text-xs">{{ meta(pipeline.editor.status).label }}</span>
          </div>
        </button>
        <div v-if="expandedEditor && pipeline.editor.output" class="px-2 pb-2 ml-8">
          <pre class="p-2 bg-bg rounded-lg text-[11px] text-ink-soft font-mono whitespace-pre-wrap break-words max-h-64 overflow-y-auto leading-relaxed border border-line/50">{{ pipeline.editor.output }}</pre>
        </div>
      </div>

      <!-- ═══ 后处理段（仅在有数据时显示） ═══ -->
      <template v-if="pipeline.postprocess && pipeline.postprocess.status !== 'idle'">
        <!-- 连接线 -->
        <div class="ml-4 h-3 border-l-2 border-dashed border-line"></div>

        <div class="rounded-lg overflow-hidden">
          <button
            class="w-full flex items-center gap-2.5 px-2 py-2 hover:bg-line/30 rounded-lg transition-colors"
            @click="expandedPostprocess = !expandedPostprocess"
          >
            <span class="text-[10px] text-ink-soft/60 w-3 text-center">{{ expandedPostprocess ? '▾' : '▸' }}</span>
            <span class="text-base">📦</span>
            <div class="flex-1 text-left min-w-0">
              <div class="text-sm font-medium">后处理</div>
              <div class="text-xs text-ink-soft truncate">{{ pipeline.postprocess.detail }}</div>
            </div>
            <div class="flex items-center gap-1.5 shrink-0" :class="meta(pipeline.postprocess.status).text">
              <div class="w-1.5 h-1.5 rounded-full" :class="[meta(pipeline.postprocess.status).dot, pipeline.postprocess.status === 'running' ? 'animate-pulse' : '']"></div>
              <span class="text-xs">{{ meta(pipeline.postprocess.status).label }}</span>
            </div>
          </button>
          <div v-if="expandedPostprocess" class="px-2 pb-2 ml-8">
            <!-- 成功：显示三项计数 -->
            <div v-if="pipeline.postprocess.status === 'done' && !pipeline.postprocess.reason" class="flex gap-3 text-xs">
              <span class="px-2 py-0.5 bg-accent-soft/50 rounded text-accent">知识 {{ pipeline.postprocess.knowledge }}</span>
              <span class="px-2 py-0.5 bg-accent-soft/50 rounded text-accent">变量 {{ pipeline.postprocess.variable }}</span>
              <span class="px-2 py-0.5 bg-accent-soft/50 rounded text-accent">任务 {{ pipeline.postprocess.task }}</span>
            </div>
            <!-- 跳过/失败：显示 reason -->
            <div v-else-if="pipeline.postprocess.reason" class="text-[11px] text-ink-soft bg-bg rounded-lg p-2 border border-line/50">
              <span class="text-ink-soft/60">原因：</span>{{ pipeline.postprocess.reason }}
            </div>
            <!-- 运行中 -->
            <div v-else-if="pipeline.postprocess.status === 'running'" class="text-[11px] text-ink-soft/50 italic">
              处理中…
            </div>
          </div>
        </div>
      </template>
    </div>

    <!-- 底部 extra slot（停止按钮等） -->
    <div v-if="$slots.extra" class="px-4 pb-3">
      <slot name="extra" />
    </div>
  </div>
</template>
