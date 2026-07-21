<script setup>
import { ref } from 'vue'

const sampleIntent = '写一段紧张的追逐戏，让侦探在雨夜的巷子里追上嫌疑人'

const props = defineProps({
  /** 写作进行中:发送键变停止键,输入禁用 */
  writing: { type: Boolean, default: false },
  /** 不可写作(无角色/Campaign):输入禁用,但不显示停止键 */
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
  <div class="shrink-0 border-t border-line bg-bg">
    <div class="mx-auto w-full max-w-[720px] px-4 sm:px-8 py-3">
      <!-- 纸卡输入框 -->
      <div
        class="flex items-end gap-2 rounded-xl border bg-surface px-3.5 py-2.5 shadow-card transition-colors"
        :class="writing ? 'border-line' : 'border-line focus-within:border-accent-border'"
      >
        <textarea
          v-model="intent"
          :placeholder="placeholder || (writing ? '写作中…' : sampleIntent)"
          :disabled="disabled || writing"
          rows="1"
          class="flex-1 bg-transparent resize-none outline-none text-[15px] leading-relaxed text-ink placeholder:text-ink-faint max-h-32 disabled:opacity-50"
          @keydown.enter.exact.prevent="submit"
          @input="$event.target.style.height='auto'; $event.target.style.height=$event.target.scrollHeight+'px'"
        ></textarea>

        <button
          v-if="!writing"
          @click="submit"
          :disabled="!intent.trim() || disabled"
          class="shrink-0 w-9 h-9 rounded-full flex items-center justify-center transition-colors"
          :class="intent.trim() && !disabled
            ? 'bg-accent text-white hover:bg-accent-bright'
            : 'bg-surface-2 text-ink-faint'"
          aria-label="发送"
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M5 12h14M13 6l6 6-6 6"/>
          </svg>
        </button>
        <button
          v-else
          @click="$emit('cancel')"
          class="shrink-0 w-9 h-9 rounded-full flex items-center justify-center bg-err text-white hover:opacity-85 transition-opacity"
          aria-label="停止生成"
          title="停止生成"
        >
          <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor"><rect x="5" y="5" width="14" height="14" rx="2"/></svg>
        </button>
      </div>
      <div class="mt-1.5 px-1 text-[11px] text-ink-faint">
        <template v-if="writing">写作中 · 点停止键中断</template>
        <template v-else>描述你要写的场景或意图 · 回车发送 · Shift+Enter 换行</template>
      </div>
    </div>
  </div>
</template>
