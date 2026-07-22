<script setup>
import Overlay from '../ui/Overlay.vue'
import IconButton from '../ui/IconButton.vue'

// 统一弹层承载：Overlay + 标题栏 + 内容区。
// bodyScroll=true（默认）：本组件滚动井滚动，底部 pb-12 留白 + 弹性 overscroll。
// bodyScroll=false：填满高度，由子组件自管滚动（MetaScreen / CampaignScreen 等）。
defineProps({
  show: { type: Boolean, default: false },
  title: { type: String, default: '' },
  side: { type: String, default: 'right' }, // left|right|center|full
  panelWidthClass: { type: String, default: '' },
  showChrome: { type: Boolean, default: true },
  /** 是否由 PanelHost 内容井滚动；false 时子组件自管（需 h-full + 内部 overflow） */
  bodyScroll: { type: Boolean, default: true },
})
const emit = defineEmits(['close'])
</script>

<template>
  <Overlay
    :show="show"
    :side="side"
    :show-close="false"
    :panel-width-class="panelWidthClass"
    @update:show="emit('close')"
  >
    <template #default="{ close }">
      <div class="flex flex-col h-full min-h-0 min-w-0 w-full">
        <div
          v-if="showChrome && (title || $slots.header)"
          class="shrink-0 h-14 flex items-center justify-between px-4 border-b border-line bg-surface"
        >
          <slot name="header">
            <h2 class="text-sm font-semibold text-ink truncate">{{ title }}</h2>
          </slot>
          <IconButton title="关闭" @click="close">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18 6L6 18M6 6l12 12"/></svg>
          </IconButton>
        </div>

        <!-- 默认：可滚动 + 底部留白 + 弹性 -->
        <div
          v-if="bodyScroll"
          class="sf-drawer-scroll flex-1 min-h-0 min-w-0 overflow-y-auto overscroll-y-contain"
        >
          <div class="min-w-0 pb-12">
            <slot :close="close" />
          </div>
        </div>
        <!-- 自管滚动：填满剩余高度 -->
        <div
          v-else
          class="flex-1 min-h-0 min-w-0 overflow-hidden flex flex-col"
        >
          <slot :close="close" />
        </div>

        <div v-if="$slots.footer" class="shrink-0 border-t border-line p-3 bg-surface">
          <slot name="footer" :close="close" />
        </div>
      </div>
    </template>
  </Overlay>
</template>
