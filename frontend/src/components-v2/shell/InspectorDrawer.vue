<script setup>
/**
 * InspectorDrawer — 调试抽屉（右栏 / overlay）。
 *
 * 用 ui/Tabs 分四个面板,每个对应一个 debug 组件:
 *   - trace   → PipelineTracePanel(流水线 trace:导演/子 Agent/编剧/后处理)
 *   - events  → PluginEventLog(插件事件 feed)
 *   - hooks   → PromptHookAuditLog(prompt hook 审计)
 *   - logs    → LogPanel(app/frontend 日志)
 */
import { ref } from 'vue'
import { useUiStore } from '../../stores/index.js'
import Tabs from '../ui/Tabs.vue'
import PipelineTracePanel from '../debug/PipelineTracePanel.vue'
import PluginEventLog from '../debug/PluginEventLog.vue'
import PromptHookAuditLog from '../debug/PromptHookAuditLog.vue'
import LogPanel from '../debug/LogPanel.vue'
import { cardShellClearCache } from '../../tauri-api.js'

const ui = useUiStore()

const activeTab = ref('trace')
const tabs = [
  { key: 'trace', label: '流水线' },
  { key: 'events', label: '插件事件' },
  { key: 'hooks', label: 'Hook 审计' },
  { key: 'logs', label: '日志' },
]

// LogPanel 暴露 loadLogs,切到日志 tab 时刷新
const logPanelRef = ref(null)
function onTabChange(key) {
  if (key === 'logs') logPanelRef.value?.loadLogs?.()
}

// L6 收尾：卡壳磁盘缓存清空（壳资源刷新通道）。放调试抽屉页脚——
// 专业工具面，不占写作面。
const clearingShellCache = ref(false)
const clearShellCacheResult = ref('')
async function clearShellCache() {
  if (clearingShellCache.value) return
  clearingShellCache.value = true
  clearShellCacheResult.value = ''
  try {
    const n = await cardShellClearCache()
    clearShellCacheResult.value = `已清 ${n ?? 0} 个缓存对象`
  } catch (e) {
    clearShellCacheResult.value = `清空失败: ${e?.message || e}`
  } finally {
    clearingShellCache.value = false
  }
}
</script>

<template>
  <div class="flex flex-col h-full w-full min-w-0 bg-surface border-l border-line">
    <div class="sf-toolbar flex items-center justify-between px-4 border-b border-line">
      <span class="font-semibold text-ink text-sm">调试</span>
      <button
        type="button"
        class="sf-toolbar-icon transition-colors"
        @click="ui.showDebugDrawer = false"
        aria-label="关闭"
        title="关闭"
      >
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18 6L6 18M6 6l12 12"/></svg>
      </button>
    </div>

    <div class="sf-drawer-scroll flex-1 min-h-0 min-w-0 overflow-y-auto overscroll-y-contain p-3 pb-10">
      <Tabs v-model="activeTab" :tabs="tabs" @update:model-value="onTabChange">
        <div class="min-w-0 pt-1">
          <PipelineTracePanel v-if="activeTab === 'trace'" />
          <PluginEventLog v-else-if="activeTab === 'events'" />
          <PromptHookAuditLog v-else-if="activeTab === 'hooks'" />
          <LogPanel v-else-if="activeTab === 'logs'" ref="logPanelRef" />
        </div>
      </Tabs>
    </div>

    <!-- 维护区：卡壳缓存刷新通道（L6 UI 入口） -->
    <div class="shrink-0 border-t border-line px-3 py-2 flex items-center gap-2">
      <button
        type="button"
        class="text-[11px] px-2 py-1 rounded-md border border-line text-ink-soft hover:text-ink hover:bg-surface-2 disabled:opacity-50"
        :disabled="clearingShellCache"
        @click="clearShellCache"
      >
        {{ clearingShellCache ? '清空中…' : '清空卡壳缓存' }}
      </button>
      <span v-if="clearShellCacheResult" class="text-[11px] text-ink-faint truncate">
        {{ clearShellCacheResult }}
      </span>
    </div>
  </div>
</template>
