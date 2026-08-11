<script setup>
import { computed } from 'vue'
import { useUiStore, useWritingStore, useCampaignStore } from '../../stores/index.js'

const ui = useUiStore()
const writing = useWritingStore()
const campaign = useCampaignStore()

// 副标题(对应 App.vue:1233-1238 的状态文本)
const subtitle = computed(() => {
  if (writing.isWriting) return '写作中…'
  if (writing.writingMode === 'campaign')
    return `Campaign · ${campaign.activeCampaign?.story_clock || '第 1 轮'}`
  if (writing.writingMode === 'legacy') return '兼容模式'
  return '导入角色卡或打开 Campaign'
})
</script>

<template>
  <header
    class="story-topbar relative isolate grid h-[calc(52px+env(safe-area-inset-top))] pt-[env(safe-area-inset-top)] shrink-0 grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-2 border-b border-line px-3 sm:px-4"
  >
    <!-- 左侧留白槽：窄窗显示菜单，桌面仍为标题保留对称空间。 -->
    <div class="col-start-1 flex min-w-0 items-center justify-start">
      <button
        class="story-topbar-icon lg:hidden"
        @click="ui.showSidebar = true"
        aria-label="菜单"
        title="打开菜单"
      >
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
          <path d="M5 7.5h14M5 12h14M5 16.5h14"/>
        </svg>
      </button>
    </div>

    <!-- 中轴固定，不再受左右操作区宽度影响。 -->
    <div
      class="story-title-rail min-w-0 text-center"
      data-topbar-slot="title"
    >
      <div class="flex min-w-0 items-center justify-center gap-2">
        <span class="truncate text-[15px] font-semibold leading-tight tracking-[-0.01em] text-ink">
          {{ ui.pageTitle }}
        </span>
        <span
          v-if="writing.isWriting"
          class="inline-flex shrink-0 items-center gap-1 rounded-full border border-running/25 bg-running/10 px-1.5 py-0.5"
          aria-label="生成中"
        >
          <span class="h-1.5 w-1.5 rounded-full bg-running animate-pulse"></span>
          <span class="hidden text-[9px] font-semibold tracking-wide text-running xs:inline">生成中</span>
        </span>
      </div>
      <div class="mt-0.5 truncate text-[10px] font-medium leading-tight tracking-[0.08em] text-ink-faint">
        {{ subtitle }}
      </div>
    </div>

    <div class="col-start-3 flex min-w-0 items-center justify-end gap-1.5">
      <div
        v-if="writing.writingMode === 'campaign' && campaign.activeCampaign"
        class="story-state-dock"
        aria-label="故事状态"
      >
        <button
          type="button"
          class="story-state-button"
          aria-label="查看总结"
          title="查看本局总结"
          @click="ui.openCampaignPanel('summaries')"
        >
          <svg aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
            <path d="M6.5 4.5h8l3 3v12h-11z"/>
            <path d="M14.5 4.5v3h3M9 11h6M9 14.5h6"/>
          </svg>
          <span class="hidden xs:inline">总结</span>
        </button>
        <span class="h-3.5 w-px bg-line" aria-hidden="true"></span>
        <button
          type="button"
          class="story-state-button"
          aria-label="查看变量"
          title="查看本局与角色变量"
          @click="ui.openCampaignPanel('variables')"
        >
          <svg aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round">
            <path d="M5 7h14M5 12h14M5 17h14"/>
            <circle cx="9" cy="7" r="1.75" fill="var(--color-surface)"/>
            <circle cx="15" cy="12" r="1.75" fill="var(--color-surface)"/>
            <circle cx="11" cy="17" r="1.75" fill="var(--color-surface)"/>
          </svg>
          <span class="hidden xs:inline">变量</span>
        </button>
      </div>

      <span
        v-if="writing.writingMode === 'campaign' && campaign.activeCampaign"
        class="hidden h-5 w-px bg-line/80 xs:block"
        aria-hidden="true"
      ></span>

      <!-- 调试入口保持低对比，但和左侧菜单使用同一按钮尺寸。 -->
      <button
        class="story-topbar-icon text-ink-faint"
        @click="ui.showDebugDrawer = true"
        aria-label="过程与调试"
        title="过程与调试"
      >
        <svg aria-hidden="true" width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.65" stroke-linecap="round" stroke-linejoin="round">
          <path d="M5 5.5h14v13H5z"/>
          <path d="M14.5 5.5v13M17 9h-1M17 12h-1M17 15h-1"/>
        </svg>
      </button>
    </div>
  </header>
</template>

<style scoped>
.story-topbar {
  position: relative;
  background:
    linear-gradient(
      90deg,
      color-mix(in srgb, var(--color-accent) 3.5%, transparent),
      transparent 28%,
      transparent 72%,
      color-mix(in srgb, var(--color-accent) 2.5%, transparent)
    ),
    color-mix(in srgb, var(--color-surface) 68%, var(--color-bg));
  box-shadow:
    inset 0 1px color-mix(in srgb, var(--color-surface) 82%, transparent),
    0 1px 0 color-mix(in srgb, var(--color-ink) 3%, transparent);
}

.story-title-rail {
  position: absolute;
  top: 50%;
  left: 50%;
  width: max-content;
  max-width: 30vw;
  transform: translate(-50%, -50%);
  pointer-events: none;
}

.story-topbar-icon {
  display: inline-flex;
  width: 2.75rem;
  height: 2.75rem;
  align-items: center;
  justify-content: center;
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  color: var(--color-ink-soft);
  transition:
    color 150ms ease,
    border-color 150ms ease,
    background-color 150ms ease;
}

.story-topbar-icon:hover {
  border-color: var(--color-line);
  background: color-mix(in srgb, var(--color-surface) 72%, transparent);
  color: var(--color-ink);
}

.story-state-dock {
  display: inline-flex;
  flex-shrink: 0;
  align-items: center;
  padding: 0.125rem;
  border: 1px solid color-mix(in srgb, var(--color-line) 86%, transparent);
  border-radius: var(--radius-lg);
  background: color-mix(in srgb, var(--color-surface) 74%, transparent);
  box-shadow: var(--shadow-card);
}

.story-state-button {
  display: inline-flex;
  min-height: 1.75rem;
  align-items: center;
  justify-content: center;
  gap: 0.35rem;
  padding-inline: 0.55rem;
  border-radius: var(--radius-md);
  color: var(--color-ink-soft);
  font-size: 0.75rem;
  transition:
    color 150ms ease,
    background-color 150ms ease;
}

.story-state-button:hover {
  background: var(--color-accent-soft);
  color: var(--color-accent-bright);
}

@media (max-width: 29.99rem) {
  .story-state-button {
    width: 1.75rem;
    padding-inline: 0;
  }
}

@media (min-width: 40rem) {
  .story-title-rail {
    max-width: 42vw;
  }
}
</style>
