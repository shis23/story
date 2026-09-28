<script setup>
import { ref, watch, nextTick, onMounted } from 'vue'

const props = defineProps({
  modelValue: { type: String, default: '' },
  placeholder: { type: String, default: '' },
  disabled: { type: Boolean, default: false },
  autoResize: { type: Boolean, default: false },
  rows: { type: Number, default: 3 },
  invalid: { type: Boolean, default: false },
})
defineEmits(['update:modelValue'])

const el = ref(null)

function resize() {
  if (!props.autoResize || !el.value) return
  el.value.style.height = 'auto'
  el.value.style.height = el.value.scrollHeight + 'px'
}

watch(
  () => props.modelValue,
  () => nextTick(resize)
)

// F-35：autoResize 此前只在 modelValue 变化时 resize —— 首帧高度仍是 rows
// 的默认值；调用方若只读 :model-value（不 v-model 回写）则输入永不触发 resize。
onMounted(() => nextTick(resize))
</script>

<template>
  <textarea
    ref="el"
    :value="modelValue"
    :placeholder="placeholder"
    :disabled="disabled"
    :rows="rows"
    :aria-invalid="invalid ? 'true' : undefined"
    class="w-full bg-surface border border-line rounded-md px-3 py-1.5 text-sm text-ink placeholder:text-ink-faint transition-colors duration-150 outline-none focus:border-accent-border disabled:bg-surface-2 disabled:text-ink-faint disabled:cursor-not-allowed resize-y"
    :class="invalid ? 'border-err focus:border-err' : ''"
    @input="$emit('update:modelValue', $event.target.value); resize()"
  />
</template>
