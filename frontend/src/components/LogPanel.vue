<script setup>
import { ref, onMounted } from 'vue'
import { logQuery, logClear, logExportBundle } from '../tauri-api.js'

const logs = ref([])
const loading = ref(false)
const filter = ref({ kind: null, level: null, keyword: '', limit: 100 })
const activeTab = ref('all') // all | backend | llm | frontend

// 加载日志
async function loadLogs() {
  loading.value = true
  const f = { ...filter.value }
  if (activeTab.value !== 'all') {
    f.kind = activeTab.value
  }
  try {
    logs.value = await logQuery(f)
  } catch (e) {
    console.error('加载日志失败:', e)
  } finally {
    loading.value = false
  }
}

// 清空日志
async function handleClear() {
  const kind = activeTab.value === 'all' ? null : activeTab.value
  await logClear(kind)
  await loadLogs()
}

// 导出 bundle（用 Tauri 文件对话框，与 CampaignPanel 导出一致）
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

// 级别颜色（走主题 token：Error=err / Warn=warn / Info=running / Debug=ink-soft）
function levelColor(level) {
  switch (level) {
    case 'Error': return 'text-err'
    case 'Warn': return 'text-warn'
    case 'Info': return 'text-running'
    case 'Debug': return 'text-ink-soft'
    default: return 'text-ink-soft'
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
        class="min-h-[44px] px-3 text-xs font-medium transition-colors"
        :class="activeTab === tab.key
          ? 'text-accent border-b-2 border-accent'
          : 'text-ink-soft hover:text-ink'"
      >
        {{ tab.label }}
      </button>
      <div class="flex-1"></div>
      <button @click="handleExport" class="w-11 h-11 flex items-center justify-center text-xs text-ink-soft hover:text-accent transition-colors" title="导出 bundle">
        📦
      </button>
      <button @click="handleClear" class="w-11 h-11 flex items-center justify-center text-xs text-ink-soft hover:text-err transition-colors" title="清空日志">
        🗑
      </button>
      <button @click="loadLogs" class="w-11 h-11 flex items-center justify-center text-xs text-ink-soft hover:text-accent transition-colors" title="刷新">
        🔄
      </button>
    </div>

    <!-- 日志列表 -->
    <div class="max-h-64 overflow-y-auto divide-y divide-line/30">
      <div v-if="loading" class="px-3 py-4 text-center text-xs text-ink-soft">
        加载中…
      </div>
      <template v-else>
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
      </template>
    </div>
  </div>
</template>
