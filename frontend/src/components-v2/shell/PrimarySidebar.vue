<script setup>
import { computed } from 'vue'
import { useTheme } from '../../useTheme.js'
import { useUiStore, useWritingStore, useCampaignStore } from '../../stores/index.js'

const ui = useUiStore()
const writing = useWritingStore()
const campaign = useCampaignStore()
const { theme, toggle } = useTheme()

// docked：桌面常驻模式（隐藏关闭按钮）；默认 false（移动端抽屉）
defineProps({
  docked: { type: Boolean, default: false },
})

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

// 内联 SVG 图标（stroke 风格，编辑部细线语言；禁止 emoji）
const iconPaths = {
  history:
    '<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/>',
  book:
    '<path d="M2 4.5h5.5a4 4 0 0 1 4 4V20a3 3 0 0 0-3-3H2z"/><path d="M22 4.5h-5.5a4 4 0 0 0-4 4V20a3 3 0 0 1 3-3H22z"/>',
  users:
    '<path d="M16.5 20v-1.5a4 4 0 0 0-4-4h-6a4 4 0 0 0-4 4V20"/><circle cx="9.5" cy="7.5" r="3.5"/><path d="M21.5 20v-1.5a4 4 0 0 0-2.6-3.75"/><path d="M15 4.25a3.5 3.5 0 0 1 0 6.5"/>',
  import:
    '<path d="M20 15v3.5a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V15"/><path d="M7.5 10.5L12 15l4.5-4.5"/><path d="M12 15V3.5"/>',
  zap:
    '<path d="M13 2.5L4 13.5h7L10 21.5l9-11h-7z"/>',
  file:
    '<path d="M13.5 2.5H7a2 2 0 0 0-2 2v15a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M13.5 2.5V8H19"/><path d="M15.5 13h-7"/><path d="M15.5 16.5h-7"/>',
  package:
    '<path d="M20.5 16v-7.5a2 2 0 0 0-1-1.73l-6.5-3.7a2 2 0 0 0-2 0L4.5 6.77a2 2 0 0 0-1 1.73V16a2 2 0 0 0 1 1.73l6.5 3.7a2 2 0 0 0 2 0l6.5-3.7a2 2 0 0 0 1-1.73z"/><path d="M4 7.2l8 4.55 8-4.55"/><path d="M12 21.5V11.75"/>',
  sliders:
    '<path d="M5 21v-6"/><path d="M5 11V3"/><path d="M12 21v-8"/><path d="M12 9V3"/><path d="M19 21v-4"/><path d="M19 13V3"/><path d="M3 15h4"/><path d="M10 9h4"/><path d="M17 17h4"/>',
  meta:
    '<path d="M20.5 11.5a8 8 0 0 1-8 8 8.2 8.2 0 0 1-3.6-.85L3.5 20.5l1.85-5.4a8 8 0 1 1 15.15-3.6z"/>',
  feather:
    '<path d="M19.7 12.7a5.5 5.5 0 0 0-7.78-7.78L5.5 10.34V18.5h8.16z"/><path d="M15.5 8.5L3 21"/><path d="M17 14.5H9.5"/>',
  sun:
    '<circle cx="12" cy="12" r="4"/><path d="M12 2.5V5"/><path d="M12 19v2.5"/><path d="M4.8 4.8l1.8 1.8"/><path d="M17.4 17.4l1.8 1.8"/><path d="M2.5 12H5"/><path d="M19 12h2.5"/><path d="M4.8 19.2l1.8-1.8"/><path d="M17.4 6.6l1.8-1.8"/>',
  moon:
    '<path d="M20.5 13.2A8.5 8.5 0 1 1 10.8 3.5a7 7 0 0 0 9.7 9.7z"/>',
}

