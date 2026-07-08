<script setup>
/**
 * ConversationHistoryList — 会话历史列表（v2 迁移）
 *
 * 迁移自 App.vue:1277-1304。读 campaignStore.conversationHistory。
 * 空态文案"暂无会话。点「新建 Campaign」开始第一局故事。"。
 * 每个会话卡片显示 card_name/message_count/created_at。
 */
import { useCampaignStore } from '../../stores/index.js'

const campaign = useCampaignStore()

const emit = defineEmits(['open', 'delete', 'new-campaign'])
</script>

<template>
  <div class="mx-auto max-w-2xl px-4 py-6">
    <div class="flex items-center justify-between mb-4">
      <h2 class="text-xl font-bold text-ink">会话历史</h2>
      <button
        @click="emit('new-campaign')"
        class="min-h-[44px] px-4 rounded-lg bg-accent text-white text-sm shadow-glow-accent hover:opacity-90 transition-opacity"
      >✚ 新建 Campaign</button>
    </div>
    <div v-if="campaign.conversationHistory.length === 0" class="text-center text-ink-soft py-16 text-sm">
      暂无会话。点「新建 Campaign」开始第一局故事。
    </div>
    <div v-else class="space-y-2 sf-stagger">
      <div
        v-for="(conv, i) in campaign.conversationHistory"
        :key="conv.id"
        :style="{ '--i': i }"
        @click="emit('open', conv)"
        class="p-3.5 rounded-xl bg-surface shadow-card hover:shadow-rise hover:-translate-y-px cursor-pointer transition-all duration-200 flex items-center gap-2"
      >
        <div class="flex-1 min-w-0">
          <div class="text-sm text-ink font-medium truncate">{{ conv.card_name || '未知角色卡' }}</div>
          <div class="text-xs text-ink-soft mt-1">
            {{ conv.message_count || 0 }} 条消息 · 创建于 {{ new Date(conv.created_at).toLocaleString() }}
          </div>
        </div>
        <button
          @click="emit('delete', conv, $event)"
          class="shrink-0 w-9 h-9 flex items-center justify-center rounded-lg text-ink-faint hover:text-err hover:bg-err/10 transition-colors"
          title="删除会话"
          aria-label="删除会话"
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
        </button>
      </div>
    </div>
  </div>
</template>
