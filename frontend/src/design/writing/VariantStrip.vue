<script setup>
/**
 * VariantStrip — 并排变体卡（重设计：替代旧的 ‹ 1/3 › 切换器）。
 *
 * 纯展示组件。点击卡片 = 切换当前版本（switch-variant）；
 * 当前版本卡内提供「采纳此版」（accept-variant）。
 * 事件名与 payload 对齐现有 useMessageVariants 契约，接线零适配。
 */
import { computed } from 'vue'

const props = defineProps({
  /** 完整 message 对象（取 variants / active_variant / id） */
  message: { type: Object, required: true },
  busy: { type: Boolean, default: false },
})

const emit = defineEmits(['switch-variant', 'accept-variant'])

const variants = computed(() => props.message.variants || [])
const activeIndex = computed(() => props.message.active_variant)

function previewOf(v) {
  const text = (v?.display_content ?? v?.content ?? '').trim()
  return text.length > 90 ? text.slice(0, 90) + '…' : text
}

function wordsOf(v) {
  return ((v?.display_content ?? v?.content ?? '').replace(/\s/g, '') || '').length
}

function pick(index) {
  if (index === activeIndex.value || props.busy) return
  emit('switch-variant', { messageId: props.message.id, index })
}

function accept() {
  emit('accept-variant', { nodeId: props.message.id })
}
</script>

<template>
  <div class="mt-4">
    <div class="flex items-center gap-2 mb-2">
      <span class="text-[11px] tracking-wide text-ink-faint">共 {{ variants.length }} 版</span>
      <span class="h-px flex-1 bg-line"></span>
    </div>

    <div class="grid gap-2.5 sm:grid-cols-3">
      <button
        v-for="(v, i) in variants"
        :key="i"
        @click="pick(i)"
        :disabled="busy"
        class="relative rounded-lg border p-3.5 text-left transition-all disabled:opacity-50"
        :class="i === activeIndex
          ? 'border-accent-border bg-accent-soft/40 shadow-card'
          : 'border-line bg-surface hover:border-accent-border hover:shadow-card'"
      >
        <div class="flex items-center gap-1.5 mb-1.5">
          <span
            class="text-xs font-medium"
            :class="i === activeIndex ? 'text-accent-bright' : 'text-ink-soft'"
          >变体 {{ String.fromCharCode(65 + i) }}</span>
          <span v-if="v.status === 'final'" class="text-[10px] text-ok">已采纳</span>
          <span v-else-if="v.status === 'discarded'" class="text-[10px] text-ink-faint">旧版</span>
          <!-- 选中圈（对齐选定图 radio） -->
          <span
            class="ml-auto w-3.5 h-3.5 rounded-full border flex items-center justify-center"
            :class="i === activeIndex ? 'border-accent' : 'border-line'"
            aria-hidden="true"
          >
            <span v-if="i === activeIndex" class="w-1.5 h-1.5 rounded-full bg-accent"></span>
          </span>
        </div>
        <p
          class="text-xs leading-relaxed line-clamp-3 min-h-[3.75rem]"
          :class="v.status === 'discarded' ? 'text-ink-faint' : 'text-ink-soft'"
        >{{ previewOf(v) }}</p>
        <div class="mt-2 text-[10px] text-ink-faint tabular-nums">{{ wordsOf(v).toLocaleString() }} 字</div>
      </button>
    </div>

    <!-- 当前版本未采纳时给主行动 -->
    <div v-if="variants[activeIndex] && variants[activeIndex].status !== 'final'" class="mt-2 flex justify-end">
      <button
        @click="accept"
        :disabled="busy"
        class="min-h-8 px-3 rounded-md text-xs font-medium text-accent-bright border border-accent-border hover:bg-accent-soft transition-colors disabled:opacity-40"
      >采纳此版</button>
    </div>
  </div>
</template>