// 导航项（事件与顺序对齐原实现；分区仅为视觉分组）
const sections = computed(() => [
  {
    label: '创作',
    items: [
      {
        key: 'history',
        label: '会话历史',
        icon: 'history',
        event: 'view-history',
        badge: campaign.conversationHistory.length,
      },
      {
        key: 'campaign',
        label: 'Campaign 管理',
        icon: 'book',
        event: 'open-campaign',
        active: writing.writingMode === 'campaign',
      },
    ],
  },
  {
    label: '素材',
    items: [
      { key: 'chars', label: '角色卡', icon: 'users', event: 'open-char-list' },
      { key: 'import', label: '导入', icon: 'import', event: 'import' },
    ],
  },
  {
    label: '系统',
    items: [
      {
        key: 'conn',
        label: '连接',
        icon: 'zap',
        event: 'open-conn',
        status: writing.activeConnection ? 'ok' : 'warn',
      },
      { key: 'preset', label: '预设', icon: 'file', event: 'open-preset' },
      { key: 'plugin', label: '插件', icon: 'package', event: 'open-plugin' },
      { key: 'agent-profile', label: 'Agent 配置', icon: 'sliders', event: 'open-agent-profile' },
      { key: 'meta', label: 'Meta 助手', icon: 'meta', event: 'open-meta' },
    ],
  },
])

function click(item) {
  emit(item.event)
  emit('close')
}

function clickNew() {
  emit('new-campaign')
  emit('close')
}
</script>

<template>
  <div class="flex flex-col h-full w-full bg-bg">
    <!-- 品牌区 -->
    <div class="shrink-0 flex items-center gap-2.5 px-5 h-16 border-b border-line">
      <span class="text-accent" aria-hidden="true">
        <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" v-html="iconPaths.feather"></svg>
      </span>
      <div class="flex-1 min-w-0">
        <div class="font-semibold text-[17px] leading-tight text-ink">StoryForge</div>
        <div class="text-[11px] text-ink-faint leading-tight mt-0.5">互动叙事写作台</div>
      </div>
      <button
        v-if="!docked"
        class="w-9 h-9 flex items-center justify-center rounded-md text-ink-soft hover:bg-surface-2 transition-colors"
        @click="emit('close')"
        aria-label="关闭"
      >
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="M18 6L6 18M6 6l12 12"/></svg>
      </button>
    </div>

    <!-- 主操作 -->
    <div class="shrink-0 px-4 pt-4">
      <button
        class="w-full flex items-center justify-center gap-1.5 h-10 rounded-md bg-accent text-white text-sm font-medium shadow-card hover:bg-accent-bright transition-colors"
        @click="clickNew"
      >
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>
        新建 Campaign
      </button>
    </div>

    <!-- 分区导航 -->
    <nav class="flex-1 overflow-y-auto px-3 py-4 space-y-5">
      <div v-for="section in sections" :key="section.label">
        <div class="px-2.5 pb-1.5 text-[11px] tracking-[0.14em] text-ink-faint select-none">
          {{ section.label }}
        </div>
        <div class="space-y-0.5">
          <button
            v-for="item in section.items"
            :key="item.key"
            class="w-full flex items-center gap-2.5 px-2.5 h-9 rounded-md text-[13px] transition-colors"
            :class="item.active
              ? 'bg-accent-soft text-accent-bright font-medium'
              : 'text-ink-soft hover:text-ink hover:bg-surface-2'"
            @click="click(item)"
          >
            <span class="shrink-0 opacity-80" aria-hidden="true">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" v-html="iconPaths[item.icon]"></svg>
            </span>
            <span class="flex-1 text-left truncate">{{ item.label }}</span>
            <span
              v-if="item.badge != null && item.badge > 0"
              class="text-[11px] leading-none text-ink-faint border border-line rounded-full px-1.5 py-0.5"
            >{{ item.badge }}</span>
            <span
              v-if="item.status"
              class="w-1.5 h-1.5 rounded-full shrink-0"
              :class="item.status === 'ok' ? 'bg-ok' : 'bg-warn'"
              :title="item.status === 'ok' ? '已配置连接' : '未配置连接'"
            ></span>
          </button>
        </div>
      </div>
    </nav>

    <!-- 底部：主题切换 -->
    <div class="shrink-0 border-t border-line p-3">
      <button
        class="w-full flex items-center gap-2.5 px-2.5 h-9 rounded-md text-[13px] text-ink-soft hover:text-ink hover:bg-surface-2 transition-colors"
        @click="toggle"
      >
        <span class="shrink-0 opacity-80" aria-hidden="true">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" v-html="theme === 'dark' ? iconPaths.sun : iconPaths.moon"></svg>
        </span>
        <span class="flex-1 text-left">{{ theme === 'dark' ? '切换为浅色' : '夜读模式' }}</span>
      </button>
    </div>
  </div>
</template>
