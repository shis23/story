<script setup>
import { ref, watch } from 'vue'
import { useClickOutside } from './useClickOutside.js'

const props = defineProps({
  modelValue: { type: Boolean, default: false }, // 控制展开
  /** 下拉对齐：默认左对齐触发器 */
  align: { type: String, default: 'left' }, // 'left' | 'right'
  /** 下拉最小宽度（px），默认与触发器同宽 */
  minWidth: { type: Number, default: 0 },
  /** 是否禁用 */
  disabled: { type: Boolean, default: false },
})
const emit = defineEmits(['update:modelValue'])

const rootRef = ref(null)
const open = ref(props.modelValue)

watch(() => props.modelValue, (v) => { open.value = v })
watch(open, (v) => { if (v !== props.modelValue) emit('update:modelValue', v) })

useClickOutside(rootRef, () => { open.value = false }, { enabled: true })

function toggle() {
  if (props.disabled) return
  open.value = !open.value
}

// ESC 关闭
watch(open, (v) => {
  if (!v) return
  const onKey = (e) => {
    if (e.key === 'Escape') {
      open.value = false
      document.removeEventListener('keydown', onKey, true)
    }
  }
  document.addEventListener('keydown', onKey, true)
})

defineExpose({ close: () => { open.value = false } })
</script>

<template>
  <div ref="rootRef" class="relative inline-block">
    <!-- 触发器：点击切换 -->
    <div @click="toggle">
      <slot name="trigger" :open="open" :toggle="toggle" />
    </div>
    <Transition name="sf-dropdown">
      <div
        v-if="open"
        class="absolute z-30 mt-1 bg-surface border border-line rounded-xl shadow-lg py-1"
        :class="align === 'right' ? 'right-0' : 'left-0'"
        :style="minWidth ? { minWidth: minWidth + 'px' } : {}"
        @click.stop
      >
        <slot :close="() => { open = false }" />
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.sf-dropdown-enter-active,
.sf-dropdown-leave-active {
  transition: opacity 0.14s ease, transform 0.14s ease;
}
.sf-dropdown-enter-from,
.sf-dropdown-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}
</style>
