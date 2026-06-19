<script setup>
import { ref } from 'vue'
import { sampleIntent } from '../mock.js'

const props = defineProps({
  /** 写作进行中：发送键变停止键，输入禁用 */
  writing: { type: Boolean, default: false },
  /** 不可写作（无角色/Campaign）：输入禁用，但不显示停止键 */
  disabled: { type: Boolean, default: false },
  placeholder: { type: String, default: '' },
})

const emit = defineEmits(['start-writing', 'cancel'])

const intent = ref('')

function submit() {
  if (!intent.value.trim() || props.disabled || props.writing) return
  emit('start-writing', intent.value)
  intent.value = ''
}
</script>

<template>
  <div class="shrink-0 glass border-t border-line">
    <!-- 单框输入区（不再大框套小框） -->
    <div class="flex items-end gap-2 px-3 py-2.5 transition-colors"
      :class="writing ? '' : 'focus-within:border-accent'">
      <textarea
        v-model="intent"
        :placeholder="placeholder || (writing ? '写作中…' : sampleIntent)"
        :disabled="disabled || writing"
        rows="1"
        class="flex-1 bg-transparent resize-none outline-none text-[15px] text-ink placeholder:text-ink-soft/50 max-h-32 disabled:opacity-50"
        @keydown.enter.exact.prevent="submit"
        @input="$event.target.style.height='auto'; $event.target.style.height=$event.target.scrollHeight+'px'"
      ></textarea>

      <!-- 发送键 / 停止键（同一槽位） -->
      <button
        v-if="!writing"
        @click="submit"
        :disabled="!intent.trim() || disabled"
        class="shrink-0 w-11 h-11 rounded-full flex items-center justify-center transition-all"
        :class="intent.trim() && !disabled
          ? 'bg-accent text-white shadow-glow-accent hover:opacity-90'
          : 'bg-surface-2 text-ink-faint'"
        aria-label="发送"
      >
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M5 12h14M13 6l6 6-6 6"/>
        </svg>
      </button>
      <button
        v-else
        @click="$emit('cancel')"
        class="shrink-0 w-11 h-11 rounded-full flex items-center justify-center bg-err text-white shadow-glow-err hover:opacity-90 transition-all animate-pulse"
        aria-label="停止生成"
        title="停止生成"
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor"><rect x="5" y="5" width="14" height="14" rx="2"/></svg>
      </button>
    </div>
    <div class="text-[11px] text-ink-soft/60 px-3 pb-2">
      <template v-if="writing">写作中 · 点停止键中断</template>
      <template v-else>描述你要写的场景或意图 · 回车发送 · Shift+Enter 换行</template>
    </div>
  </div>
</template>
