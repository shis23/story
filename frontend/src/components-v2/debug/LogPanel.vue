<script setup>
/**
 * LogPanel — app/frontend 日志查询面板。
 *
 * 迁移自 src/components/LogPanel.vue。调 tauri-api:
 *   log_query / log_clear / log_export_bundle。
 * 用 ui/DataTable 渲染日志条(level / timestamp / message),支持 level 过滤(ui/Select)。
 * 复制 / 导出按钮。
 *
 * 保留原 tab(全部 / 后端 / LLM / 前端)与 level 过滤、刷新 / 清空 / 导出 bundle 行为。
 */
import { ref, onMounted, onUnmounted } from 'vue'
import { logQuery, logClear, logExportBundle } from '../../tauri-api.js'
import Tabs from '../ui/Tabs.vue'
import Select from '../ui/Select.vue'
import Button from '../ui/Button.vue'
import IconButton from '../ui/IconButton.vue'
import DataTable from '../ui/DataTable.vue'
import Badge from '../ui/Badge.vue'
import EmptyState from '../ui/EmptyState.vue'
import LoadingState from '../ui/LoadingState.vue'

const logs = ref([])
const loading = ref(false)
const activeTab = ref('all') // all | backend | llm | frontend
const levelFilter = ref('') // '' = 全部

// ─── 加载日志 ───
async function loadLogs() {
  loading.value = true
  const filter = { limit: 100 }
  if (activeTab.value !== 'all') filter.kind = activeTab.value
  if (levelFilter.value) filter.level = levelFilter.value
  try {
    logs.value = await logQuery(filter)
  } catch (e) {
    console.error('加载日志失败:', e)
  } finally {
    loading.value = false
  }
}

// ─── 清空日志 ───
async function handleClear() {
  const kind = activeTab.value === 'all' ? null : activeTab.value
  await logClear(kind)
  await loadLogs()
}

// ─── 导出 bundle(用 Tauri 文件对话框,与原实现一致) ───
async function handleExport() {
  try {
    const bundle = await logExportBundle(true)
    const json = JSON.stringify(bundle, null, 2)
    const data = new TextEncoder().encode(json)
    const { save } = await import('@tauri-apps/plugin-dialog')
    const filePath = await save({
      defaultPath: `storyforge-logs.json`,
      filters: [{ name: 'JSON', extensions: ['json'] }],
    })
    if (filePath) {
      const { writeBinaryFile } = await import('@tauri-apps/plugin-fs')
      await writeBinaryFile(filePath, data)
    }
  } catch (e) {
    console.error('导出失败:', e)
  }
}

// ─── 切换 tab ───
function onTabChange() {
  loadLogs()
}

function onLevelChange() {
  loadLogs()
}

// ─── 格式化时间 ───
function formatTime(ts) {
  try {
    const d = new Date(ts)
    return d.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', second: '2-digit' })
  } catch {
    return ts
  }
}

// ─── level → Badge variant(走主题 token) ───
function levelVariant(level) {
  switch (level) {
    case 'Error': return 'err'
    case 'Warn': return 'warn'
    case 'Info': return 'accent'
    case 'Debug': return 'neutral'
    default: return 'neutral'
  }
}

const tabs = [
  { key: 'all', label: '全部' },
  { key: 'backend', label: '后端' },
  { key: 'llm', label: 'LLM' },
  { key: 'frontend', label: '前端' },
]

const levelOptions = [
  { value: '', label: '全部级别' },
  { value: 'Error', label: 'Error' },
  { value: 'Warn', label: 'Warn' },
  { value: 'Info', label: 'Info' },
  { value: 'Debug', label: 'Debug' },
]

// ─── DataTable 配置 ───
const columns = [
  { key: 'timestamp', label: '时间', width: '90px' },
  { key: 'level', label: '级别', width: '80px' },
  { key: 'kind', label: '来源', width: '80px' },
  { key: 'message', label: '消息' },
]

// 定时轮询：写作/抽取产生的日志会持续写入 LogStore 内存 buffer，
// 只 onMounted 拉一次的话用户看不到后续日志。3 秒轮询保证日志面板实时刷新。
let pollTimer = null
onMounted(() => {
  loadLogs()
  pollTimer = setInterval(loadLogs, 3000)
})
onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer)
})

defineExpose({ loadLogs })
</script>

<template>
  <section class="flex flex-col gap-3">
    <header class="flex items-center gap-2">
      <h3 class="text-sm font-semibold text-ink">日志</h3>
      <Badge variant="neutral" size="sm">{{ logs.length }} 条</Badge>
      <div class="ml-auto flex items-center gap-1">
        <IconButton size="sm" variant="ghost" title="刷新" @click="loadLogs">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12a9 9 0 1 1-2.64-6.36"/><path d="M21 3v6h-6"/></svg>
        </IconButton>
        <IconButton size="sm" variant="ghost" title="导出 bundle" @click="handleExport">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v3.5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V15"/><path d="M7 10l5 5 5-5"/><path d="M12 15V3"/></svg>
        </IconButton>
        <IconButton size="sm" variant="ghost" title="清空日志" @click="handleClear">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
        </IconButton>
      </div>
    </header>

    <Tabs v-model="activeTab" :tabs="tabs" @update:model-value="onTabChange">
      <div class="flex gap-2 mb-3">
        <div class="min-w-[140px]">
          <Select v-model="levelFilter" :options="levelOptions" @update:model-value="onLevelChange" />
        </div>
      </div>

      <LoadingState v-if="loading" />

      <EmptyState
        v-else-if="logs.length === 0"
        title="暂无日志"
      />

      <div v-else class="max-h-96 overflow-y-auto rounded-lg border border-line">
        <DataTable :columns="columns" :rows="logs" empty-title="暂无日志">
          <template #cell-timestamp="{ row }">
            <span class="text-xs font-mono text-ink-faint">{{ formatTime(row.timestamp) }}</span>
          </template>
          <template #cell-level="{ row }">
            <Badge :variant="levelVariant(row.level)" size="sm">{{ row.level }}</Badge>
          </template>
          <template #cell-kind="{ row }">
            <span class="text-xs text-ink-soft">[{{ row.kind }}]</span>
          </template>
          <template #cell-message="{ row }">
            <span class="text-xs font-mono text-ink break-all">{{ row.message }}</span>
          </template>
        </DataTable>
      </div>
    </Tabs>
  </section>
</template>
