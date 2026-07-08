<script setup>
import { ref, onMounted, watch } from 'vue'
import { confirmDialog, alertDialog } from '../../components/base/BaseDialog.js'
import { listTasks, createTask, completeTask, abandonTask } from '../../tauri-api.js'
import { taskStatusText } from '../../utils/taskStatus.js'
import DataTable from '../ui/DataTable.vue'
import Badge from '../ui/Badge.vue'
import Button from '../ui/Button.vue'
import Input from '../ui/Input.vue'
import Textarea from '../ui/Textarea.vue'
import Select from '../ui/Select.vue'
import EmptyState from '../ui/EmptyState.vue'
import LoadingState from '../ui/LoadingState.vue'

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

// ─── status → Badge variant 映射 ───
function statusVariant(task) {
  const s = typeof task.status === 'string' ? task.status : ''
  if (s === 'pending') return 'neutral'
  if (s === 'active') return 'accent'
  if (s === 'completed') return 'ok'
  if (s === 'abandoned') return 'neutral'
  return 'warn' // 复合/未知状态
}

// ─── DataTable 配置 ───
const columns = [
  { key: 'title', label: '任务' },
  { key: 'status', label: '状态', width: '120px' },
  { key: 'meta', label: '来源/轮次', width: '140px' },
]

// ─── 暴露 refresh 给父组件 ───
defineExpose({ refresh: load })
</script>

<template>
  <!-- 状态过滤 -->
  <div class="flex gap-2 mb-3">
    <div class="min-w-[140px]">
      <Select v-model="statusFilter" :options="statusOptions" @update:model-value="onFilterChange" />
    </div>
  </div>

  <!-- 新建任务表单 -->
  <div v-if="showNewTask" class="bg-surface rounded-lg border border-line p-3 space-y-2 mb-3">
    <div class="text-xs font-medium text-ink">新建任务</div>
    <Input v-model="newTaskTitle" placeholder="任务标题" @keyup.enter="handleCreateTask" />
    <Textarea v-model="newTaskDesc" placeholder="任务描述（可选）" :rows="2" />
    <div class="flex gap-2">
      <Button variant="default" size="md" class="flex-1" @click="showNewTask = false">取消</Button>
      <Button
        variant="primary"
        size="md"
        class="flex-1"
        :disabled="!newTaskTitle.trim()"
        @click="handleCreateTask"
      >创建</Button>
    </div>
  </div>

  <LoadingState v-if="loading" />
  <div v-else-if="error" class="text-center text-err text-sm py-8">加载失败: {{ error }}</div>
  <EmptyState v-else-if="tasks.length === 0 && !showNewTask" title="暂无任务">
    <template #action>
      <Button variant="primary" size="md" @click="showNewTask = true">新建任务</Button>
    </template>
  </EmptyState>

  <template v-else>
    <DataTable :columns="columns" :rows="tasks" empty-title="暂无任务">
      <template #cell-title="{ row }">
        <div class="text-sm font-medium text-ink">{{ row.title }}</div>
        <div v-if="row.description" class="text-xs text-ink-soft mt-0.5 line-clamp-2">{{ row.description }}</div>
      </template>
      <template #cell-status="{ row }">
        <Badge :variant="statusVariant(row)" size="sm">{{ taskStatusText(row.status) }}</Badge>
      </template>
      <template #cell-meta="{ row }">
        <div class="text-[10px] text-ink-soft leading-snug">
          <div v-if="row.source">{{ row.source }}</div>
          <div v-if="row.created_turn">轮次 {{ row.created_turn }}</div>
        </div>
      </template>
      <template #row-action="{ row }">
        <div class="flex flex-col gap-1">
          <Button
            v-if="row.status !== 'completed' && row.status?.likely_completed == null"
            variant="default"
            size="sm"
            @click.stop="handleCompleteTask(row.id)"
          >完成</Button>
          <Button
            v-if="row.status !== 'completed' && row.status !== 'abandoned'"
            variant="ghost"
            size="sm"
            @click.stop="handleAbandonTask(row.id)"
          >放弃</Button>
        </div>
      </template>
    </DataTable>

    <!-- 有任务时仍可新建 -->
    <Button
      v-if="!showNewTask"
      variant="default"
      size="md"
      class="w-full mt-3 border-dashed"
      @click="showNewTask = true"
    >+ 新建任务</Button>
  </template>
</template>
