<script setup>
import { computed } from 'vue'
import {
  Dialog,
  DialogPanel,
  TransitionRoot,
  TransitionChild,
} from '@headlessui/vue'

const props = defineProps({
  show: { type: Boolean, default: false },
  modelValue: { type: Boolean, default: null },
  side: { type: String, default: 'right' }, // left | right | center | full
  title: { type: String, default: '' },
})
const emit = defineEmits(['update:show', 'update:modelValue', 'close'])

// 兼容 v-model:show 与 v-model:modelValue 两种用法；modelValue 优先（若显式传了）
const isOpen = computed({
  get() {
    if (props.modelValue !== null) return props.modelValue
    return props.show
  },
  set(val) {
    emit('update:show', val)
    emit('update:modelValue', val)
    if (!val) emit('close')
  },
})

// 面板定位与过渡方向
const panelWrapperClass = computed(() => {
  switch (props.side) {
    case 'left':
      return 'fixed left-0 top-0 h-full'
    case 'center':
      return 'fixed inset-0 flex items-center justify-center p-4'
    case 'full':
      return 'fixed inset-0'
    case 'right':
    default:
      return 'fixed right-0 top-0 h-full'
  }
})

const panelClass = computed(() => {
  const base = 'bg-surface shadow-float flex flex-col overflow-hidden'
  switch (props.side) {
    case 'left':
      return `${base} h-full w-full max-w-sm`
    case 'center':
      return `${base} w-full max-w-lg rounded-xl`
    case 'full':
      return `${base} h-full w-full`
    case 'right':
    default:
      return `${base} h-full w-full max-w-sm`
  }
})

// 过渡动画方向：左右抽屉横向滑入，center/full 用淡入缩放
const panelTransition = computed(() => {
  switch (props.side) {
    case 'left':
      return {
        enter: 'transition ease-[cubic-bezier(0.22,1,0.36,1)] duration-200',
        enterFrom: '-translate-x-full opacity-0',
        enterTo: 'translate-x-0 opacity-100',
        leave: 'transition ease-[cubic-bezier(0.22,1,0.36,1)] duration-150',
        leaveFrom: 'translate-x-0 opacity-100',
        leaveTo: '-translate-x-full opacity-0',
      }
    case 'right':
      return {
        enter: 'transition ease-[cubic-bezier(0.22,1,0.36,1)] duration-200',
        enterFrom: 'translate-x-full opacity-0',
        enterTo: 'translate-x-0 opacity-100',
        leave: 'transition ease-[cubic-bezier(0.22,1,0.36,1)] duration-150',
        leaveFrom: 'translate-x-0 opacity-100',
        leaveTo: 'translate-x-full opacity-0',
      }
    default:
      return {
        enter: 'transition ease-[cubic-bezier(0.22,1,0.36,1)] duration-200',
        enterFrom: 'opacity-0 scale-95',
        enterTo: 'opacity-100 scale-100',
        leave: 'transition ease-[cubic-bezier(0.22,1,0.36,1)] duration-150',
        leaveFrom: 'opacity-100 scale-100',
        leaveTo: 'opacity-0 scale-95',
      }
  }
})

const overlayTransition = {
  enter: 'transition-opacity duration-200',
  enterFrom: 'opacity-0',
  enterTo: 'opacity-100',
  leave: 'transition-opacity duration-150',
  leaveFrom: 'opacity-100',
  leaveTo: 'opacity-0',
}
</script>

<template>
  <TransitionRoot :show="isOpen" as="template">
    <Dialog @close="isOpen = false" class="relative z-50">
      <!-- 遮罩 -->
      <TransitionChild as="template" v-bind="overlayTransition">
        <div class="fixed inset-0 bg-bg/70" aria-hidden="true" />
      </TransitionChild>

      <!-- 面板容器 -->
      <div :class="panelWrapperClass">
        <TransitionChild as="template" v-bind="panelTransition">
          <DialogPanel :class="panelClass">
            <!-- 标题栏 -->
            <div
              v-if="title"
              class="flex items-center justify-between px-4 py-3 border-b border-line shrink-0"
            >
              <h2 class="text-base font-medium text-ink">{{ title }}</h2>
              <button
                type="button"
                class="text-ink-soft hover:text-ink transition-colors px-1 -mr-1"
                @click="isOpen = false"
                aria-label="关闭"
              >
                ✕
              </button>
            </div>
            <!-- 无标题时仍提供关闭按钮 -->
            <div
              v-else
              class="absolute right-2 top-2 z-10"
            >
              <button
                type="button"
                class="text-ink-soft hover:text-ink transition-colors px-1"
                @click="isOpen = false"
                aria-label="关闭"
              >
                ✕
              </button>
            </div>

            <!-- 内容 -->
            <div class="flex-1 overflow-auto">
              <slot :close="() => (isOpen = false)" />
            </div>
          </DialogPanel>
        </TransitionChild>
      </div>
    </Dialog>
  </TransitionRoot>
</template>
