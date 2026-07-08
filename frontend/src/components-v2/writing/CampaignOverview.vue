<script setup>
/**
 * CampaignOverview — Campaign 概览视图（v2 迁移）
 *
 * 迁移自 App.vue:1265-1274。读 campaignStore.activeCampaign。
 * 显示 Campaign 名/story_clock/created_at + 3 个按钮（进入面板/新建/历史）。
 */
import { useCampaignStore } from '../../stores/index.js'

const campaign = useCampaignStore()

const emit = defineEmits(['open-campaign', 'new-campaign', 'view-history'])
</script>

<template>
  <div class="mx-auto max-w-2xl px-4 py-6">
    <h2 class="text-xl font-bold text-ink mb-1">📜 {{ campaign.activeCampaign?.name }}</h2>
    <p v-if="campaign.activeCampaign?.story_clock" class="text-xs text-ink-soft mb-1">
      故事时间：{{ campaign.activeCampaign.story_clock }}
    </p>
    <p v-if="campaign.activeCampaign?.created_at" class="text-xs text-ink-soft mb-5">
      创建于 {{ new Date(campaign.activeCampaign.created_at).toLocaleDateString() }}
    </p>
    <div class="flex flex-wrap gap-2 mb-6">
      <button
        @click="emit('open-campaign')"
        class="min-h-[44px] px-4 rounded-lg bg-accent text-white text-sm font-medium shadow-glow-accent hover:opacity-90 transition-opacity"
      >进入 Campaign 面板</button>
      <button
        @click="emit('new-campaign')"
        class="min-h-[44px] px-4 rounded-lg bg-surface-2 text-ink text-sm hover:bg-line transition-colors"
      >✚ 新建 Campaign</button>
      <button
        @click="emit('view-history')"
        class="min-h-[44px] px-4 rounded-lg bg-surface-2 text-ink-soft text-sm hover:bg-line transition-colors"
      >📋 会话历史 ({{ campaign.conversationHistory.length }})</button>
    </div>
  </div>
</template>
