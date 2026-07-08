<script setup>
import { onMounted, onBeforeUnmount } from 'vue'

const props = defineProps({
  variant: { type: String, default: 'neutral' }, // ok | warn | err | neutral (accent)
  message: { type: String, default: '' },
  duration: { type: Number, default: 0 }, // 0 = 不自动关闭
})

const emit = defineEmits(['close'])

const accentClass = {
  ok: 'bg-ok',
  warn: 'bg-warn',
  err: 'bg-err',
  neutral: 'bg-accent',
}

let timer = null

onMounted(() => {
  if (props.duration > 0) {
    timer = setTimeout(() => emit('close'), props.duration)
  }
})

onBeforeUnmount(() => {
  if (timer) clearTimeout(timer)
})
</script>

<template>
  <div
    class="glass-strong shadow-float rounded-lg px-4 py-3 flex items-start gap-3 relative overflow-hidden"
  >
    <!-- 左侧彩色竖线 -->
    <span
      :class="['absolute left-0 top-0 bottom-0 w-1', accentClass[props.variant] || accentClass.neutral]"
    />
    <div class="flex-1 min-w-0 pl-1">
      <p v-if="message" class="text-ink text-sm">{{ message }}</p>
      <slot />
    </div>
    <div v-if="$slots.action" class="shrink-0">
      <slot name="action" />
    </div>
    <button
      type="button"
      class="shrink-0 text-ink-faint hover:text-ink transition-colors"
      @click="emit('close')"
    >
      ✕
    </button>
  </div>
</template>
