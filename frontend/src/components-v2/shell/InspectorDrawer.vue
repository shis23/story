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
</script>

<template>
  <div class="flex flex-col h-full w-full min-w-0 bg-surface border-l border-line">
    <div class="shrink-0 h-14 flex items-center justify-between px-4 border-b border-line">
      <span class="font-semibold text-ink text-sm">调试</span>
      <button
        class="w-9 h-9 flex items-center justify-center rounded-lg text-ink-soft hover:bg-surface-2 transition-colors"
        @click="ui.showDebugDrawer = false"
        aria-label="关闭"
      >
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18 6L6 18M6 6l12 12"/></svg>
      </button>
    </div>

    <div class="flex-1 overflow-y-auto p-3">
      <Tabs v-model="activeTab" :tabs="tabs" @update:model-value="onTabChange">
        <PipelineTracePanel v-if="activeTab === 'trace'" />
        <PluginEventLog v-else-if="activeTab === 'events'" />
        <PromptHookAuditLog v-else-if="activeTab === 'hooks'" />
        <LogPanel v-else-if="activeTab === 'logs'" ref="logPanelRef" />
      </Tabs>
    </div>
  </div>
</template>
