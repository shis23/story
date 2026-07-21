<script setup>
import { computed } from 'vue'

const props = defineProps({
  value: { type: Number, default: 0 },
  max: { type: Number, default: 100 },
  indeterminate: { type: Boolean, default: false },
  variant: { type: String, default: 'accent' }, // accent | ok | err
})

const variantFillClass = {
  accent: 'bg-accent',
  ok: 'bg-ok',
  err: 'bg-err',
}

const pct = computed(() => {
  if (props.max <= 0) return 0
  const ratio = props.value / props.max
  if (ratio <= 0) return 0
  if (ratio >= 1) return 100
  return ratio * 100
})

const fillClass = computed(() => variantFillClass[props.variant] || variantFillClass.accent)
</script>

<template>
  <div class="w-full bg-surface-2 rounded-full overflow-hidden h-1.5">
    <div
      v-if="indeterminate"
      :class="['h-full rounded-full progress-indeterminate', fillClass]"
    />
    <div
      v-else
      :class="['h-full rounded-full transition-all duration-200', fillClass]"
      :style="{ width: pct + '%' }"
    />
  </div>
</template>

<style scoped>
@keyframes progress-slide {
  0% { transform: translateX(-100%); width: 40%; }
  50% { width: 60%; }
  100% { transform: translateX(250%); width: 40%; }
}
.progress-indeterminate {
  animation: progress-slide 1.2s var(--ease-soft, ease-in-out) infinite;
}
</style>
