<script setup>
/**
 * ConversationHistoryList — 会话历史列表（v2 迁移）
 *
 * 迁移自 App.vue:1277-1304。读 campaignStore.conversationHistory。
 * 每个会话卡片显示 card_name/message_count/created_at。
 * 皮肤：纸上编辑部（衬线标题、纸卡列表、细线分隔）。
 */
import { useCampaignStore } from '../../stores/index.js'
import EmptyState from '../ui/EmptyState.vue'

const campaign = useCampaignStore()

const emit = defineEmits(['open', 'delete', 'new-campaign'])
</script>

<template>
  <div class="mx-auto w-full max-w-[720px] px-4 sm:px-8 py-8">
    <div class="flex items-end justify-between mb-6">
      <div>
        <h2 class="font-semibold text-[22px] text-ink leading-tight">会话历史</h2>
        <p v-if="campaign.conversationHistory.length" class="mt-1 text-xs text-ink-faint">
          共 {{ campaign.conversationHistory.length }} 局底稿
        </p>
      </div>
      <!-- 头部新建入口：仅列表非空且移动端出现。
           空态由英雄区 CTA 承担；桌面由常驻侧栏主按钮承担，避免重复。 -->
      <button
        v-if="campaign.conversationHistory.length"
        @click="emit('new-campaign')"
        class="lg:hidden min-h-9 px-3.5 rounded-md bg-accent text-white text-[13px] font-medium shadow-card hover:bg-accent-bright transition-colors flex items-center gap-1.5"
      >
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>
        新建 Campaign
      </button>
    </div>

    <EmptyState
      v-if="campaign.conversationHistory.length === 0"
      title="还没有写下第一笔"
      description="每一局故事都会在这里留下底稿，随时回到未完的情节。"
    >
      <template #icon>
        <svg width="30" height="30" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/></svg>
      </template>
      <template #action>
        <button
          @click="emit('new-campaign')"
          class="min-h-9 px-4 rounded-md bg-accent text-white text-[13px] font-medium shadow-card hover:bg-accent-bright transition-colors"
        >开始第一局故事</button>
      </template>
    </EmptyState>

    <div v-else class="rounded-xl border border-line bg-surface shadow-card overflow-hidden divide-y divide-line sf-stagger">
      <div
        v-for="(conv, i) in campaign.conversationHistory"
        :key="conv.id"
        :style="{ '--i': i }"
        @click="emit('open', conv)"
        class="group px-4 sm:px-5 py-4 cursor-pointer transition-colors hover:bg-surface-2/60 flex items-center gap-3"
      >
        <div class="flex-1 min-w-0">
          <div class="text-[15px] text-ink font-medium truncate">{{ conv.card_name || '未知角色卡' }}</div>
          <div class="text-xs text-ink-faint mt-1">
            {{ conv.message_count || 0 }} 条消息 · 创建于 {{ new Date(conv.created_at).toLocaleString() }}
          </div>
        </div>
        <button
          @click="emit('delete', conv, $event)"
          class="shrink-0 w-8 h-8 flex items-center justify-center rounded-md text-ink-faint hover:text-err hover:bg-err/10 transition-colors"
          title="删除会话"
          aria-label="删除会话"
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
        </button>
        <span class="shrink-0 text-ink-faint opacity-0 group-hover:opacity-100 transition-opacity" aria-hidden="true">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M9 6l6 6-6 6"/></svg>
        </span>
      </div>
    </div>
  </div>
</template>
