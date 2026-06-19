<script setup>
import { ref, watch, computed } from 'vue'
import { useClickOutside } from './useClickOutside.js'

const props = defineProps({
  modelValue: { type: Boolean, default: false },
  title: { type: String, default: '' },
  /** sm=28rem md=32rem lg=42rem 2xl=56rem full=全屏 */
  size: { type: String, default: 'md' },
  /** 'drawer' = 底部抽屉（移动）/ 居中（桌面）；'center' = 始终居中；'left' = 左侧滑出；'right' = 右侧滑出 */
  position: { type: String, default: 'drawer' },
  closeOnMask: { type: Boolean, default: true },
  closeOnEsc: { type: Boolean, default: true },
  /** 容器底色：默认 bg-bg（统一）；个别详情页可传 'surface' */
  surface: { type: String, default: 'bg' },
  /** 是否显示默认顶栏（标题+关闭按钮）；false 时由调用方在 slot 自绘 */
  showHeader: { type: Boolean, default: true },
  /** 内容区是否滚动；false 时去掉 overflow-y-auto，调用方自行管理内部滚动（如复杂分栏布局） */
  bodyScroll: { type: Boolean, default: true },
})
const emit = defineEmits(['update:modelValue', 'close'])

const contentRef = ref(null)

const sizeClass = computed(() => ({
  sm: 'sm:max-w-md',
  md: 'sm:max-w-lg',
  lg: 'sm:max-w-2xl',
  '2xl': 'sm:max-w-3xl',
  full: 'sm:max-w-4xl',
}[props.size] || 'sm:max-w-lg'))

// 定位：center 居中；drawer 底部抽屉(移动)/居中(桌面)；left/right 侧边滑出(贴边满高)
const positionClass = computed(() => {
  if (props.position === 'center') return 'items-center'
  if (props.position === 'left') return 'items-stretch justify-start'
  if (props.position === 'right') return 'items-stretch justify-end'
  return 'items-end sm:items-center' // drawer
})

// 侧边抽屉宽度约束（left/right）；drawer/center 用 size
const widthClass = computed(() => {
  if (props.position === 'left' || props.position === 'right') {
    return 'w-full ' + sizeClass.value + ' h-full'
  }
  return 'w-full ' + sizeClass.value
})

const surfaceClass = computed(() =>
  props.surface === 'surface' ? 'bg-surface' : 'bg-bg'
)

const isSide = computed(() => props.position === 'left' || props.position === 'right')

// 侧边抽屉圆角只在朝外那侧
const sideRoundClass = computed(() => {
  if (props.position === 'left') return 'rounded-r-2xl'
  if (props.position === 'right') return 'rounded-l-2xl'
  return ''
})

// 动画名：left/right 侧滑；其余上移淡入
const transitionName = computed(() =>
  isSide.value ? `sf-side-${props.position}` : 'sf-overlay'
)

function close() {
  emit('update:modelValue', false)
  emit('close')
}

function onMaskClick() {
  if (props.closeOnMask) close()
}

// 点击内容外部（即点遮罩）关闭 —— 内容区 click.stop 兜底
useClickOutside(contentRef, () => {
  if (props.closeOnMask) close()
}, { enabled: props.modelValue })

// ESC 关闭
watch(() => props.modelValue, (v) => {
  if (!v) return
  if (!props.closeOnEsc) return
  const onKey = (e) => {
    if (e.key === 'Escape') {
      close()
      document.removeEventListener('keydown', onKey, true)
    }
  }
  document.addEventListener('keydown', onKey, true)
})

// 弹层打开时锁 body 滚动，避免背景滚动穿透
watch(() => props.modelValue, (v) => {
  if (typeof document === 'undefined') return
  document.body.style.overflow = v ? 'hidden' : ''
})
</script>

<template>
  <Teleport to="body">
    <Transition :name="transitionName">
      <div
        v-if="modelValue"
        class="fixed inset-0 z-50 flex justify-center bg-black/40 backdrop-blur-sm"
        :class="[positionClass, isSide ? '' : 'p-0 sm:p-4']"
        @click.self="onMaskClick"
      >
        <div
          ref="contentRef"
          class="flex flex-col w-full shadow-xl border border-line overflow-hidden"
          :class="[widthClass, surfaceClass, isSide ? '' : 'max-h-[90vh] rounded-t-2xl sm:rounded-2xl', sideRoundClass]"
          @click.stop
        >
          <!-- 默认顶栏 -->
          <div v-if="showHeader" class="sticky top-0 z-10 flex items-center gap-3 px-4 py-3 border-b border-line bg-inherit shrink-0">
            <button
              @click="close"
              class="shrink-0 w-11 h-11 flex items-center justify-center rounded-full text-ink-soft hover:bg-accent-soft transition-colors"
              title="关闭"
              aria-label="关闭"
            >
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M19 12H5M11 18l-6-6 6-6"/></svg>
            </button>
            <h2 class="flex-1 min-w-0 font-medium text-ink truncate">{{ title }}</h2>
            <slot name="header-extra" />
          </div>

          <!-- 内容区：由调用方填充。bodyScroll=true 时自行滚动；false 时调用方管理内部滚动 -->
          <div :class="bodyScroll ? 'flex-1 min-h-0 overflow-y-auto' : 'flex-1 min-h-0 flex flex-col'">
            <slot :close="close" />
          </div>

          <!-- 可选底部 slot（操作栏） -->
          <div v-if="$slots.footer" class="shrink-0 border-t border-line px-4 py-3 bg-inherit">
            <slot name="footer" :close="close" />
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
/* 遮罩淡入；内容随 drawer/center 微上移 */
.sf-overlay-enter-active,
.sf-overlay-leave-active {
  transition: opacity 0.18s ease;
}
.sf-overlay-enter-active > div,
.sf-overlay-leave-active > div {
  transition: transform 0.22s cubic-bezier(0.22, 1, 0.36, 1), opacity 0.18s ease;
}
.sf-overlay-enter-from,
.sf-overlay-leave-to {
  opacity: 0;
}
.sf-overlay-enter-from > div,
.sf-overlay-leave-to > div {
  opacity: 0;
  transform: translateY(16px);
}

/* 侧边滑出：left 从左、right 从右 */
.sf-side-left-enter-active,
.sf-side-left-leave-active,
.sf-side-right-enter-active,
.sf-side-right-leave-active {
  transition: opacity 0.18s ease;
}
.sf-side-left-enter-active > div,
.sf-side-left-leave-active > div {
  transition: transform 0.26s cubic-bezier(0.22, 1, 0.36, 1);
}
.sf-side-right-enter-active > div,
.sf-side-right-leave-active > div {
  transition: transform 0.26s cubic-bezier(0.22, 1, 0.36, 1);
}
.sf-side-left-enter-from,
.sf-side-left-leave-to,
.sf-side-right-enter-from,
.sf-side-right-leave-to {
  opacity: 0;
}
.sf-side-left-enter-from > div,
.sf-side-left-leave-to > div {
  transform: translateX(-100%);
}
.sf-side-right-enter-from > div,
.sf-side-right-leave-to > div {
  transform: translateX(100%);
}
</style>
