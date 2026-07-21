<script setup>
/**
 * EmptyHero — 写作屏空态英雄区（纯展示）。
 * 事件：import / new-campaign / view-history（由接线层映射到既有 composable）。
 */
defineEmits(['import', 'new-campaign', 'view-history'])
</script>

<template>
  <section class="pt-14 pb-10 text-center">
    <div class="flex items-center justify-center gap-3 text-accent/70 mb-7" aria-hidden="true">
      <span class="h-px w-12 bg-accent-border"></span>
      <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M19.7 12.7a5.5 5.5 0 0 0-7.78-7.78L5.5 10.34V18.5h8.16z"/><path d="M15.5 8.5L3 21"/><path d="M17 14.5H9.5"/></svg>
      <span class="h-px w-12 bg-accent-border"></span>
    </div>
    <h1 class="font-semibold text-[26px] sm:text-[28px] leading-snug text-ink">开始你的故事</h1>
    <p class="mt-2.5 text-sm text-ink-soft">故事不是被发现的，而是被锻造的。</p>

    <div class="mt-9 grid sm:grid-cols-3 gap-3 text-left">
      <button
        class="group rounded-lg border border-line bg-surface p-4 shadow-card transition-all hover:border-accent-border hover:shadow-rise"
        @click="$emit('import')"
      >
        <span class="text-ink-soft group-hover:text-accent transition-colors" aria-hidden="true">
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M20 15v3.5a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V15"/><path d="M7.5 10.5L12 15l4.5-4.5"/><path d="M12 15V3.5"/></svg>
        </span>
        <div class="mt-2.5 text-sm font-medium text-ink">导入角色卡</div>
        <div class="mt-1 text-xs leading-relaxed text-ink-soft">从 PNG / JSON 卡开始一段新故事</div>
      </button>
      <button
        class="group rounded-lg border border-line bg-surface p-4 shadow-card transition-all hover:border-accent-border hover:shadow-rise"
        @click="$emit('new-campaign')"
      >
        <span class="text-ink-soft group-hover:text-accent transition-colors" aria-hidden="true">
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>
        </span>
        <div class="mt-2.5 text-sm font-medium text-ink">新建 Campaign</div>
        <div class="mt-1 text-xs leading-relaxed text-ink-soft">选卡与开场白，开启长篇连载</div>
      </button>
      <button
        class="group rounded-lg border border-line bg-surface p-4 shadow-card transition-all hover:border-accent-border hover:shadow-rise"
        @click="$emit('view-history')"
      >
        <span class="text-ink-soft group-hover:text-accent transition-colors" aria-hidden="true">
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/></svg>
        </span>
        <div class="mt-2.5 text-sm font-medium text-ink">继续上次</div>
        <div class="mt-1 text-xs leading-relaxed text-ink-soft">打开会话历史，回到未完的情节</div>
      </button>
    </div>
  </section>
</template>
