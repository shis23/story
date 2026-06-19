<script setup>
/**
 * AppSidebar — 左导航栏
 *
 * 双形态：
 *   - 桌面（lg+）：常驻侧栏，由父组件用 hidden lg:flex 控制
 *   - 移动（<lg）：抽屉，由父组件包 BaseOverlay 或本组件 mobile prop 触发
 *
 * 承载所有管理入口（Campaign/角色卡/预设/插件/连接/Meta/导入）+ 写作导航（新建对话/会话历史）+ 底部主题/高玩开关。
 * 顶栏不再堆按钮，所有入口收归此处。
 */
import { useTheme } from '../useTheme.js'

const props = defineProps({
  powerMode: { type: Boolean, default: false },
  writingMode: { type: String, default: 'none' }, // 'campaign' | 'legacy' | 'none'
  activeCampaignName: { type: String, default: null },
  activeCharName: { type: String, default: null },
  activeConnection: { type: Object, default: null },
  /** 当前中间栏视图：'write' | 'history' | 'overview'，用于高亮 */
  view: { type: String, default: 'write' },
  conversationCount: { type: Number, default: 0 },
  /** 移动抽屉模式：点击项后触发 close */
  mobile: { type: Boolean, default: false },
})

const emit = defineEmits([
  'open-campaign', 'open-char-list', 'open-preset', 'open-plugin',
  'open-conn', 'open-meta', 'import', 'new-campaign',
  'view-write', 'view-history', 'view-overview',
  'toggle-power', 'close',
])

const { theme, toggle: toggleTheme } = useTheme()

const navItems = [
  // 写作组
  { group: '写作', items: [
    { key: 'new', label: '新建 Campaign', icon: '✚', emit: 'new-campaign' },
    { key: 'history', label: '会话历史', icon: '📜', emit: 'view-history', badge: props.conversationCount },
  ]},
  // 素材组
  { group: '素材', items: [
    { key: 'campaign', label: 'Campaign 管理', icon: '🎪', emit: 'open-campaign', active: props.writingMode === 'campaign' },
    { key: 'chars', label: '角色卡', icon: '📋', emit: 'open-char-list' },
    { key: 'import', label: '导入', icon: '📥', emit: 'import' },
  ]},
  // 配置组
  { group: '配置', items: [
    { key: 'conn', label: '连接', icon: '⚡', emit: 'open-conn', status: props.activeConnection ? 'ok' : 'warn' },
    { key: 'preset', label: '预设', icon: '📑', emit: 'open-preset' },
    { key: 'plugin', label: '插件', icon: '🔌', emit: 'open-plugin' },
    { key: 'meta', label: 'Meta 助手', icon: '🔧', emit: 'open-meta' },
  ]},
]

function onItemClick(item) {
  emit(item.emit)
  // 点导航项关闭左导航，打开子面板；子面板返回时会重新打开左导航
  emit('close')
}
</script>

<template>
  <!-- 根宽度撑满抽屉容器（w-full），消除"背景比内容宽" -->
  <nav class="flex flex-col h-full w-full bg-surface border-r border-line">
    <!-- 顶：品牌 -->
    <div class="shrink-0 px-4 h-14 flex items-center gap-2.5 border-b border-line">
      <div class="w-7 h-7 rounded-lg bg-accent shadow-glow-accent flex items-center justify-center text-sm font-bold text-white">S</div>
      <span class="font-semibold text-ink tracking-tight">StoryForge</span>
      <!-- 关闭抽屉，返回主栏 -->
      <button
        @click="emit('close')"
        class="ml-auto w-9 h-9 flex items-center justify-center rounded-lg text-ink-soft hover:bg-surface-2 transition-colors"
        aria-label="关闭"
        title="关闭"
      >
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18 6 6 18M6 6l12 12"/></svg>
      </button>
    </div>

    <!-- 中：导航（滚动） -->
    <div class="flex-1 overflow-y-auto px-3 py-4 space-y-5">
      <div v-for="section in navItems" :key="section.group" class="space-y-0.5">
        <div class="px-3 mb-1.5 text-[10px] font-semibold uppercase tracking-wider text-ink-faint">{{ section.group }}</div>
        <button
          v-for="item in section.items"
          :key="item.key"
          @click="onItemClick(item)"
          class="group w-full flex items-center gap-3 px-3 min-h-[44px] rounded-lg text-sm transition-all duration-200 relative"
          :class="item.active
            ? 'bg-accent-soft text-accent'
            : 'text-ink-soft hover:bg-surface-2 hover:text-ink'"
        >
          <!-- active 指示条 -->
          <span v-if="item.active" class="absolute left-0 top-1/2 -translate-y-1/2 w-0.5 h-5 rounded-full bg-accent"></span>
          <span class="text-base w-5 text-center shrink-0">{{ item.icon }}</span>
          <span class="flex-1 text-left truncate">{{ item.label }}</span>
          <!-- 连接状态点 -->
          <span v-if="item.status === 'ok'" class="w-1.5 h-1.5 rounded-full bg-ok shadow-glow-ok"></span>
          <span v-else-if="item.status === 'warn'" class="w-1.5 h-1.5 rounded-full bg-warn"></span>
          <!-- 会话历史计数 -->
          <span v-if="item.badge" class="text-[10px] text-ink-faint tabular-nums">{{ item.badge }}</span>
        </button>
      </div>
    </div>

    <!-- 底：当前上下文 + 主题/高玩 -->
    <div class="shrink-0 border-t border-line p-3 space-y-1">
      <!-- 当前上下文摘要 -->
      <div class="px-3 py-2 rounded-lg bg-surface-2 text-xs">
        <div class="text-ink truncate font-medium">
          {{ writingMode === 'campaign' ? activeCampaignName : (writingMode === 'legacy' ? activeCharName : '未选择') }}
        </div>
        <div class="text-ink-faint mt-0.5">
          {{ writingMode === 'campaign' ? 'Campaign 模式' : (writingMode === 'legacy' ? '兼容模式' : '导入卡或开 Campaign') }}
        </div>
      </div>

      <div class="pt-1">
        <button
          @click="toggleTheme"
          class="w-full min-h-[40px] flex items-center justify-center gap-2 rounded-lg text-xs text-ink-soft hover:bg-surface-2 hover:text-ink transition-colors"
          :title="theme === 'dark' ? '切到亮色' : '切到暗色'"
        >
          <span>{{ theme === 'dark' ? '☀️' : '🌙' }}</span>
          <span>{{ theme === 'dark' ? '切到亮色' : '切到暗色' }}</span>
        </button>
      </div>
    </div>
  </nav>
</template>
