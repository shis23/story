<script setup>
/**
 * MetaScreen — Meta 助手壳（重设计 · 纯展示）。
 *
 * fillContent=true（对话）：内容区填满剩余高度，子组件自管（消息滚 + 输入贴底）。
 * fillContent=false（其它 tab）：内容区可滚，底部留白。
 */
defineProps({
  activeTab: { type: String, default: 'chat' },
  tabs: {
    type: Array,
    default: () => [
      { key: 'chat', label: '对话' },
      { key: 'patches', label: 'Patch' },
      { key: 'health', label: '健康检查' },
      { key: 'mvu', label: 'MVU' },
      { key: 'explain', label: '生成解释' },
    ],
  },
  pendingPatchCount: { type: Number, default: 0 },
  globalError: { type: String, default: '' },
  campaignName: { type: String, default: '' },
  /** 对话等需要「输入贴底」的屏传 true */
  fillContent: { type: Boolean, default: false },
})

defineEmits(['close', 'change-tab'])
</script>

<template>
  <div class="h-full w-full min-w-0 flex flex-col bg-bg">
    <header class="sf-toolbar px-4 border-b border-line bg-surface flex items-center gap-3">
      <div class="min-w-0 flex-1">
        <div class="text-[11px] text-ink-faint">Meta 助手</div>
        <h1 class="text-sm font-semibold text-ink truncate">
          {{ campaignName ? `维护 · ${campaignName}` : '配置与数据健康' }}
        </h1>
      </div>
      <button
        type="button"
        class="sf-toolbar-icon transition-colors"
        title="关闭"
        aria-label="关闭"
        @click="$emit('close')"
      >
        <svg class="mx-auto" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18 6L6 18M6 6l12 12"/></svg>
      </button>
    </header>

    <div
      v-if="globalError"
      class="shrink-0 px-4 py-2 text-xs text-err border-b border-err/20 bg-err/10 break-words"
    >
      {{ globalError }}
    </div>

    <nav class="shrink-0 px-2 pt-1 border-b border-line bg-surface flex items-center gap-0.5 overflow-x-auto min-w-0">
      <button
        v-for="t in tabs"
        :key="t.key"
        type="button"
        class="min-h-10 px-2.5 text-[13px] border-b-2 -mb-px whitespace-nowrap transition-colors shrink-0"
        :class="activeTab === t.key
          ? 'border-accent text-accent-bright font-medium'
          : 'border-transparent text-ink-soft hover:text-ink'"
        @click="$emit('change-tab', t.key)"
      >
        {{ t.label }}
        <span
          v-if="t.key === 'patches' && pendingPatchCount > 0"
          class="ml-1 inline-flex min-w-[18px] h-[18px] px-1 items-center justify-center rounded-full bg-warn/15 text-warn text-[10px]"
        >{{ pendingPatchCount }}</span>
      </button>
    </nav>

    <!-- 对话：填满高度，输入条由 MetaChat 贴底 -->
    <div
      v-if="fillContent"
      class="flex-1 min-h-0 min-w-0 flex flex-col overflow-hidden"
    >
      <slot :active-tab="activeTab" />
    </div>
    <!-- 其它 tab：可滚动 + 底部留白 -->
    <div
      v-else
      class="sf-drawer-scroll flex-1 min-h-0 min-w-0 overflow-y-auto overscroll-y-contain bg-bg"
    >
      <div class="w-full min-w-0 p-4 pb-12">
        <slot :active-tab="activeTab" />
      </div>
    </div>
  </div>
</template>
