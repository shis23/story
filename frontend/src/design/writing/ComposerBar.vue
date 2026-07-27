<script setup>
/**
 * ComposerBar — 写作意图输入（重设计：稿纸下方的独立指令条）。
 *
 * 结构：生成模式 + 写作意图 + 发送/停止键。
 * 不展示只会预填文本的伪快捷能力，所有意图由用户直接输入。
 */
import { ref } from 'vue'
import { generationModeCatalog } from '../../utils/generationModes.js'

const props = defineProps({
  writing: { type: Boolean, default: false },
  disabled: { type: Boolean, default: false },
  placeholder: { type: String, default: '描述下一步想写什么…' },
  generationMode: { type: String, default: 'continuation' },
  showGenerationModes: { type: Boolean, default: false },
})
const emit = defineEmits(['start-writing', 'cancel', 'update-generation-mode'])

const intent = ref('')
const textareaRef = ref(null)

const generationModes = generationModeCatalog

function submit() {
  if (!intent.value.trim() || props.disabled || props.writing) return
  emit('start-writing', intent.value)
  intent.value = ''
}
</script>

<template>
  <div
    class="overflow-hidden rounded-2xl border bg-surface shadow-rise transition-all"
    :class="writing ? 'border-line' : 'border-line focus-within:border-accent-border focus-within:shadow-card'"
  >
    <div v-if="showGenerationModes" class="flex flex-wrap items-center gap-1.5 px-4 pb-2 pt-3" aria-label="生成模式">
      <span class="mr-1 text-[10px] font-medium uppercase tracking-[0.14em] text-ink-faint">本轮模式</span>
      <button
        v-for="mode in generationModes"
        :key="mode.value"
        type="button"
        :disabled="disabled || writing"
        class="min-h-7 rounded-lg border px-2.5 text-xs transition-all disabled:opacity-40"
        :class="generationMode === mode.value
          ? 'border-accent-border bg-accent-soft text-accent-bright shadow-sm'
          : 'border-line bg-surface-2 text-ink-soft hover:border-accent-border'"
        :title="`${mode.hint} · ${mode.callEstimate}`"
        @click="emit('update-generation-mode', mode.value)"
      >
        {{ mode.label }}
      </button>
      <span class="ml-1 text-[10px] text-ink-faint">
        {{ generationModes.find((mode) => mode.value === generationMode)?.callEstimate }}
      </span>
    </div>

    <div
      data-testid="composer-input-shell"
      class="mx-3 mb-3 flex items-end gap-2 rounded-xl bg-surface-2/65 px-3 py-2.5 transition-all focus-within:bg-bg focus-within:shadow-[inset_0_0_0_1px_var(--color-accent-border)]"
    >
      <textarea
        ref="textareaRef"
        v-model="intent"
        aria-label="写作意图"
        :placeholder="writing ? '写作中…' : placeholder"
        :disabled="disabled || writing"
        rows="1"
        class="composer-textarea min-h-[3.5rem] max-h-36 flex-1 resize-none appearance-none border-0 bg-transparent px-1 py-1 text-[15px] leading-7 text-ink outline-none placeholder:text-ink-faint focus:outline-none focus:ring-0 disabled:opacity-50"
        @keydown.enter.exact.prevent="submit"
        @input="$event.target.style.height='auto'; $event.target.style.height=$event.target.scrollHeight+'px'"
      ></textarea>

      <button
        v-if="!writing"
        @click="submit"
        :disabled="!intent.trim() || disabled"
        class="mb-0.5 flex h-10 w-10 shrink-0 items-center justify-center rounded-xl transition-all"
        :class="intent.trim() && !disabled
          ? 'bg-accent text-white shadow-card hover:-translate-y-0.5 hover:bg-accent-bright'
          : 'bg-surface-2 text-ink-faint'"
        aria-label="开始写作"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 2L11 13"/><path d="M22 2l-7 20-4-9-9-4z"/></svg>
      </button>
      <button
        v-else
        @click="emit('cancel')"
        class="mb-0.5 flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-err text-white transition-opacity hover:opacity-85"
        aria-label="停止生成"
        title="停止生成"
      >
        <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor"><rect x="5" y="5" width="14" height="14" rx="2"/></svg>
      </button>
    </div>

  </div>
</template>

<style scoped>
.composer-textarea:focus,
.composer-textarea:focus-visible {
  border: 0;
  outline: none;
  box-shadow: none !important;
}
</style>
