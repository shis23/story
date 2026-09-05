<script setup>
import { computed } from 'vue'
import { Menu, NotebookText, PanelLeftClose, PanelLeftOpen, PanelRight, SlidersHorizontal } from '@lucide/vue'
import { useUiStore, useWritingStore, useCampaignStore } from '../../stores/index.js'

const ui = useUiStore()
const writing = useWritingStore()
const campaign = useCampaignStore()
const props = defineProps({
  sidebarDocked: { type: Boolean, default: false },
  sidebarVisible: { type: Boolean, default: false },
})
const emit = defineEmits(['toggle-sidebar'])
const sidebarLabel = computed(() => props.sidebarDocked
  ? (props.sidebarVisible ? '收起侧栏' : '展开侧栏')
  : '菜单')
const hasCampaign = computed(() => writing.writingMode === 'campaign' && campaign.activeCampaign)
const subtitle = computed(() => {
  if (writing.isWriting) return '写作中…'
  if (writing.writingMode === 'campaign')
    return `Campaign · ${campaign.activeCampaign?.story_clock || '第 1 轮'}`
  if (writing.writingMode === 'legacy') return '兼容模式'
  return 'StoryForge'
})
</script>

<template>
  <header class="story-topbar sf-toolbar">
    <button
      type="button"
      class="story-topbar-icon sf-toolbar-icon story-menu"
      :aria-label="sidebarLabel"
      :aria-expanded="sidebarVisible"
      :title="sidebarDocked ? sidebarLabel : '打开菜单'"
      @click="emit('toggle-sidebar')"
    >
      <component :is="sidebarDocked ? (sidebarVisible ? PanelLeftClose : PanelLeftOpen) : Menu" :size="19" aria-hidden="true" />
    </button>

    <div class="story-title-rail" data-topbar-slot="title">
      <div class="truncate text-[15px] font-semibold leading-5 text-ink" :title="ui.pageTitle">
        {{ ui.pageTitle }}
      </div>
      <div
        role="status"
        class="truncate text-[11px] leading-4"
        :class="writing.isWriting ? 'text-running' : 'text-ink-soft'"
      >{{ subtitle }}</div>
    </div>

    <div class="story-tools">
      <div v-if="hasCampaign" class="story-state-dock" role="group" aria-label="故事状态">
        <button
          type="button"
          class="story-topbar-icon sf-toolbar-icon"
          aria-label="查看总结"
          title="查看本局总结"
          @click="ui.openCampaignPanel('summaries')"
        >
          <NotebookText :size="18" aria-hidden="true" />
        </button>
        <button
          type="button"
          class="story-topbar-icon sf-toolbar-icon"
          aria-label="查看变量"
          title="查看本局与角色变量"
          @click="ui.openCampaignPanel('variables')"
        >
          <SlidersHorizontal :size="18" aria-hidden="true" />
        </button>
      </div>
      <span v-if="hasCampaign" class="h-5 w-px bg-line" aria-hidden="true"></span>
      <button
        type="button"
        class="story-topbar-icon sf-toolbar-icon"
        aria-label="过程与调试"
        title="过程与调试"
        @click="ui.showDebugDrawer = true"
      >
        <PanelRight :size="18" aria-hidden="true" />
      </button>
    </div>
    <div v-if="writing.isWriting" class="story-activity" data-testid="writing-activity" aria-hidden="true"></div>
  </header>
</template>

<style scoped>
.story-topbar {
  position: relative;
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: center;
  gap: 8px;
  padding-inline: 12px;
  border-bottom: 1px solid var(--color-line);
  background: var(--color-surface);
}

.story-title-rail {
  min-width: 0;
  overflow: hidden;
}

.story-tools,
.story-state-dock {
  display: flex;
  flex-shrink: 0;
  align-items: center;
}

.story-tools {
  gap: 4px;
}

.story-activity {
  position: absolute;
  bottom: -1px;
  inset-inline: 0;
  height: 2px;
  overflow: hidden;
  background: color-mix(in srgb, var(--color-running) 10%, transparent);
}

.story-activity::after {
  content: '';
  display: block;
  width: 30%;
  height: 100%;
  background: var(--color-running);
  animation: sf-writing-sweep 1.8s var(--ease-soft) infinite;
}

@media (max-width: 22.49rem) {
  .story-topbar {
    gap: 4px;
    padding-inline: 8px;
  }
}

@media (min-width: 64rem) {
  .story-topbar {
    padding-inline: 24px;
  }
}

@media (prefers-reduced-motion: reduce) {
  .story-activity::after {
    width: 100%;
    animation: none;
  }
}
</style>
