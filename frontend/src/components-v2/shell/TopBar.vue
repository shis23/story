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
  <header class="shrink-0 h-[52px] flex items-center gap-2 px-3 sm:px-4 bg-bg border-b border-line">
    <!-- 移动端菜单（桌面侧栏常驻，隐藏） -->
    <button
      class="lg:hidden w-10 h-10 flex items-center justify-center rounded-md text-ink-soft hover:bg-surface-2 transition-colors"
      @click="ui.showSidebar = true"
      aria-label="菜单"
    >
      <svg width="19" height="19" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="M4 7h16M4 12h16M4 17h16"/></svg>
    </button>

    <!-- 标题：桌面左对齐面包屑感，移动端居中 -->
    <div class="flex-1 min-w-0 text-center lg:text-left lg:pl-2">
      <div class="font-semibold text-ink truncate text-[15px] leading-tight">{{ ui.pageTitle }}</div>
      <div class="text-[11px] text-ink-faint truncate leading-tight mt-0.5">{{ subtitle }}</div>
    </div>

    <!-- 生成状态徽章 -->
    <div
      v-if="writing.isWriting"
      class="flex items-center gap-1.5 px-2.5 py-1 rounded-full border border-running/30 bg-running/10"
    >
      <span class="w-1.5 h-1.5 rounded-full bg-running animate-pulse"></span>
      <span class="text-[11px] text-running font-medium">生成中</span>
    </div>

    <!-- 调试/高玩抽屉入口（退后：低对比图标） -->
    <button
      class="w-10 h-10 flex items-center justify-center rounded-md text-ink-faint hover:text-ink-soft hover:bg-surface-2 transition-colors"
      @click="ui.showDebugDrawer = true"
      aria-label="过程与调试"
      title="过程与调试"
    >
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><rect x="3.5" y="4.5" width="17" height="15" rx="2"/><path d="M14.5 4.5v15"/></svg>
    </button>
  </header>
</template>
