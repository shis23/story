<script setup>
import Overlay from '../ui/Overlay.vue'
import IconButton from '../ui/IconButton.vue'

// 统一弹层承载:包一层 Overlay,带标题栏 + 关闭。
// 用法:<PanelHost :show="ui.showXxx" title="标题" @close="ui.showXxx = false">内容</PanelHost>
defineProps({
  show: { type: Boolean, default: false },
  title: { type: String, default: '' },
  side: { type: String, default: 'right' }, // left|right|center|full
  /** 透传 Overlay.panelWidthClass，固定抽屉宽，避免内容撑开跳变 */
  panelWidthClass: { type: String, default: '' },
  /** 是否渲染 PanelHost 自带标题栏；false 时由子组件（如 MetaScreen）自管顶栏 */
  showChrome: { type: Boolean, default: true },
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
      <div class="flex flex-col h-full min-w-0 w-full">
        <div
          v-if="showChrome && (title || $slots.header)"
          class="shrink-0 h-14 flex items-center justify-between px-4 border-b border-line glass"
        >
          <slot name="header">
            <h2 class="text-sm font-semibold text-ink truncate">{{ title }}</h2>
          </slot>
          <IconButton title="关闭" @click="close">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18 6L6 18M6 6l12 12"/></svg>
          </IconButton>
        </div>
        <!-- min-h-0 防止 flex 子项把外壳撑宽；overflow 由子屏自管 -->
        <div class="flex-1 min-h-0 min-w-0 overflow-hidden flex flex-col">
          <slot :close="close" />
        </div>
        <div v-if="$slots.footer" class="shrink-0 border-t border-line p-3">
          <slot name="footer" :close="close" />
        </div>
      </div>
    </template>
  </Overlay>
</template>
