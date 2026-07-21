<script setup>
/**
 * MetaScreen — Meta 助手壳（重设计 · 纯展示）。
 * 内容区用 slots 注入既有 chat/patch/health/mvu/explain 能力组件，
 * design 层只负责纸面壳与 tab 呈现。
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
})

defineEmits(['close', 'change-tab'])
</script>

<template>
  <div class="h-full flex flex-col bg-bg">
    <header class="shrink-0 h-14 px-4 border-b border-line bg-surface flex items-center gap-3">
      <div class="min-w-0 flex-1">
        <div class="text-[11px] text-ink-faint">Meta 助手</div>
        <h1 class="text-sm font-semibold text-ink truncate">
          {{ campaignName ? `维护 · ${campaignName}` : '配置与数据健康' }}
        </h1>
      </div>
      <button
        type="button"
        class="min-h-8 w-8 rounded-md text-ink-faint hover:bg-surface-2 hover:text-ink transition-colors"
        title="关闭"
        aria-label="关闭"
        @click="$emit('close')"
      >
        <svg class="mx-auto" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18 6L6 18M6 6l12 12"/></svg>
      </button>
    </header>

    <div v-if="globalError" class="shrink-0 px-4 py-2 text-xs text-err border-b border-err/20 bg-err/10">
      {{ globalError }}
    </div>

    <nav class="shrink-0 px-3 pt-2 border-b border-line bg-surface flex items-center gap-1 overflow-x-auto">
      <button
        v-for="t in tabs"
        :key="t.key"
        type="button"
        class="min-h-10 px-3 text-[13px] border-b-2 -mb-px whitespace-nowrap transition-colors"
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

    <div class="flex-1 min-h-0 overflow-y-auto bg-bg">
      <div class="p-3 sm:p-4">
        <slot :active-tab="activeTab" />
      </div>
    </div>
  </div>
</template>
