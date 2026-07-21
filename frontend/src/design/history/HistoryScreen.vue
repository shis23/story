<script setup>
/**
 * HistoryScreen — 会话历史（重设计 · 纯展示）。
 * 对齐 selected 图纸质列表；不 import store/tauri。
 */
defineProps({
  conversations: { type: Array, default: () => [] },
  title: { type: String, default: '会话历史' },
})

defineEmits(['open', 'delete', 'new-campaign'])

function formatDate(value) {
  if (!value) return ''
  try {
    return new Date(value).toLocaleString()
  } catch {
    return String(value)
  }
}
</script>

<template>
  <div class="mx-auto w-full max-w-[760px] px-4 sm:px-8 py-8">
    <header class="flex items-end justify-between mb-6">
      <div>
        <h1 class="font-semibold text-[22px] text-ink leading-tight">{{ title }}</h1>
        <p v-if="conversations.length" class="mt-1 text-xs text-ink-faint">
          共 {{ conversations.length }} 局底稿
        </p>
      </div>
      <button
        v-if="conversations.length"
        type="button"
        class="lg:hidden min-h-9 px-3.5 rounded-md bg-accent text-white text-[13px] font-medium shadow-card hover:bg-accent-bright transition-colors flex items-center gap-1.5"
        @click="$emit('new-campaign')"
      >
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>
        新建 Campaign
      </button>
    </header>

    <section v-if="!conversations.length" class="rounded-xl border border-line bg-surface shadow-card px-6 py-14 text-center">
      <div class="flex justify-center text-ink-faint mb-4" aria-hidden="true">
        <svg width="30" height="30" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/></svg>
      </div>
      <h2 class="font-semibold text-[18px] text-ink">还没有写下第一笔</h2>
      <p class="mt-2 text-sm text-ink-soft">每一局故事都会在这里留下底稿，随时回到未完的情节。</p>
      <button
        type="button"
        class="mt-6 min-h-9 px-4 rounded-md bg-accent text-white text-[13px] font-medium shadow-card hover:bg-accent-bright transition-colors"
        @click="$emit('new-campaign')"
      >开始第一局故事</button>
    </section>

    <div v-else class="rounded-xl border border-line bg-surface shadow-card overflow-hidden divide-y divide-line">
      <div
        v-for="conv in conversations"
        :key="conv.id"
        class="group px-4 sm:px-5 py-4 cursor-pointer transition-colors hover:bg-surface-2/60 flex items-center gap-3"
        @click="$emit('open', conv)"
      >
        <div class="flex-1 min-w-0">
          <div class="text-[15px] text-ink font-medium truncate">{{ conv.card_name || conv.name || '未知角色卡' }}</div>
          <div class="text-xs text-ink-faint mt-1">
            {{ conv.message_count || 0 }} 条消息
            <template v-if="conv.created_at"> · 创建于 {{ formatDate(conv.created_at) }}</template>
          </div>
        </div>
        <button
          type="button"
          class="shrink-0 w-8 h-8 flex items-center justify-center rounded-md text-ink-faint hover:text-err hover:bg-err/10 transition-colors"
          title="删除会话"
          aria-label="删除会话"
          @click.stop="$emit('delete', conv, $event)"
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
