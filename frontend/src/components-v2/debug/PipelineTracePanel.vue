<script setup>
/**
 * PipelineTracePanel — 流水线 trace 面板。
 *
 * 读 pluginStore.pluginPipelineEvents(最近 100 条,store 已裁剪)。
 * 按 event_type 归类到 Director / Subagents / Editor / Postprocess 四个阶段,
 * 每阶段显示状态 / 详情 / 输出。复用 utils/pipelineTrace.js(subagentRolesFromProvenance)。
 *
 * 事件结构(见 usePluginBridge.pushPluginEventRecord):
 *   流水线事件: { id, event: { event_type, data } }
 *   普通插件事件: { id, event: 'EVENT_NAME', data: {...} }
 * 此面板只关注流水线事件(event 是对象且带 event_type)。
 */
import { computed, ref } from 'vue'
import { usePluginStore } from '../../stores/index.js'
import { subagentRolesFromProvenance } from '../../utils/pipelineTrace.js'
import Tabs from '../ui/Tabs.vue'
import EmptyState from '../ui/EmptyState.vue'
import Badge from '../ui/Badge.vue'
import CodeBlock from '../ui/CodeBlock.vue'

const plugin = usePluginStore()

const activeStage = ref('director')
const tabs = [
  { key: 'director', label: '导演' },
  { key: 'subagents', label: '子 Agent' },
  { key: 'editor', label: '编剧' },
  { key: 'postprocess', label: '后处理' },
]

// ─── 提取流水线事件(只取 event 是对象且带 event_type 的记录) ───
const pipelineEvents = computed(() =>
  plugin.pluginPipelineEvents.filter(
    (r) => r && r.event && typeof r.event === 'object' && r.event.event_type,
  ),
)

// ─── 按阶段归类 ───
const directorEvents = computed(() =>
  pipelineEvents.value.filter((r) => r.event.event_type.startsWith('director_')),
)
const subagentEvents = computed(() =>
  pipelineEvents.value.filter((r) => r.event.event_type.startsWith('subagent_')),
)
const editorEvents = computed(() =>
  pipelineEvents.value.filter(
    (r) =>
      r.event.event_type.startsWith('editor_') ||
      // F-44：续写档的执笔者事件（usePipeline.js:84-101 广播 writer_started/writer_progress/
      // writer_done）此前不属于任何 tab，面板里"谁写的正文"完全不可见
      r.event.event_type.startsWith('writer_') ||
      r.event.event_type === 'draft_ready' ||
      r.event.event_type === 'quality_checked',
  ),
)
const postprocessEvents = computed(() =>
  pipelineEvents.value.filter((r) => r.event.event_type.startsWith('postprocess_') || r.event.event_type === 'summary_done'),
)

// ─── 状态推断:取该阶段最后一条"带状态语义"的事件推导状态 ───
// F-47：此前只看整段列表的最后一条 —— 末事件不是 *_started/_progress/_done/_failed
// 时无条件回落 'running'，阶段会永久显示"运行中"（缓冲区不清空、无 pipeline state 参照）。
// 现在从尾部往前找第一条能判定状态的事件；一条都没有才是 idle。
// 残余局限（已在修复记录标注）：不做 writingStore.pipeline.state 交叉校验，
// 进程被强杀（无 failed 事件）时仍可能停在 running/progress。
function statusFromEventType(type) {
  if (!type) return null
  if (type.endsWith('_started') || type.endsWith('_progress')) return 'running'
  if (type.endsWith('_done') || type === 'draft_ready' || type === 'quality_checked') return 'done'
  if (type.endsWith('_failed') || type.endsWith('_cancelled') || type.endsWith('_error')) return 'error'
  if (type.endsWith('_skipped')) return 'done'
  return null
}

function lastStatus(events) {
  for (let i = events.length - 1; i >= 0; i--) {
    const status = statusFromEventType(events[i]?.event?.event_type)
    if (status) return status
  }
  return 'idle'
}

