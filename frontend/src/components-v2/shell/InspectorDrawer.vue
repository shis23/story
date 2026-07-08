<script setup>
import { useUiStore, usePluginStore } from '../../stores/index.js'
import EmptyState from '../ui/EmptyState.vue'

const ui = useUiStore()
const plugin = usePluginStore()
</script>

<template>
  <div class="flex flex-col h-full w-80 bg-surface border-l border-line">
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

    <div class="flex-1 overflow-y-auto p-3 space-y-4">
      <!-- 阶段 7 补充:PipelineTracePanel / PluginEventLog / PromptHookAuditLog -->

      <EmptyState
        v-if="plugin.pluginPipelineEvents.length === 0"
        title="无调试事件"
        description="流水线事件、插件事件、prompt hook 审计将在此显示"
      />

      <div v-else>
        <h3 class="text-xs text-ink-soft mb-2">最近事件</h3>
        <div class="space-y-1">
          <div
            v-for="evt in plugin.pluginPipelineEvents.slice(-20).reverse()"
            :key="evt.seq"
            class="text-xs text-ink-soft bg-surface-2 rounded px-2 py-1 font-mono"
          >
            {{ evt.type }} <span class="text-ink-faint">{{ evt.detail }}</span>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
