<script setup>
import { ref, onMounted, watch } from 'vue'
import { confirmDialog, alertDialog } from '../../components/base/BaseDialog.js'
import { listTasks, createTask, completeTask, abandonTask } from '../../tauri-api.js'
import { taskStatusText, isLikelyCompleted } from '../../utils/taskStatus.js'
import DataTable from '../ui/DataTable.vue'
import Badge from '../ui/Badge.vue'
import Button from '../ui/Button.vue'
import Input from '../ui/Input.vue'
import Textarea from '../ui/Textarea.vue'
import Select from '../ui/Select.vue'
import EmptyState from '../ui/EmptyState.vue'
import LoadingState from '../ui/LoadingState.vue'
import { errorText } from '../../utils/errorText.js'

const props = defineProps({
  campaignId: { type: String, required: true }
})

// ─── 状态 ───
const tasks = ref([])
const loading = ref(false)
const error = ref(null)
const statusFilter = ref('') // '' = 全部
// 写操作防重（F-24）：这两个写命令都不幂等，连点会产生重复任务/重复导模块。
const creatingTask = ref(false)
const busyTaskId = ref(null)

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
    error.value = errorText(e)
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
  if (!newTaskTitle.value.trim() || creatingTask.value) return
  creatingTask.value = true
  try {
    await createTask(props.campaignId, newTaskTitle.value.trim(), newTaskDesc.value.trim(), [{ kind: 'manual' }])
    newTaskTitle.value = ''
    newTaskDesc.value = ''
    showNewTask.value = false
    await load()
  } catch (e) {
    await alertDialog('创建任务失败: ' + errorText(e))
  } finally {
    creatingTask.value = false
  }
}

async function handleCompleteTask(taskId) {
  if (busyTaskId.value) return
  busyTaskId.value = taskId
  try {
    await completeTask(taskId)
    await load()
  } catch (e) {
    await alertDialog('完成任务失败: ' + errorText(e))
  } finally {
    busyTaskId.value = null
  }
}

async function handleAbandonTask(taskId) {
  if (busyTaskId.value) return
  const ok = await confirmDialog('确定放弃该任务？', { title: '放弃确认' })
  if (!ok) return
  busyTaskId.value = taskId
  try {
    await abandonTask(taskId)
    await load()
  } catch (e) {
    await alertDialog('放弃任务失败: ' + errorText(e))
  } finally {
    busyTaskId.value = null
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
        :loading="creatingTask"
        @click="handleCreateTask"
      >{{ creatingTask ? '创建中…' : '创建' }}</Button>
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
    <DataTable :columns="columns" :rows="tasks" row-key="id" empty-title="暂无任务">
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
          <!--
            F-07：`likely_completed`（agent 自报可能完成）必须由用户确认或驳回，
            后端不会自动终态。此前该分支把「完成」按钮藏起来，导致这类任务永远无法终态。
          -->
          <Button
            v-if="row.status !== 'completed' && row.status !== 'abandoned'"
            variant="default"
            size="sm"
            :loading="busyTaskId === row.id"
            @click.stop="handleCompleteTask(row.id)"
          >{{ isLikelyCompleted(row.status) ? '确认完成' : '完成' }}</Button>
          <Button
            v-if="row.status !== 'completed' && row.status !== 'abandoned'"
            variant="ghost"
            size="sm"
            :disabled="busyTaskId === row.id"
            @click.stop="handleAbandonTask(row.id)"
          >{{ isLikelyCompleted(row.status) ? '误判，放弃' : '放弃' }}</Button>
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
