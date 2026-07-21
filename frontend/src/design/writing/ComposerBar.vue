<script setup>
/**
 * ComposerBar — 写作意图输入（重设计：稿纸下方的独立指令条）。
 *
 * 结构（对齐选定图②）：占位提示 + 快捷指令 chips + 圆形发送/停止键。
 * chips 为纯展示层预填（向 textarea 注入前缀文本），不发明后端能力——
 * 接线时原样作为 start-writing 文本发出即可。
 */
import { ref } from 'vue'

const props = defineProps({
  writing: { type: Boolean, default: false },
  disabled: { type: Boolean, default: false },
  placeholder: { type: String, default: '告诉你的想法，或用快捷指令开始…' },
})
const emit = defineEmits(['start-writing', 'cancel'])

const intent = ref('')
const textareaRef = ref(null)

const chips = [
  { label: '续写', prefix: '继续往下写：' },
  { label: '润色', prefix: '润色这段文字：' },
  { label: '扩写', prefix: '扩写这段：' },
  { label: '改写', prefix: '换一种写法重写：' },
  { label: '总结', prefix: '总结以上内容：' },
]

function applyChip(chip) {
  if (props.disabled || props.writing) return
  intent.value = chip.prefix
  textareaRef.value?.focus()
}

function submit() {
  if (!intent.value.trim() || props.disabled || props.writing) return
  emit('start-writing', intent.value)
  intent.value = ''
}
</script>

<template>
  <div
    class="rounded-xl border bg-surface px-4 py-3 shadow-card transition-colors"
    :class="writing ? 'border-line' : 'border-line focus-within:border-accent-border'"
  >
    <div class="flex items-end gap-3">
      <textarea
        ref="textareaRef"
        v-model="intent"
        :placeholder="writing ? '写作中…' : placeholder"
        :disabled="disabled || writing"
        rows="1"
        class="flex-1 bg-transparent resize-none outline-none text-[15px] leading-relaxed text-ink placeholder:text-ink-faint max-h-36 disabled:opacity-50"
        @keydown.enter.exact.prevent="submit"
        @input="$event.target.style.height='auto'; $event.target.style.height=$event.target.scrollHeight+'px'"
      ></textarea>

      <button
        v-if="!writing"
        @click="submit"
        :disabled="!intent.trim() || disabled"
        class="shrink-0 w-10 h-10 rounded-full flex items-center justify-center transition-colors"
        :class="intent.trim() && !disabled
          ? 'bg-accent text-white hover:bg-accent-bright shadow-card'
          : 'bg-surface-2 text-ink-faint'"
        aria-label="开始写作"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 2L11 13"/><path d="M22 2l-7 20-4-9-9-4z"/></svg>
      </button>
      <button
        v-else
        @click="emit('cancel')"
        class="shrink-0 w-10 h-10 rounded-full flex items-center justify-center bg-err text-white hover:opacity-85 transition-opacity"
        aria-label="停止生成"
        title="停止生成"
      >
        <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor"><rect x="5" y="5" width="14" height="14" rx="2"/></svg>
      </button>
    </div>

    <!-- 快捷指令 -->
    <div class="mt-2 flex items-center gap-1 flex-wrap">
      <button
        v-for="chip in chips"
        :key="chip.label"
        @click="applyChip(chip)"
        :disabled="disabled || writing"
        class="min-h-7 px-2.5 rounded-md text-xs text-ink-soft hover:bg-accent-soft hover:text-accent-bright disabled:opacity-40 transition-colors"
      >{{ chip.label }}</button>
      <span class="ml-auto text-[11px] text-ink-faint hidden sm:inline">
        <template v-if="writing">写作中 · 点右侧停止键中断</template>
        <template v-else>回车发送 · Shift+Enter 换行</template>
      </span>
    </div>
  </div>
</template>
