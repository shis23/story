<script setup>
import { computed } from 'vue'
import {
  Dialog as HDialog,
  DialogPanel,
  DialogTitle,
  TransitionRoot,
  TransitionChild,
} from '@headlessui/vue'

const props = defineProps({
  show: { type: Boolean, default: false },
  title: { type: String, default: '' },
  variant: { type: String, default: 'confirm' }, // confirm | danger | warn
  confirmText: { type: String, default: '确认' },
  cancelText: { type: String, default: '取消' },
})
const emit = defineEmits(['update:show', 'confirm', 'cancel', 'close'])

const isOpen = computed({
  get: () => props.show,
  set: (val) => {
    emit('update:show', val)
    if (!val) emit('close')
  },
})

// 确认按钮按 variant 取色
const confirmBtnClass = computed(() => {
  switch (props.variant) {
    case 'danger':
      return 'bg-err text-bg font-medium hover:opacity-90'
    case 'warn':
      return 'bg-warn text-bg font-medium hover:opacity-90'
    case 'confirm':
    default:
      return 'bg-accent text-bg font-medium shadow-glow-accent hover:opacity-90'
  }
})

function handleConfirm() {
  emit('confirm')
  isOpen.value = false
}
function handleCancel() {
  emit('cancel')
  isOpen.value = false
}
</script>

<template>
  <TransitionRoot :show="isOpen" as="template">
    <HDialog @close="isOpen = false" class="relative z-50">
      <!-- 遮罩 -->
      <TransitionChild
        as="template"
        enter="transition-opacity duration-200"
        enterFrom="opacity-0"
        enterTo="opacity-100"
        leave="transition-opacity duration-150"
        leaveFrom="opacity-100"
        leaveTo="opacity-0"
      >
        <div class="fixed inset-0 bg-bg/70" aria-hidden="true" />
      </TransitionChild>

      <!-- 居中容器 -->
      <div class="fixed inset-0 flex items-center justify-center p-4">
        <TransitionChild
          as="template"
          enter="transition ease-[cubic-bezier(0.22,1,0.36,1)] duration-200"
          enterFrom="opacity-0 scale-95"
          enterTo="opacity-100 scale-100"
          leave="transition ease-[cubic-bezier(0.22,1,0.36,1)] duration-150"
          leaveFrom="opacity-100 scale-100"
          leaveTo="opacity-0 scale-95"
        >
          <DialogPanel
            class="w-full max-w-sm bg-surface rounded-xl shadow-float p-5"
          >
            <DialogTitle
              v-if="title"
              class="text-lg text-ink font-medium"
            >
              {{ title }}
            </DialogTitle>

            <!-- 内容 -->
            <div class="mt-3 text-sm text-ink-soft">
              <slot />
            </div>

            <!-- 底部按钮 -->
            <div class="mt-5 flex justify-end gap-2">
              <button
                type="button"
                class="inline-flex items-center justify-center px-3.5 py-1.5 text-sm rounded-lg bg-surface-2 text-ink border border-line hover:border-accent-border transition-colors duration-150 select-none"
                @click="handleCancel"
              >
                {{ cancelText }}
              </button>
              <button
                type="button"
                class="inline-flex items-center justify-center px-3.5 py-1.5 text-sm rounded-lg transition-opacity duration-150 select-none"
                :class="confirmBtnClass"
                @click="handleConfirm"
              >
                {{ confirmText }}
              </button>
            </div>
          </DialogPanel>
        </TransitionChild>
      </div>
    </HDialog>
  </TransitionRoot>
</template>
