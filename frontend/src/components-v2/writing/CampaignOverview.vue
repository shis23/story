<script setup>
/**
 * CampaignOverview — Campaign 概览视图（v2 迁移）
 *
 * 迁移自 App.vue:1265-1274。读 campaignStore.activeCampaign。
 * 显示 Campaign 名/story_clock/created_at + 3 个按钮（进入面板/新建/历史）。
 * 皮肤：纸上编辑部（衬线题名、纸卡操作区）。
 */
import { useCampaignStore } from '../../stores/index.js'

const campaign = useCampaignStore()

const emit = defineEmits(['open-campaign', 'new-campaign', 'view-history'])
</script>

<template>
  <div class="mx-auto w-full max-w-[720px] px-4 sm:px-8 py-10">
    <!-- 题名区 -->
    <div class="flex items-center gap-3 text-accent/70 mb-5" aria-hidden="true">
      <span class="h-px w-10 bg-accent-border"></span>
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M2 4.5h5.5a4 4 0 0 1 4 4V20a3 3 0 0 0-3-3H2z"/><path d="M22 4.5h-5.5a4 4 0 0 0-4 4V20a3 3 0 0 1 3-3H22z"/></svg>
      <span class="h-px w-10 bg-accent-border"></span>
    </div>
    <h2 class="font-semibold text-[26px] leading-snug text-ink">
      {{ campaign.activeCampaign?.name }}
    </h2>
    <div class="mt-3 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-ink-soft">
      <span v-if="campaign.activeCampaign?.story_clock">故事时间 · {{ campaign.activeCampaign.story_clock }}</span>
      <span v-if="campaign.activeCampaign?.created_at">创建于 {{ new Date(campaign.activeCampaign.created_at).toLocaleDateString() }}</span>
      <span>底稿 {{ campaign.conversationHistory.length }} 份</span>
    </div>

    <!-- 操作区 -->
    <div class="mt-8 rounded-xl border border-line bg-surface shadow-card divide-y divide-line overflow-hidden">
      <button
        @click="emit('open-campaign')"
        class="w-full flex items-center gap-3 px-5 py-4 text-left transition-colors hover:bg-surface-2/60 group"
      >
        <span class="text-accent" aria-hidden="true">
          <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M2 4.5h5.5a4 4 0 0 1 4 4V20a3 3 0 0 0-3-3H2z"/><path d="M22 4.5h-5.5a4 4 0 0 0-4 4V20a3 3 0 0 1 3-3H22z"/></svg>
        </span>
        <span class="flex-1">
          <span class="block text-sm font-medium text-ink">Campaign 面板</span>
          <span class="block text-xs text-ink-faint mt-0.5">实例、知识、任务与摘要</span>
        </span>
        <span class="text-ink-faint group-hover:text-accent transition-colors" aria-hidden="true">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M9 6l6 6-6 6"/></svg>
        </span>
      </button>
      <button
        @click="emit('view-history')"
        class="w-full flex items-center gap-3 px-5 py-4 text-left transition-colors hover:bg-surface-2/60 group"
      >
        <span class="text-ink-soft" aria-hidden="true">
          <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/></svg>
        </span>
        <span class="flex-1">
          <span class="block text-sm font-medium text-ink">会话历史</span>
          <span class="block text-xs text-ink-faint mt-0.5">{{ campaign.conversationHistory.length }} 局底稿，随时续写</span>
        </span>
        <span class="text-ink-faint group-hover:text-accent transition-colors" aria-hidden="true">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M9 6l6 6-6 6"/></svg>
        </span>
      </button>
      <button
        @click="emit('new-campaign')"
        class="w-full flex items-center gap-3 px-5 py-4 text-left transition-colors hover:bg-surface-2/60 group"
      >
        <span class="text-ink-soft" aria-hidden="true">
          <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>
        </span>
        <span class="flex-1">
          <span class="block text-sm font-medium text-ink">新建 Campaign</span>
          <span class="block text-xs text-ink-faint mt-0.5">换一个角色，开一条新故事线</span>
        </span>
        <span class="text-ink-faint group-hover:text-accent transition-colors" aria-hidden="true">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M9 6l6 6-6 6"/></svg>
        </span>
      </button>
    </div>
  </div>
</template>
