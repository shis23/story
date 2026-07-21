<script setup>
/**
 * WritingScreenDemo — 设计预览页（#design-writing）。
 * 仿真壳（侧栏+面包屑顶栏，静态展示用）+ fixture 全状态 + 事件监视条。
 */
import { ref, computed } from 'vue'
import WritingScreen from './WritingScreen.vue'
import {
  fxMessagesWritten,
  fxPipelineDone,
  fxPipelineStreaming,
  fxGreetings,
  fxPageTitle,
  fxDuration,
} from './fixtures.js'

const states = [
  { key: 'empty', label: '空态' },
  { key: 'greeting', label: '开场白' },
  { key: 'streaming', label: '写作中' },
  { key: 'written', label: '已完成（变体）' },
]
const current = ref('streaming')

const screenProps = computed(() => {
  switch (current.value) {
    case 'greeting':
      return { greetingOptions: fxGreetings, selectedGreetingIndex: 0 }
    case 'streaming':
      return {
        title: fxPageTitle,
        messages: [fxMessagesWritten[0]],
        isWriting: true,
        showPipeline: true,
        pipeline: fxPipelineStreaming,
        streamingRoleLabel: '林述 · 旁白',
      }
    case 'written':
      return {
        title: fxPageTitle,
        durationText: fxDuration,
        messages: fxMessagesWritten,
        pipeline: fxPipelineDone,
        canBranch: true,
      }
    default:
      return {}
  }
})

const events = ref([])
function log(name) {
  return (payload) => {
    events.value.unshift({ name, payload: JSON.stringify(payload), at: new Date().toLocaleTimeString() })
    if (events.value.length > 6) events.value.pop()
    console.log(`[design:event] ${name}`, payload)
  }
}

// 仿真壳导航（静态，仅营造上下文）
const navItems = ['首页', '活动', '角色卡', '导入', '连接', '设置']
const activeNav = ref('活动')
</script>

<template>
  <div class="h-screen flex flex-col bg-bg">
    <!-- 预览工具条（dev only） -->
    <div class="shrink-0 flex items-center gap-2 px-4 py-1.5 border-b border-line bg-surface-2/70 z-10">
      <span class="text-[11px] text-ink-faint mr-1">设计预览 · 写作屏</span>
      <button
        v-for="s in states"
        :key="s.key"
        @click="current = s.key"
        class="min-h-6 px-2 rounded text-[11px] transition-colors"
        :class="current === s.key
          ? 'bg-accent-soft text-accent-bright font-medium'
          : 'text-ink-soft hover:bg-surface-2'"
      >{{ s.label }}</button>
      <a href="#" class="ml-auto text-[11px] text-ink-faint hover:text-ink-soft">← 返回应用</a>
    </div>

    <div class="flex-1 flex min-h-0">
      <!-- 仿真侧栏（静态） -->
      <aside class="hidden md:flex flex-col w-56 shrink-0 border-r border-line bg-bg">
        <div class="flex items-center gap-2.5 px-5 h-16 border-b border-line">
          <span class="text-accent">
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M19.7 12.7a5.5 5.5 0 0 0-7.78-7.78L5.5 10.34V18.5h8.16z"/><path d="M15.5 8.5L3 21"/><path d="M17 14.5H9.5"/></svg>
          </span>
          <div>
            <div class="font-semibold text-[16px] leading-tight text-ink">StoryForge</div>
            <div class="text-[10px] text-ink-faint mt-0.5">互动叙事写作台</div>
          </div>
        </div>
        <div class="px-4 pt-4">
          <div class="w-full flex items-center justify-center gap-1.5 h-9 rounded-md bg-accent text-white text-[13px] font-medium shadow-card">
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>
            新建活动
          </div>
        </div>
        <nav class="flex-1 px-3 py-4 space-y-0.5">
          <div
            v-for="item in navItems"
            :key="item"
            class="px-2.5 h-9 flex items-center rounded-md text-[13px]"
            :class="item === activeNav ? 'bg-accent-soft text-accent-bright font-medium' : 'text-ink-soft'"
          >{{ item }}</div>
        </nav>
        <div class="border-t border-line px-5 py-3">
          <div class="text-[10px] text-ink-faint">今日创作</div>
          <div class="mt-1 text-xs text-ink-soft">1,253 字 · 48 分钟</div>
        </div>
      </aside>

      <!-- 主区 -->
      <div class="flex-1 flex flex-col min-w-0">
        <!-- 仿真面包屑顶栏（静态） -->
        <header class="shrink-0 h-[52px] flex items-center gap-2 px-4 sm:px-6 border-b border-line bg-bg">
          <div class="min-w-0">
            <div class="text-[13px] text-ink truncate">
              <span class="text-ink-soft">活动</span>
              <span class="mx-1.5 text-ink-faint">/</span>
              <span class="font-medium">风起之地 · 第一卷</span>
            </div>
            <div class="text-[10px] text-ink-faint mt-0.5">自动保存于 12:46:31</div>
          </div>
          <div class="ml-auto flex items-center gap-1 text-ink-faint">
            <span class="w-8 h-8 flex items-center justify-center rounded-md">
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><rect x="3.5" y="4.5" width="17" height="15" rx="2"/><path d="M14.5 4.5v15"/></svg>
            </span>
          </div>
        </header>

        <div class="flex-1 min-h-0">
          <WritingScreen
            v-bind="screenProps"
            @start-writing="log('start-writing')"
            @cancel="log('cancel')"
            @import="log('import')"
            @new-campaign="log('new-campaign')"
            @view-history="log('view-history')"
            @select-greeting="log('select-greeting')"
            @reroll="log('reroll')"
            @reroll-user="log('reroll-user')"
            @switch-variant="log('switch-variant')"
            @edit-variant="log('edit-variant')"
            @accept-variant="log('accept-variant')"
            @delete-variant="log('delete-variant')"
            @add-variant="log('add-variant')"
            @branch="log('branch')"
          />
        </div>
      </div>
    </div>

    <!-- 事件监视条 -->
    <div v-if="events.length" class="shrink-0 border-t border-line bg-surface px-4 py-2 max-h-28 overflow-y-auto z-10">
      <div v-for="(e, i) in events" :key="i" class="text-[11px] font-mono text-ink-soft truncate">
        <span class="text-ink-faint">{{ e.at }}</span>
        <span class="text-accent-bright"> {{ e.name }}</span>
        <span> {{ e.payload }}</span>
      </div>
    </div>
  </div>
</template>
