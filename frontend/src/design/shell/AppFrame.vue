<script setup>
/**
 * AppFrame — 应用壳（重设计 · 纯展示布局）。
 *
 * 对齐 selected 图：
 *   - 左栏 260px 可收起（lg+）；移动端抽屉
 *   - TopBar 52px
 *   - 主区自滚动；Inspector 覆盖式（不挤压写作栏）
 *
 * 不读 store。展开态由 props 注入；导航与面板由 slots 填充。
 * 插槽提供 docked；sidebar/topbar 另提供收起与切换回调。
 */
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'

const props = defineProps({
  sidebarOpen: { type: Boolean, default: false },
  sidebarCollapsed: { type: Boolean, default: false },
  inspectorOpen: { type: Boolean, default: false },
})

const emit = defineEmits(['update:sidebarOpen', 'update:sidebarCollapsed', 'update:inspectorOpen'])

// Keep one sidebar runtime mounted across visibility and breakpoint changes.
const isDesktop = ref(typeof window === 'undefined' ? true : window.innerWidth >= 1024)
const sidebarVisible = computed(() => isDesktop.value ? !props.sidebarCollapsed : props.sidebarOpen)

function syncDesktopBreakpoint() {
  const nextDesktop = window.innerWidth >= 1024
  // A drawer opened at an earlier width must not reappear on a later resize.
  if (nextDesktop !== isDesktop.value && props.sidebarOpen) emit('update:sidebarOpen', false)
  isDesktop.value = nextDesktop
}

function toggleSidebar() {
  if (isDesktop.value) emit('update:sidebarCollapsed', !props.sidebarCollapsed)
  else emit('update:sidebarOpen', !props.sidebarOpen)
}

function collapseSidebar() {
  if (isDesktop.value) emit('update:sidebarCollapsed', true)
  else emit('update:sidebarOpen', false)
}

onMounted(() => window.addEventListener('resize', syncDesktopBreakpoint))
onBeforeUnmount(() => window.removeEventListener('resize', syncDesktopBreakpoint))
</script>

<template>
  <div class="sf-safe-screen h-dvh flex flex-col bg-bg overflow-hidden">
    <div class="flex-1 flex min-h-0">
      <!--
        One persistent sidebar keeps runtime slots alive across responsive
        breakpoints; mobile changes its presentation into a drawer instead of
        mounting another copy.
      -->
      <Transition name="sf-drawer-left" :css="!isDesktop">
        <aside
          v-show="sidebarVisible"
          :inert="!sidebarVisible"
          class="w-[min(100vw,var(--layout-sidebar))] bg-bg flex flex-col min-h-0"
          :class="isDesktop
            ? 'shrink-0 border-r border-line'
            : 'sf-safe-screen fixed top-0 bottom-0 left-0 z-[var(--z-drawer)] border-r border-line shadow-float'"
        >
          <slot name="sidebar" :docked="isDesktop" :collapse-sidebar="collapseSidebar" />
        </aside>
      </Transition>

      <main class="flex-1 flex flex-col min-w-0 min-h-0">
        <div class="shrink-0">
          <slot name="topbar" :docked="isDesktop" :sidebar-visible="sidebarVisible" :toggle-sidebar="toggleSidebar" />
        </div>
        <div class="flex-1 min-h-0 overflow-hidden flex flex-col">
          <slot name="content" />
        </div>
        <slot name="composer" />
      </main>
    </div>

    <!-- 移动侧栏遮罩 + 抽屉 -->
    <Transition name="sf-fade">
      <div
        v-if="!isDesktop && sidebarOpen"
        class="fixed inset-0 z-[var(--z-overlay)] bg-ink/30"
        @click="$emit('update:sidebarOpen', false)"
      ></div>
    </Transition>
    <!-- Inspector：覆盖式；宽 --layout-inspector（略窄于功能抽屉） -->
    <Transition name="sf-fade">
      <div
        v-if="inspectorOpen"
        class="fixed inset-0 z-[var(--z-overlay)] bg-ink/25"
        @click="$emit('update:inspectorOpen', false)"
      ></div>
    </Transition>
    <Transition name="sf-drawer-right">
      <div
        v-if="inspectorOpen"
        class="sf-safe-screen fixed top-0 bottom-0 right-0 z-[var(--z-drawer)] w-[min(100vw,var(--layout-inspector))] bg-surface border-l border-line shadow-float flex flex-col"
      >
        <slot name="inspector" />
      </div>
    </Transition>

    <!-- 功能面板 / 隐藏 runtime（PluginHost、MvuJsRuntime 等） -->
    <slot name="panels" />
  </div>
</template>
