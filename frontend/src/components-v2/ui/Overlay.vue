<script setup>
import { computed } from 'vue'
import {
  Dialog,
  DialogPanel,
  TransitionRoot,
  TransitionChild,
} from '@headlessui/vue'

/**
 * Overlay — 统一弹层/抽屉。
 *
 * 宽度策略（纸上编辑部）：
 *   left/right 默认固定 --layout-drawer（380px），内容再长也不撑开、再空也不缩。
 *   center 默认 --layout-dialog；full 全屏。
 *   可用 panelWidthClass 覆盖（仅特殊屏，如 Inspector）。
 */
const props = defineProps({
  show: { type: Boolean, default: false },
  modelValue: { type: Boolean, default: null },
  side: { type: String, default: 'right' }, // left | right | center | full
  title: { type: String, default: '' },
  // P3-1：子组件自带关闭键时传 false，避免双 ×
  showClose: { type: Boolean, default: true },
  /**
   * 覆盖默认宽度 class。
   * 空字符串 = 使用布局 token 默认（推荐）。
   * 例 Inspector：'w-[min(100vw,var(--layout-inspector))]'
   */
  panelWidthClass: { type: String, default: '' },
})
const emit = defineEmits(['update:show', 'update:modelValue', 'close'])

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

/** 默认：侧滑固定 drawer 宽；居中 dialog 宽；全屏 100% */
const defaultSideWidth =
  'w-[min(100vw,var(--layout-drawer))] shrink-0'
const defaultCenterWidth =
  'w-full max-w-[var(--layout-dialog)]'

const panelClass = computed(() => {
  const base = 'bg-surface shadow-float flex flex-col overflow-hidden min-w-0'
  const custom = props.panelWidthClass?.trim()
  switch (props.side) {
    case 'left':
    case 'right':
      // 固定宽：min() 封顶视口；不用 max-w-only 以免内容把壳撑到不同视觉宽度
      return `${base} h-full ${custom || defaultSideWidth}`
    case 'center':
      return `${base} ${custom || defaultCenterWidth} rounded-xl`
    case 'full':
      return `${base} h-full w-full`
    default:
      return `${base} h-full ${custom || defaultSideWidth}`
  }
})

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
    <Dialog @close="isOpen = false" class="relative z-[var(--z-drawer)]">
      <TransitionChild as="template" v-bind="overlayTransition">
        <div class="fixed inset-0 bg-bg/70" aria-hidden="true" />
      </TransitionChild>

      <div :class="panelWrapperClass">
        <TransitionChild as="template" v-bind="panelTransition">
          <DialogPanel :class="panelClass">
            <div
              v-if="title"
              class="flex items-center justify-between px-4 py-3 border-b border-line shrink-0"
            >
              <h2 class="text-base font-semibold text-ink">{{ title }}</h2>
              <button
                type="button"
                class="text-ink-soft hover:text-ink transition-colors px-1 -mr-1"
                @click="isOpen = false"
                aria-label="关闭"
              >
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18"/></svg>
              </button>
            </div>
            <div
              v-else-if="showClose"
              class="absolute right-2 top-2 z-10"
            >
              <button
                type="button"
                class="text-ink-soft hover:text-ink transition-colors px-1"
                @click="isOpen = false"
                aria-label="关闭"
              >
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18"/></svg>
              </button>
            </div>

            <!-- 侧滑：overflow-hidden 交给 PanelHost 内容井滚动，避免双滚动抢手势；
                 居中/全屏：自身可滚 -->
            <div
              class="flex-1 min-h-0 min-w-0 flex flex-col"
              :class="side === 'left' || side === 'right' || side === 'full'
                ? 'overflow-hidden'
                : 'overflow-y-auto overscroll-y-contain sf-drawer-scroll'"
            >
              <slot :close="() => (isOpen = false)" />
            </div>
          </DialogPanel>
        </TransitionChild>
      </div>
    </Dialog>
  </TransitionRoot>
</template>
