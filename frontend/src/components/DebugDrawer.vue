<script setup>
/**
 * DebugDrawer — 右调试区
 *
 * 双形态：
 *   - 桌面（lg+）：常驻右栏，由父组件用 hidden lg:flex 控制
 *   - 移动（<lg）：抽屉，由父组件包 BaseOverlay 触发
 *
 * 承载 Agent 配置 / Profile / 日志 / 插件侧栏 四个 tab。
 * 调试区与写作面分区：bg-surface + border-l，专业工具感。
 */
import { ref } from 'vue'
import AgentConfigCard from './AgentConfigCard.vue'
import AgentProfileManager from './AgentProfileManager.vue'
import LogPanel from './LogPanel.vue'
import PluginHost from './PluginHost.vue'

const props = defineProps({
  /** 插件侧栏列表（来自 App.vue sidebarPlugins） */
  sidebarPlugins: { type: Array, default: () => [] },
  mobile: { type: Boolean, default: false },
})
const emit = defineEmits(['open-connection-config', 'close'])

const activeTab = ref('agent') // agent | profile | log | plugins
const agentConfigRef = ref(null)

const tabs = [
  { key: 'agent', label: 'Agent', icon: '🎬' },
  { key: 'profile', label: 'Profile', icon: '⚙️' },
  { key: 'log', label: '日志', icon: '📋' },
  { key: 'plugins', label: '插件', icon: '🧩' },
]

// 暴露 AgentConfigCard ref，供父组件连接变更后刷新
defineExpose({
  loadConnections: () => agentConfigRef.value?.loadConnections?.(),
})
</script>

<template>
  <aside class="flex flex-col h-full w-full bg-surface border-l border-line">
    <!-- 顶栏：标题 + tab -->
    <div class="shrink-0 border-b border-line">
      <div class="flex items-center gap-2 px-4 h-14">
        <span class="text-sm font-semibold text-ink">调试</span>
        <span class="text-[10px] px-1.5 py-0.5 rounded bg-accent-soft text-accent font-medium">PRO</span>
        <button
          @click="emit('close')"
          class="ml-auto w-9 h-9 flex items-center justify-center rounded-lg text-ink-soft hover:bg-surface-2 transition-colors"
          aria-label="关闭"
        >
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18 6 6 18M6 6l12 12"/></svg>
        </button>
      </div>
      <!-- tab 条 -->
      <div class="flex px-2 gap-0.5">
        <button
          v-for="t in tabs"
          :key="t.key"
          @click="activeTab = t.key"
          class="flex-1 min-h-[40px] flex items-center justify-center gap-1.5 text-xs font-medium border-b-2 transition-colors"
          :class="activeTab === t.key
            ? 'border-accent text-accent'
            : 'border-transparent text-ink-soft hover:text-ink'"
        >
          <span>{{ t.icon }}</span>{{ t.label }}
        </button>
      </div>
    </div>

    <!-- 内容区（独立滚动，与写作面分区） -->
    <div class="flex-1 min-h-0 overflow-y-auto p-3 bg-bg">
      <AgentConfigCard
        v-show="activeTab === 'agent'"
        ref="agentConfigRef"
        :embedded="true"
        @open-connection-config="$emit('open-connection-config')"
      />
      <AgentProfileManager v-show="activeTab === 'profile'" :embedded="true" />
      <div v-show="activeTab === 'log'">
        <LogPanel />
      </div>
      <div v-show="activeTab === 'plugins'">
        <div v-if="sidebarPlugins.length === 0" class="text-center text-ink-faint text-xs py-8">
          无侧栏插件<br>
          <span class="text-[10px]">在「插件」管理中启用声明 SidebarPanel 的插件</span>
        </div>
        <div v-else class="space-y-2">
          <PluginHost
            v-for="p in sidebarPlugins"
            :key="p.id"
            :plugin="p"
            height="160px"
          />
        </div>
      </div>
    </div>
  </aside>
</template>
