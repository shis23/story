<script setup>
import { computed, ref } from 'vue'

const props = defineProps({
  content: { type: String, default: '' },
  placement: { type: String, default: 'top' }, // top | bottom | left | right
})
const visible = ref(false)

function show() {
  visible.value = true
}
function hide() {
  visible.value = false
}

// 各方向的定位类
const positionClass = computed(() => {
  switch (props.placement) {
    case 'bottom':
      return 'top-full left-1/2 -translate-x-1/2 mt-1.5'
    case 'left':
      return 'right-full top-1/2 -translate-y-1/2 mr-1.5'
    case 'right':
      return 'left-full top-1/2 -translate-y-1/2 ml-1.5'
    case 'top':
    default:
      return 'bottom-full left-1/2 -translate-x-1/2 mb-1.5'
  }
})

// 进出动画方向
const transitionClass = computed(() => {
  switch (props.placement) {
    case 'bottom':
      return {
        enter: 'transition ease-[cubic-bezier(0.22,1,0.36,1)] duration-150',
        enterFrom: 'opacity-0 translate-y-1',
        enterTo: 'opacity-100 translate-y-0',
        leave: 'transition ease-[cubic-bezier(0.22,1,0.36,1)] duration-100',
        leaveFrom: 'opacity-100 translate-y-0',
        leaveTo: 'opacity-0 translate-y-1',
      }
    case 'left':
      return {
        enter: 'transition ease-[cubic-bezier(0.22,1,0.36,1)] duration-150',
        enterFrom: 'opacity-0 translate-x-1',
        enterTo: 'opacity-100 translate-x-0',
        leave: 'transition ease-[cubic-bezier(0.22,1,0.36,1)] duration-100',
        leaveFrom: 'opacity-100 translate-x-0',
        leaveTo: 'opacity-0 translate-x-1',
      }
    case 'right':
      return {
        enter: 'transition ease-[cubic-bezier(0.22,1,0.36,1)] duration-150',
        enterFrom: 'opacity-0 -translate-x-1',
        enterTo: 'opacity-100 translate-x-0',
        leave: 'transition ease-[cubic-bezier(0.22,1,0.36,1)] duration-100',
        leaveFrom: 'opacity-100 translate-x-0',
        leaveTo: 'opacity-0 -translate-x-1',
      }
    case 'top':
    default:
      return {
        enter: 'transition ease-[cubic-bezier(0.22,1,0.36,1)] duration-150',
        enterFrom: 'opacity-0 -translate-y-1',
        enterTo: 'opacity-100 translate-y-0',
        leave: 'transition ease-[cubic-bezier(0.22,1,0.36,1)] duration-100',
        leaveFrom: 'opacity-100 translate-y-0',
        leaveTo: 'opacity-0 -translate-y-1',
      }
  }
})
</script>

<template>
  <span
    class="relative inline-flex"
    @mouseenter="show"
    @mouseleave="hide"
    @focusin="show"
    @focusout="hide"
  >
    <slot />

    <Transition v-bind="transitionClass">
      <span
        v-if="visible && content"
        :class="[
          'absolute z-50 bg-surface-2 text-ink text-xs px-2 py-1 rounded shadow-float border border-line whitespace-nowrap pointer-events-none',
          positionClass,
        ]"
        role="tooltip"
      >
        {{ content }}
      </span>
    </Transition>
  </span>
</template>
