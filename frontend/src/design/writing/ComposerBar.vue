<script setup>
/**
 * ComposerBar — 写作意图输入（重设计：稿纸下方的独立指令条）。
 *
 * 结构：生成模式 + 写作意图 + 发送/停止键。
 * 不展示只会预填文本的伪快捷能力，所有意图由用户直接输入。
 */
import { ref } from 'vue'
import { ArrowUp, Square } from '@lucide/vue'
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
    class="overflow-hidden rounded-lg border bg-surface shadow-rise transition-[border-color,box-shadow]"
    :class="writing ? 'border-line' : 'border-line focus-within:border-accent-border focus-within:shadow-card'"
  >
    <div v-if="showGenerationModes" class="flex flex-wrap items-center gap-2 px-3 pb-2 pt-3" aria-label="生成模式">
      <div class="flex min-w-0 items-center gap-0.5 rounded-md bg-surface-2 p-0.5" role="group" aria-label="本轮模式">
      <button
        v-for="mode in generationModes"
        :key="mode.value"
        type="button"
        :disabled="disabled || writing"
        class="min-h-8 rounded px-2.5 text-xs transition-[background-color,color,box-shadow] disabled:opacity-40"
        :class="generationMode === mode.value
          ? 'bg-surface text-accent-bright shadow-card'
          : 'text-ink-soft hover:text-ink'"
        :aria-pressed="generationMode === mode.value"
        :title="`${mode.hint} · ${mode.callEstimate}`"
        @click="emit('update-generation-mode', mode.value)"
      >
        {{ mode.label }}
      </button>
      </div>
      <span class="ml-1 text-[10px] text-ink-faint">
        {{ generationModes.find((mode) => mode.value === generationMode)?.callEstimate }}
      </span>
    </div>

    <div
      data-testid="composer-input-shell"
      class="mx-3 mb-3 flex min-w-0 items-end gap-2 border-t border-line pt-2"
    >
      <textarea
        ref="textareaRef"
        v-model="intent"
        aria-label="写作意图"
        :placeholder="writing ? '写作中…' : placeholder"
        :disabled="disabled || writing"
        rows="1"
        class="composer-textarea min-w-0 min-h-[3.5rem] max-h-36 flex-1 resize-none appearance-none border-0 bg-transparent px-1 py-1 text-[15px] leading-7 text-ink outline-none placeholder:text-ink-faint focus:outline-none focus:ring-0 disabled:opacity-50"
        @keydown.enter.exact.prevent="submit"
        @input="$event.target.style.height='auto'; $event.target.style.height=$event.target.scrollHeight+'px'"
      ></textarea>

      <button
        v-if="!writing"
        @click="submit"
        :disabled="!intent.trim() || disabled"
        class="sf-command mb-0.5 flex h-10 w-10 shrink-0 items-center justify-center rounded-md"
        :class="intent.trim() && !disabled
          ? 'bg-accent text-white shadow-card hover:brightness-110'
          : 'bg-surface-2 text-ink-faint'"
        aria-label="开始写作"
        title="开始写作"
      >
        <ArrowUp :size="20" aria-hidden="true" />
      </button>
      <button
        v-else
        @click="emit('cancel')"
        class="sf-command mb-0.5 flex h-10 w-10 shrink-0 items-center justify-center rounded-md bg-err text-white hover:opacity-85"
        aria-label="停止生成"
        title="停止生成"
      >
        <Square :size="14" fill="currentColor" aria-hidden="true" />
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
