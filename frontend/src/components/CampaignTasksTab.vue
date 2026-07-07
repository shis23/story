<script setup>
import { ref, onMounted, watch } from 'vue'
import { confirmDialog, alertDialog } from './base/BaseDialog.js'
import { listTasks, createTask, completeTask, abandonTask } from '../tauri-api.js'
import { taskStatusClass, taskStatusText } from '../utils/taskStatus.js'

const props = defineProps({
  campaignId: { type: String, required: true }
})

const emit = defineEmits(['refresh'])

// ─── 状态 ───
const tasks = ref([])
const loading = ref(false)
const error = ref(null)
const statusFilter = ref('') // '' = 全部

// ─── 新建任务 ───
const showNewTask = ref(false)
const newTaskTitle = ref('')
const newTaskDesc = ref('')

// ─── 加载 ───
async function load() {
  if (!props.campaignId) return
  loading.value = true
  error.value = null
  try {
    const filter = statusFilter.value || null
    tasks.value = await listTasks(props.campaignId, filter)
  } catch (e) {
    error.value = String(e)
  } finally {
    loading.value = false
  }
}

onMounted(load)

watch(() => props.campaignId, () => {
  if (props.campaignId) load()
})

// ─── 过滤切换时重新加载 ───
function onFilterChange() {
  load()
}

// ─── 任务操作 ───
async function handleCreateTask() {
  if (!newTaskTitle.value.trim()) return
  try {
    await createTask(props.campaignId, newTaskTitle.value.trim(), newTaskDesc.value.trim(), [{ kind: 'manual' }])
    newTaskTitle.value = ''
    newTaskDesc.value = ''
    showNewTask.value = false
    await load()
    emit('refresh')
  } catch (e) {
    await alertDialog('创建任务失败: ' + e)
  }
}

async function handleCompleteTask(taskId) {
  try {
    await completeTask(taskId)
    await load()
    emit('refresh')
  } catch (e) {
    await alertDialog('完成任务失败: ' + e)
  }
}

async function handleAbandonTask(taskId) {
  const ok = await confirmDialog('确定放弃该任务？', { title: '放弃确认' })
  if (!ok) return
  try {
    await abandonTask(taskId)
    await load()
    emit('refresh')
  } catch (e) {
    await alertDialog('放弃任务失败: ' + e)
  }
}

const statusOptions = [
  { value: '', label: '全部状态' },
  { value: 'pending', label: '待处理' },
  { value: 'active', label: '进行中' },
  { value: 'completed', label: '已完成' },
  { value: 'abandoned', label: '已放弃' },
]

// ─── 暴露 refresh 给父组件 ───
defineExpose({ refresh: load })
</script>

<template>
  <!-- 状态过滤 -->
  <div class="flex gap-2 mb-3">
    <select
      v-model="statusFilter"
      @change="onFilterChange"
      class="min-h-[44px] px-3 text-xs rounded-lg border border-line bg-bg focus:outline-none focus:border-accent"
    >
      <option v-for="opt in statusOptions" :key="opt.value" :value="opt.value">{{ opt.label }}</option>
    </select>
  </div>

  <!-- 新建任务表单 -->
  <div v-if="showNewTask" class="bg-surface rounded-xl border border-line p-3 space-y-2 mb-3">
    <div class="text-xs font-medium text-ink">新建任务</div>
    <input
      v-model="newTaskTitle"
      placeholder="任务标题"
      class="w-full min-h-[44px] px-3 text-sm rounded-lg border border-line bg-bg focus:outline-none focus:border-accent"
      @keyup.enter="handleCreateTask"
    />
    <textarea
      v-model="newTaskDesc"
      placeholder="任务描述（可选）"
      rows="2"
      class="w-full px-3 py-2 text-sm rounded-lg border border-line bg-bg focus:outline-none focus:border-accent resize-none"
    ></textarea>
    <div class="flex gap-2">
      <button @click="showNewTask = false" class="flex-1 min-h-[44px] rounded-lg text-xs bg-bg text-ink-soft hover:bg-line transition-colors">取消</button>
      <button
        @click="handleCreateTask"
        :disabled="!newTaskTitle.trim()"
        class="flex-1 min-h-[44px] rounded-lg text-xs font-medium bg-accent text-white disabled:opacity-50 transition-colors"
      >创建</button>
    </div>
  </div>

  <div v-if="loading" class="text-center text-ink-soft text-sm py-8">加载中…</div>
  <div v-else-if="error" class="text-center text-warn text-sm py-8">加载失败: {{ error }}</div>
  <div v-else-if="tasks.length === 0 && !showNewTask" class="text-center py-8">
    <div class="text-ink-soft text-sm mb-3">暂无任务</div>
    <button @click="showNewTask = true" class="min-h-[44px] px-4 rounded-lg text-xs font-medium bg-accent text-white hover:opacity-90 transition-colors">新建任务</button>
  </div>

  <template v-else>
    <!-- 任务列表 -->
    <div
      v-for="task in tasks" :key="task.id"
      class="bg-surface rounded-xl border border-line px-3 py-2.5 mb-2"
    >
      <div class="flex items-start gap-2">
        <div class="flex-1 min-w-0">
          <div class="text-sm font-medium text-ink">{{ task.title }}</div>
          <div v-if="task.description" class="text-xs text-ink-soft mt-0.5 line-clamp-2">{{ task.description }}</div>
          <div class="flex items-center gap-2 mt-1.5 flex-wrap">
            <span class="px-1.5 py-0.5 rounded text-[10px]" :class="taskStatusClass(task.status)">
              {{ taskStatusText(task.status) }}
            </span>
            <span v-if="task.source" class="text-[10px] text-ink-soft">{{ task.source }}</span>
            <span v-if="task.created_turn" class="text-[10px] text-ink-soft">轮次 {{ task.created_turn }}</span>
          </div>
        </div>
        <div class="flex flex-col gap-1 shrink-0">
          <button
            v-if="task.status !== 'completed' && task.status?.likely_completed == null"
            @click="handleCompleteTask(task.id)"
            class="min-h-[36px] px-3 rounded-full text-[10px] font-medium bg-ok/10 text-ok hover:bg-ok/20 transition-colors"
          >完成</button>
          <button
            v-if="task.status !== 'completed' && task.status !== 'abandoned'"
            @click="handleAbandonTask(task.id)"
            class="min-h-[36px] px-3 rounded-full text-[10px] font-medium bg-ink-soft/10 text-ink-soft hover:bg-ink-soft/20 transition-colors"
          >放弃</button>
        </div>
      </div>
    </div>

    <!-- 有任务时仍可新建 -->
    <button
      v-if="!showNewTask"
      @click="showNewTask = true"
      class="w-full min-h-[44px] rounded-lg text-xs font-medium bg-bg text-ink-soft hover:bg-line border border-dashed border-line transition-colors"
    >+ 新建任务</button>
  </template>
</template>
