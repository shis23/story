<script setup>
/**
 * AppFrame — 应用壳（重设计 · 纯展示布局）。
 *
 * 对齐 selected 图：
 *   - 左栏 260px 常驻（lg+）；移动端抽屉
 *   - TopBar 52px
 *   - 主区自滚动；Inspector 覆盖式（不挤压写作栏）
 *
 * 不读 store。展开态由 props 注入；导航与面板由 slots 填充。
 * sidebar / inspector 插槽参数：{ docked: boolean }
 */
import { onBeforeUnmount, onMounted, ref } from 'vue'

defineProps({
  sidebarOpen: { type: Boolean, default: false },
  inspectorOpen: { type: Boolean, default: false },
})

defineEmits(['update:sidebarOpen', 'update:inspectorOpen'])

// CSS `hidden` keeps slot content mounted. Runtime slots can execute scripts,
// so choose exactly one sidebar tree at the responsive breakpoint instead.
const isDesktop = ref(typeof window === 'undefined' ? true : window.innerWidth >= 1024)

function syncDesktopBreakpoint() {
  isDesktop.value = window.innerWidth >= 1024
}

onMounted(() => window.addEventListener('resize', syncDesktopBreakpoint))
onBeforeUnmount(() => window.removeEventListener('resize', syncDesktopBreakpoint))
</script>

<template>
  <div class="h-screen flex flex-col bg-bg overflow-hidden">
    <div class="flex-1 flex min-h-0">
      <!--
        One persistent sidebar keeps runtime slots alive across responsive
        breakpoints; mobile changes its presentation into a drawer instead of
        mounting another copy.
      -->
      <aside
        v-show="isDesktop || sidebarOpen"
        class="w-[min(100vw,var(--layout-sidebar))] bg-bg flex flex-col min-h-0"
        :class="isDesktop
          ? 'shrink-0 border-r border-line'
          : 'fixed top-[env(safe-area-inset-top)] bottom-0 left-0 z-[var(--z-drawer)] border-r border-line shadow-float'"
      >
        <slot name="sidebar" :docked="isDesktop" />
      </aside>

      <main class="flex-1 flex flex-col min-w-0 min-h-0">
        <div class="shrink-0">
          <slot name="topbar" />
        </div>
        <div class="flex-1 min-h-0 overflow-hidden flex flex-col">
          <slot name="content" />
        </div>
        <slot name="composer" />
      </main>
    </div>

    <!-- 移动侧栏遮罩 + 抽屉 -->
    <div
      v-if="!isDesktop && sidebarOpen"
      class="fixed inset-0 z-[var(--z-overlay)] bg-ink/30"
      @click="$emit('update:sidebarOpen', false)"
    ></div>
    <!-- Inspector：覆盖式；宽 --layout-inspector（略窄于功能抽屉） -->
    <div
      v-if="inspectorOpen"
      class="fixed inset-0 z-[var(--z-overlay)] bg-ink/25"
      @click="$emit('update:inspectorOpen', false)"
    ></div>
    <div
      v-if="inspectorOpen"
      class="fixed top-[env(safe-area-inset-top)] bottom-0 right-0 z-[var(--z-drawer)] w-[min(100vw,var(--layout-inspector))] bg-surface border-l border-line shadow-float flex flex-col"
    >
      <slot name="inspector" />
    </div>

    <!-- 功能面板 / 隐藏 runtime（PluginHost、MvuJsRuntime 等） -->
    <slot name="panels" />
  </div>
</template>
