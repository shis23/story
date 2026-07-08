<script setup>
import { computed } from 'vue'
import { useUiStore, useWritingStore, useCampaignStore } from '../../stores/index.js'

const ui = useUiStore()
const writing = useWritingStore()
const campaign = useCampaignStore()

// 副标题(对应 App.vue:1233-1238 的状态文本)
const subtitle = computed(() => {
  if (writing.isWriting) return '写作中…'
  if (writing.writingMode === 'campaign')
    return `Campaign · ${campaign.activeCampaign?.story_clock || '第 1 轮'}`
  if (writing.writingMode === 'legacy') return '兼容模式'
  return '导入角色卡或打开 Campaign'
})
</script>

<template>
  <header class="glass shrink-0 h-14 flex items-center gap-2 px-3 border-b border-line">
    <button
      class="w-11 h-11 flex items-center justify-center rounded-lg text-ink-soft hover:bg-surface-2 transition-colors"
      @click="ui.showSidebar = true"
      aria-label="菜单"
    >
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M3 6h18M3 12h18M3 18h18"/></svg>
    </button>

    <div class="flex-1 min-w-0 text-center">
      <div class="font-semibold text-ink truncate text-sm">{{ ui.pageTitle }}</div>
      <div class="text-[11px] text-ink-soft truncate">{{ subtitle }}</div>
    </div>

    <div v-if="writing.isWriting" class="flex items-center gap-1.5 px-2.5 py-1 rounded-full bg-accent-soft">
      <span class="w-1.5 h-1.5 rounded-full bg-accent animate-pulse"></span>
      <span class="text-[11px] text-accent font-medium">生成中</span>
    </div>

    <button
      class="w-11 h-11 flex items-center justify-center rounded-lg text-ink-soft hover:bg-surface-2 transition-colors"
      @click="ui.showDebugDrawer = true"
      aria-label="调试"
    >🛠</button>
  </header>
</template>
