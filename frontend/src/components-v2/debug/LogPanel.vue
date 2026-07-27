<script setup>
/**
 * LogPanel — 调试抽屉日志列表。
 *
 * 不用窄表格 + break-all（Inspector 里会竖排字符）。
 * 改为：卡片列表 + 消息自然换行；轮询刷新时保留滚动位置。
 * 级别过滤：前端传小写，后端同时兼容大小写。
 */
import { ref, onMounted, onUnmounted, nextTick } from 'vue'
import { logQuery, logClear, logExportBundle } from '../../tauri-api.js'
import Tabs from '../ui/Tabs.vue'
import Select from '../ui/Select.vue'
import IconButton from '../ui/IconButton.vue'
import Badge from '../ui/Badge.vue'
import EmptyState from '../ui/EmptyState.vue'
import LoadingState from '../ui/LoadingState.vue'

const logs = ref([])
const loading = ref(false)
const activeTab = ref('all') // all | backend | llm | frontend
const levelFilter = ref('') // '' = 全部；传小写给后端
const listEl = ref(null)
const expanded = ref(new Set())
let pollTimer = null
let firstLoad = true

const tabs = [
  { key: 'all', label: '全部' },
  { key: 'backend', label: '后端' },
  { key: 'llm', label: 'LLM' },
  { key: 'frontend', label: '前端' },
]

const levelOptions = [
  { value: '', label: '不限' },
  { value: 'error', label: 'Error' },
  { value: 'warn', label: 'Warn 及以上' },
  { value: 'info', label: 'Info 及以上' },
  { value: 'debug', label: 'Debug 及以上' },
]

function levelVariant(level) {
  switch (String(level || '').toLowerCase()) {
    case 'error': return 'err'
    case 'warn': return 'warn'
    case 'info': return 'accent'
    case 'debug': return 'neutral'
    default: return 'neutral'
  }
}

function kindLabel(kind) {
  const k = String(kind || '')
  if (k === 'Backend' || k === 'backend') return '后端'
  if (k === 'LlmCall' || k === 'llm') return 'LLM'
  if (k === 'FrontendPlugin' || k === 'frontend') return '前端'
  return k || '—'
}

function formatTime(ts) {
  try {
    const d = new Date(ts)
    return d.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', second: '2-digit' })
  } catch {
    return ts
  }
}

function isLong(message) {
  return String(message || '').length > 160 || String(message || '').includes('\n')
}

function previewMessage(message) {
  const s = String(message || '')
  if (s.length <= 220) return s
  return s.slice(0, 220) + '…'
}

function isOpen(id) {
  return expanded.value.has(id)
}

function toggle(id) {
  const next = new Set(expanded.value)
  if (next.has(id)) next.delete(id)
  else next.add(id)
  expanded.value = next
}

async function copyMessage(row) {
  try {
    await navigator.clipboard.writeText(row.message || '')
  } catch {
    // ignore
  }
}

function captureScroll() {
  const el = listEl.value
  if (!el) return null
  const max = el.scrollHeight - el.clientHeight
  const nearBottom = max <= 0 || el.scrollTop >= max - 24
  return { top: el.scrollTop, nearBottom }
}

async function restoreScroll(snap) {
  await nextTick()
  const el = listEl.value
  if (!el || !snap) return
  if (snap.nearBottom) {
    el.scrollTop = el.scrollHeight
  } else {
    el.scrollTop = snap.top
  }
}

async function loadLogs({ quiet = false } = {}) {
  if (!quiet) loading.value = true
  const scrollSnap = quiet || !firstLoad ? captureScroll() : null
  const filter = { limit: 120 }
  if (activeTab.value !== 'all') filter.kind = activeTab.value
  // 后端历史只认小写；兼容 UI 若写成 Error 也统一
  if (levelFilter.value) filter.level = String(levelFilter.value).toLowerCase()
  try {
    logs.value = await logQuery(filter)
  } catch (e) {
    console.error('加载日志失败:', e)
  } finally {
    if (!quiet) loading.value = false
    firstLoad = false
    if (scrollSnap) await restoreScroll(scrollSnap)
  }
}

async function handleClear() {
  const kind = activeTab.value === 'all' ? null : activeTab.value
  await logClear(kind)
  await loadLogs()
}

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

function onTabChange() {
  firstLoad = true
  loadLogs()
}

function onLevelChange() {
  firstLoad = true
  loadLogs()
}

onMounted(() => {
  loadLogs()
  // 静默轮询：不闪 Loading、尽量保持滚动位置
  pollTimer = setInterval(() => loadLogs({ quiet: true }), 3000)
})
onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer)
})

defineExpose({ loadLogs })
</script>

<template>
  <section class="flex flex-col gap-3 min-w-0">
    <header class="flex items-center gap-2 min-w-0">
      <h3 class="text-sm font-semibold text-ink shrink-0">日志</h3>
      <Badge variant="neutral" size="sm">{{ logs.length }} 条</Badge>
      <div class="ml-auto flex items-center gap-1 shrink-0">
        <IconButton size="sm" variant="ghost" title="刷新" @click="loadLogs()">
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
      <div class="flex items-end gap-2 mb-3 min-w-0">
        <label class="min-w-[148px] max-w-full">
          <span class="block mb-1 text-[11px] font-medium text-ink-soft">最低级别</span>
          <Select v-model="levelFilter" :options="levelOptions" @update:model-value="onLevelChange" />
        </label>
        <span class="pb-2 text-[10px] leading-none text-ink-faint whitespace-nowrap">保留更严重日志</span>
      </div>

      <LoadingState v-if="loading && logs.length === 0" />

      <EmptyState
        v-else-if="!loading && logs.length === 0"
        title="暂无日志"
        description="写作、连接与抽取过程中的 Info/Error 会显示在这里"
      />

      <div
        v-else
        ref="listEl"
        class="max-h-[28rem] overflow-y-auto overscroll-contain space-y-2 pr-0.5 min-w-0"
      >
        <article
          v-for="row in logs"
          :key="row.id"
          class="rounded-lg border border-line bg-surface px-3 py-2.5 min-w-0"
        >
          <div class="flex items-center gap-2 flex-wrap min-w-0">
            <span class="text-[11px] font-mono text-ink-faint shrink-0">{{ formatTime(row.timestamp) }}</span>
            <Badge :variant="levelVariant(row.level)" size="sm">{{ row.level }}</Badge>
            <span class="text-[11px] text-ink-soft shrink-0">{{ kindLabel(row.kind) }}</span>
            <span
              v-if="row.prompt_tokens != null"
              class="text-[10px] font-mono text-ink-faint"
            >{{ row.prompt_tokens }}+{{ row.completion_tokens ?? 0 }} tok</span>
            <div class="ml-auto flex items-center gap-1 shrink-0">
              <button
                v-if="isLong(row.message)"
                type="button"
                class="text-[11px] text-ink-soft hover:text-ink px-1.5 py-0.5 rounded hover:bg-surface-2"
                @click="toggle(row.id)"
              >{{ isOpen(row.id) ? '收起' : '展开' }}</button>
              <button
                type="button"
                class="text-[11px] text-ink-soft hover:text-ink px-1.5 py-0.5 rounded hover:bg-surface-2"
                title="复制"
                @click="copyMessage(row)"
              >复制</button>
            </div>
          </div>
          <pre
            class="mt-1.5 text-xs text-ink whitespace-pre-wrap break-words font-mono leading-relaxed min-w-0 max-w-full"
          >{{ isOpen(row.id) || !isLong(row.message) ? row.message : previewMessage(row.message) }}</pre>
        </article>
      </div>
    </Tabs>
  </section>
</template>