function statusVariant(status) {
  switch (status) {
    case 'running': return 'accent'
    case 'done': return 'ok'
    case 'error': return 'err'
    default: return 'neutral'
  }
}

function statusLabel(status) {
  switch (status) {
    case 'running': return '运行中'
    case 'done': return '已完成'
    case 'error': return '错误'
    default: return '空闲'
  }
}

// ─── 导演输出(累积 director_progress 的 delta) ───
const directorOutput = computed(() => {
  const parts = []
  for (const r of directorEvents.value) {
    if (r.event.event_type === 'director_progress' && r.event.data?.delta) {
      parts.push(r.event.data.delta)
    }
  }
  return parts.join('')
})

const directorDetail = computed(() => {
  const done = directorEvents.value.find((r) => r.event.event_type === 'director_done')
  if (done) return `规划完成 · 分配 ${done.event.data?.subagent_count ?? 0} 个角色`
  if (directorEvents.value.length > 0) return '导演思考中'
  return ''
})

// ─── 子 Agent:从 director_done 的 provenance 提取角色,无则用事件构造 ───
const subagentRoles = computed(() => {
  const done = directorEvents.value.find((r) => r.event.event_type === 'director_done')
  const roles = subagentRolesFromProvenance(done?.event?.data?.provenance)
  if (roles.length > 0) return roles
  // fallback:从 subagent_started 事件构造
  return subagentEvents.value
    .filter((r) => r.event.event_type === 'subagent_started' && r.event.data?.character_id)
    .map((r) => ({
      id: r.event.data.character_id,
      label: r.event.data.name || r.event.data.character_id,
    }))
})

// ─── 编剧输出(累积 editor_progress 的 delta) ───
const editorOutput = computed(() => {
  const parts = []
  for (const r of editorEvents.value) {
    // F-44：执笔者的流式输出同样累积（writer_progress），否则续写档下
    // 「编剧」tab 显示"进行中"却没有正文
    if (
      (r.event.event_type === 'editor_progress' || r.event.event_type === 'writer_progress') &&
      r.event.data?.delta
    ) {
      parts.push(r.event.data.delta)
    }
  }
  return parts.join('')
})

const editorDetail = computed(() => {
  if (editorEvents.value.some((r) => r.event.event_type === 'draft_ready')) return '成文完成'
  if (editorEvents.value.some((r) => r.event.event_type.startsWith('writer_'))) {
    return '执笔者续写 · 产出正文'
  }
  if (editorEvents.value.length > 0) return '合并 · 润色 · 成文'
  return ''
})

// ─── 后处理:取最后一条 postprocess_done/failed/skipped 的统计 ───
const postprocessSummary = computed(() => {
  const done = postprocessEvents.value.find((r) => r.event.event_type === 'postprocess_done')
  if (done?.event?.data) {
    const d = done.event.data
    return `知识 ${d.knowledge_count ?? 0} · 变量 ${d.variable_count ?? 0} · 任务 ${d.task_count ?? 0}`
  }
  const failed = postprocessEvents.value.find((r) => r.event.event_type === 'postprocess_failed')
  if (failed) return `后处理失败: ${failed.event.data?.reason || '未知'}`
  const skipped = postprocessEvents.value.find((r) => r.event.event_type === 'postprocess_skipped')
  if (skipped) return `已跳过: ${skipped.event.data?.reason || ''}`
  if (postprocessEvents.value.length > 0) return '提取知识 · 更新变量 · 检测任务'
  return ''
})

// ─── JSON 详情序列化(用于 CodeBlock 展示事件 data) ───
function eventDetailJson(events) {
  if (events.length === 0) return ''
  return JSON.stringify(
    events.map((r) => ({ seq: r.id, type: r.event.event_type, data: r.event.data || {} })),
    null,
    2,
  )
}

// F-47：模板里直接调用 eventDetailJson(...) 会在每次重渲染时对全量事件
// （上限 500 条，含流式 delta）重新 JSON.stringify。改为按阶段 computed 缓存，
// 只有该阶段事件变化时才重算。
const directorDetailJson = computed(() => eventDetailJson(directorEvents.value))
const subagentDetailJson = computed(() => eventDetailJson(subagentEvents.value))
const editorDetailJson = computed(() => eventDetailJson(editorEvents.value))
const postprocessDetailJson = computed(() => eventDetailJson(postprocessEvents.value))
</script>

