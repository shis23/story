<script setup>
import Overlay from '../ui/Overlay.vue'
import IconButton from '../ui/IconButton.vue'

// 统一弹层承载:包一层 Overlay,带标题栏 + 关闭。
// 用法:<PanelHost :show="ui.showXxx" title="标题" @close="ui.showXxx = false">内容</PanelHost>
defineProps({
  show: { type: Boolean, default: false },
  title: { type: String, default: '' },
  side: { type: String, default: 'right' }, // left|right|center|full
})
const emit = defineEmits(['close'])
</script>

<template>
  <Overlay
    :show="show"
    :side="side"
    @update:show="emit('close')"
  >
    <template #default="{ close }">
      <div class="flex flex-col h-full">
        <div
          v-if="title || $slots.header"
          class="shrink-0 h-14 flex items-center justify-between px-4 border-b border-line glass"
        >
          <slot name="header">
            <h2 class="text-sm font-semibold text-ink truncate">{{ title }}</h2>
          </slot>
          <IconButton title="关闭" @click="close">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18 6L6 18M6 6l12 12"/></svg>
          </IconButton>
        </div>
        <div class="flex-1 overflow-y-auto">
          <slot :close="close" />
        </div>
        <div v-if="$slots.footer" class="shrink-0 border-t border-line p-3">
          <slot name="footer" :close="close" />
        </div>
      </div>
    </template>
  </Overlay>
</template>
