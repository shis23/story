<script setup>
import { computed } from 'vue'
import { useTheme } from '../../useTheme.js'
import { useUiStore, useWritingStore, useCampaignStore } from '../../stores/index.js'

const ui = useUiStore()
const writing = useWritingStore()
const campaign = useCampaignStore()
const { theme, toggle } = useTheme()

const emit = defineEmits([
  'close',
  'new-campaign',
  'view-history',
  'open-campaign',
  'open-char-list',
  'import',
  'open-conn',
  'open-preset',
  'open-plugin',
  'open-meta',
  'open-agent-profile',
])

// 导航项(对齐 AppSidebar.vue:40-54)
const items = computed(() => [
  { key: 'new', label: '新建 Campaign', icon: '✚', event: 'new-campaign' },
  {
    key: 'history',
    label: '会话历史',
    icon: '📜',
    event: 'view-history',
    badge: campaign.conversationHistory.length,
  },
  { key: 'campaign', label: 'Campaign 管理', icon: '🎪', event: 'open-campaign', active: writing.writingMode === 'campaign' },
  { key: 'chars', label: '角色卡', icon: '📋', event: 'open-char-list' },
  { key: 'import', label: '导入', icon: '📥', event: 'import' },
  { key: 'conn', label: '连接', icon: '⚡', event: 'open-conn', status: writing.activeConnection ? 'ok' : 'warn' },
  { key: 'preset', label: '预设', icon: '📑', event: 'open-preset' },
  { key: 'plugin', label: '插件', icon: '🔌', event: 'open-plugin' },
  { key: 'agent-profile', label: 'Agent 配置', icon: '🤖', event: 'open-agent-profile' },
  { key: 'meta', label: 'Meta 助手', icon: '🔧', event: 'open-meta' },
])

function click(item) {
  emit(item.event)
  emit('close')
}

const statusClass = { ok: 'text-ok', warn: 'text-warn' }
</script>

<template>
  <div class="flex flex-col h-full w-72 bg-surface">
    <div class="shrink-0 h-14 flex items-center justify-between px-4 border-b border-line">
      <span class="font-semibold text-ink">StoryForge</span>
      <button
        class="w-9 h-9 flex items-center justify-center rounded-lg text-ink-soft hover:bg-surface-2 transition-colors"
        @click="emit('close')"
        aria-label="关闭"
      >
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18 6L6 18M6 6l12 12"/></svg>
      </button>
    </div>

    <div class="flex-1 overflow-y-auto py-2 px-2 space-y-0.5">
      <button
        v-for="item in items"
        :key="item.key"
        class="w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-sm transition-colors"
        :class="item.active ? 'bg-accent-soft text-accent-bright' : 'text-ink-soft hover:text-ink hover:bg-surface-2'"
        @click="click(item)"
      >
        <span class="text-base w-5 text-center">{{ item.icon }}</span>
        <span class="flex-1 text-left">{{ item.label }}</span>
        <span
          v-if="item.badge != null && item.badge > 0"
          class="text-xs text-ink-faint bg-surface-2 rounded-full px-1.5 py-0.5"
        >{{ item.badge }}</span>
        <span
          v-if="item.status"
          class="w-1.5 h-1.5 rounded-full"
          :class="statusClass[item.status]?.replace('text-', 'bg-')"
        ></span>
      </button>
    </div>

    <div class="shrink-0 border-t border-line p-2">
      <button
        class="w-full flex items-center justify-between px-3 py-2 rounded-lg text-sm text-ink-soft hover:bg-surface-2 transition-colors"
        @click="toggle"
      >
        <span>{{ theme === 'dark' ? '☀️ 浅色' : '🌙 深色' }}</span>
        <span class="text-xs">{{ theme }}</span>
      </button>
    </div>
  </div>
</template>
