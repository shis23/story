<script setup>
/**
 * OverviewScreen — Campaign 概览（重设计 · 纯展示）。
 * 当前活动摘要 + 三入口：管理面板 / 会话历史 / 新建。
 */
defineProps({
  campaignName: { type: String, default: '' },
  storyClock: { type: String, default: '' },
  createdAtText: { type: String, default: '' },
  conversationCount: { type: Number, default: 0 },
})

defineEmits(['open-campaign', 'new-campaign', 'view-history', 'continue-writing'])
</script>

<template>
  <div class="mx-auto w-full max-w-[760px] px-4 sm:px-8 py-10">
    <div class="flex items-center gap-3 text-accent/70 mb-5" aria-hidden="true">
      <span class="h-px w-10 bg-accent-border"></span>
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M2 4.5h5.5a4 4 0 0 1 4 4V20a3 3 0 0 0-3-3H2z"/><path d="M22 4.5h-5.5a4 4 0 0 0-4 4V20a3 3 0 0 1 3-3H22z"/></svg>
      <span class="h-px w-10 bg-accent-border"></span>
    </div>

    <h1 class="font-semibold text-[26px] leading-snug text-ink">
      {{ campaignName || '当前活动' }}
    </h1>
    <div class="mt-3 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-ink-soft">
      <span v-if="storyClock">故事时间 · {{ storyClock }}</span>
      <span v-if="createdAtText">创建于 {{ createdAtText }}</span>
      <span>底稿 {{ conversationCount }} 份</span>
    </div>

    <div class="mt-6">
      <button
        type="button"
        class="min-h-10 px-4 rounded-md bg-accent text-white text-[13px] font-medium shadow-card hover:bg-accent-bright transition-colors"
        @click="$emit('continue-writing')"
      >继续写作</button>
    </div>

    <div class="mt-8 rounded-xl border border-line bg-surface shadow-card divide-y divide-line overflow-hidden">
      <button
        type="button"
        class="w-full flex items-center gap-3 px-5 py-4 text-left transition-colors hover:bg-surface-2/60 group"
        @click="$emit('open-campaign')"
      >
        <span class="text-accent" aria-hidden="true">
          <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M2 4.5h5.5a4 4 0 0 1 4 4V20a3 3 0 0 0-3-3H2z"/><path d="M22 4.5h-5.5a4 4 0 0 0-4 4V20a3 3 0 0 1 3-3H22z"/></svg>
        </span>
        <span class="flex-1">
          <span class="block text-sm font-medium text-ink">活动管理</span>
          <span class="block text-xs text-ink-faint mt-0.5">实例、知识、任务与摘要</span>
        </span>
        <span class="text-ink-faint group-hover:text-accent transition-colors" aria-hidden="true">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M9 6l6 6-6 6"/></svg>
        </span>
      </button>
      <button
        type="button"
        class="w-full flex items-center gap-3 px-5 py-4 text-left transition-colors hover:bg-surface-2/60 group"
        @click="$emit('view-history')"
      >
        <span class="text-ink-soft" aria-hidden="true">
          <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/></svg>
        </span>
        <span class="flex-1">
          <span class="block text-sm font-medium text-ink">会话历史</span>
          <span class="block text-xs text-ink-faint mt-0.5">{{ conversationCount }} 局底稿，随时续写</span>
        </span>
        <span class="text-ink-faint group-hover:text-accent transition-colors" aria-hidden="true">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M9 6l6 6-6 6"/></svg>
        </span>
      </button>
      <button
        type="button"
        class="w-full flex items-center gap-3 px-5 py-4 text-left transition-colors hover:bg-surface-2/60 group"
        @click="$emit('new-campaign')"
      >
        <span class="text-ink-soft" aria-hidden="true">
          <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>
        </span>
        <span class="flex-1">
          <span class="block text-sm font-medium text-ink">新建活动</span>
          <span class="block text-xs text-ink-faint mt-0.5">换一个角色，开一条新故事线</span>
        </span>
        <span class="text-ink-faint group-hover:text-accent transition-colors" aria-hidden="true">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M9 6l6 6-6 6"/></svg>
        </span>
      </button>
    </div>
  </div>
</template>