<template>
  <section class="flex flex-col gap-3">
    <header class="flex items-center gap-2">
      <h3 class="text-sm font-semibold text-ink">流水线 Trace</h3>
      <Badge variant="neutral" size="sm">{{ pipelineEvents.length }} 事件</Badge>
    </header>

    <EmptyState
      v-if="pipelineEvents.length === 0"
      title="无流水线事件"
      description="启动一次生成后,Director / Subagents / Editor / Postprocess 各阶段事件将在此显示"
    />

    <Tabs v-else v-model="activeStage" :tabs="tabs">
      <!-- 导演 -->
      <div v-if="activeStage === 'director'" class="space-y-3">
        <div class="flex items-center gap-2">
          <Badge :variant="statusVariant(lastStatus(directorEvents))" size="sm">
            {{ statusLabel(lastStatus(directorEvents)) }}
          </Badge>
          <span class="text-xs text-ink-soft">{{ directorDetail }}</span>
        </div>
        <CodeBlock
          v-if="directorOutput"
          :code="directorOutput"
          language="director.output"
          :wrap="true"
        />
        <CodeBlock
          v-if="directorEvents.length > 0"
          :code="directorDetailJson"
          language="director.events"
        />
        <EmptyState v-else title="无导演事件" />
      </div>

      <!-- 子 Agent -->
      <div v-if="activeStage === 'subagents'" class="space-y-3">
        <div class="flex items-center gap-2">
          <Badge :variant="statusVariant(lastStatus(subagentEvents))" size="sm">
            {{ statusLabel(lastStatus(subagentEvents)) }}
          </Badge>
          <span class="text-xs text-ink-soft">{{ subagentRoles.length }} 个角色</span>
        </div>
        <ul v-if="subagentRoles.length > 0" class="space-y-1">
          <li
            v-for="role in subagentRoles"
            :key="role.id"
            class="flex items-center gap-2 text-xs text-ink bg-surface-2 rounded px-2 py-1"
          >
            <span class="font-mono text-ink-faint">{{ role.id }}</span>
            <span class="text-ink">{{ role.label }}</span>
          </li>
        </ul>
        <CodeBlock
          v-if="subagentEvents.length > 0"
          :code="subagentDetailJson"
          language="subagent.events"
        />
        <EmptyState v-else title="无子 Agent 事件" />
      </div>

      <!-- 编剧 -->
      <div v-if="activeStage === 'editor'" class="space-y-3">
        <div class="flex items-center gap-2">
          <Badge :variant="statusVariant(lastStatus(editorEvents))" size="sm">
            {{ statusLabel(lastStatus(editorEvents)) }}
          </Badge>
          <span class="text-xs text-ink-soft">{{ editorDetail }}</span>
        </div>
        <CodeBlock
          v-if="editorOutput"
          :code="editorOutput"
          language="editor.output"
          :wrap="true"
        />
        <CodeBlock
          v-if="editorEvents.length > 0"
          :code="editorDetailJson"
          language="editor.events"
        />
        <EmptyState v-else title="无编剧事件" />
      </div>

      <!-- 后处理 -->
      <div v-if="activeStage === 'postprocess'" class="space-y-3">
        <div class="flex items-center gap-2">
          <Badge :variant="statusVariant(lastStatus(postprocessEvents))" size="sm">
            {{ statusLabel(lastStatus(postprocessEvents)) }}
          </Badge>
          <span class="text-xs text-ink-soft">{{ postprocessSummary }}</span>
        </div>
        <CodeBlock
          v-if="postprocessEvents.length > 0"
          :code="postprocessDetailJson"
          language="postprocess.events"
        />
        <EmptyState v-else title="无后处理事件" />
      </div>
    </Tabs>
  </section>
</template>
