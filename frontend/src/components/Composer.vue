<script setup>
import { ref } from 'vue'
import { sampleIntent } from '../mock.js'

const props = defineProps({
  disabled: { type: Boolean, default: false },
  placeholder: { type: String, default: '' },
})

const emit = defineEmits(['start-writing'])

const intent = ref('')

function submit() {
  if (!intent.value.trim() || props.disabled) return
  emit('start-writing', intent.value)
  intent.value = ''
}
</script>

<template>
  <div class="sticky bottom-0 bg-surface/90 backdrop-blur-md border-t border-line p-3">
    <!-- 意图输入框（多行，自动撑高） -->
    <div class="flex items-end gap-2 bg-bg rounded-2xl border border-line px-3 py-2 focus-within:border-accent transition-colors">
      <textarea
        v-model="intent"
        :placeholder="placeholder || (disabled ? '写作中…' : sampleIntent)"
        :disabled="disabled"
        rows="1"
        class="flex-1 bg-transparent resize-none outline-none text-[15px] text-ink placeholder:text-ink-soft/50 max-h-32 disabled:opacity-50"
        @keydown.enter.exact.prevent="submit"
        @input="$event.target.style.height='auto'; $event.target.style.height=$event.target.scrollHeight+'px'"
      ></textarea>
      <button
        @click="submit"
        :disabled="!intent.trim() || disabled"
        class="shrink-0 w-9 h-9 rounded-full flex items-center justify-center transition-all"
        :class="intent.trim() && !disabled
          ? 'bg-accent text-white hover:opacity-90'
          : 'bg-bg text-ink-soft'"
      >
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M5 12h14M13 6l6 6-6 6"/>
        </svg>
      </button>
    </div>
    <div class="text-[11px] text-ink-soft/60 mt-1.5 px-1">
      描述你要写的场景或意图 · 回车发送
    </div>
  </div>
</template>
