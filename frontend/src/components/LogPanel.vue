<script setup>
import { ref, onMounted } from 'vue'
import { logQuery, logClear, logExportBundle } from '../tauri-api.js'

const logs = ref([])
const filter = ref({ kind: null, level: null, keyword: '', limit: 100 })
const activeTab = ref('all') // all | backend | llm | frontend

// 加载日志
async function loadLogs() {
  const f = { ...filter.value }
  if (activeTab.value !== 'all') {
    f.kind = activeTab.value
  }
  try {
    logs.value = await logQuery(f)
  } catch (e) {
    console.error('加载日志失败:', e)
  }
}

// 清空日志
async function handleClear() {
  const kind = activeTab.value === 'all' ? null : activeTab.value
  await logClear(kind)
  await loadLogs()
}

// 导出 bundle
async function handleExport() {
  try {
    const bundle = await logExportBundle(true)
    // 复制到剪贴板或下载
    const blob = new Blob([JSON.stringify(bundle, null, 2)], { type: 'application/json' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = `storyforge-logs-${new Date().toISOString().slice(0, 10)}.json`
    a.click()
    URL.revokeObjectURL(url)
  } catch (e) {
    console.error('导出失败:', e)
  }
}

// 切换 tab
function switchTab(tab) {
  activeTab.value = tab
  loadLogs()
}

// 格式化时间
function formatTime(ts) {
  try {
    const d = new Date(ts)
    return d.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', second: '2-digit' })
  } catch {
    return ts
  }
}

// 级别颜色
function levelColor(level) {
  switch (level) {
    case 'Error': return 'text-red-500'
    case 'Warn': return 'text-yellow-500'
    case 'Info': return 'text-blue-400'
    case 'Debug': return 'text-gray-400'
    default: return 'text-gray-300'
  }
}

onMounted(loadLogs)

defineExpose({ loadLogs })
</script>

<template>
  <div class="bg-bg border border-line rounded-xl overflow-hidden">
    <!-- Tab 栏 -->
    <div class="flex items-center border-b border-line bg-bg/50">
      <button
        v-for="tab in [
          { key: 'all', label: '全部' },
          { key: 'backend', label: '后端' },
          { key: 'llm', label: 'LLM' },
          { key: 'frontend', label: '前端' },
        ]"
        :key="tab.key"
        @click="switchTab(tab.key)"
        class="px-3 py-1.5 text-xs font-medium transition-colors"
        :class="activeTab === tab.key
          ? 'text-accent border-b-2 border-accent'
          : 'text-ink-soft hover:text-ink'"
      >
        {{ tab.label }}
      </button>
      <div class="flex-1"></div>
      <button @click="handleExport" class="px-2 py-1 text-xs text-ink-soft hover:text-accent" title="导出 bundle">
        📦
      </button>
      <button @click="handleClear" class="px-2 py-1 text-xs text-ink-soft hover:text-red-400" title="清空日志">
        🗑
      </button>
      <button @click="loadLogs" class="px-2 py-1 text-xs text-ink-soft hover:text-accent" title="刷新">
        🔄
      </button>
    </div>

    <!-- 日志列表 -->
    <div class="max-h-64 overflow-y-auto divide-y divide-line/30">
      <div
        v-for="log in logs"
        :key="log.id"
        class="px-3 py-1.5 text-xs font-mono hover:bg-line/20"
      >
        <span class="text-ink-soft/50">{{ formatTime(log.timestamp) }}</span>
        <span :class="levelColor(log.level)" class="ml-1 font-bold">{{ log.level }}</span>
        <span class="ml-1 text-ink-soft/70">[{{ log.kind }}]</span>
        <span class="ml-2 text-ink">{{ log.message }}</span>
      </div>
      <div v-if="logs.length === 0" class="px-3 py-4 text-center text-xs text-ink-soft/50">
        暂无日志
      </div>
    </div>
  </div>
</template>
