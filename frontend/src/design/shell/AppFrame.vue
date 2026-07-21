<script setup>
/**
 * AppFrame — 应用壳（重设计 · 纯展示布局）。
 *
 * 对齐 selected 图：
 *   - 左栏 232px 常驻（lg+）；移动端抽屉
 *   - TopBar 52px
 *   - 主区自滚动；Inspector 覆盖式（不挤压写作栏）
 *
 * 不读 store。展开态由 props 注入；导航与面板由 slots 填充。
 * sidebar / inspector 插槽参数：{ docked: boolean }
 */
defineProps({
  sidebarOpen: { type: Boolean, default: false },
  inspectorOpen: { type: Boolean, default: false },
})

defineEmits(['update:sidebarOpen', 'update:inspectorOpen'])
</script>

<template>
  <div class="h-screen flex flex-col bg-bg overflow-hidden">
    <div class="flex-1 flex min-h-0">
      <!-- 桌面侧栏 232px -->
      <aside class="hidden lg:flex w-[232px] shrink-0 border-r border-line bg-bg flex-col min-h-0">
        <slot name="sidebar" :docked="true" />
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
      v-if="sidebarOpen"
      class="lg:hidden fixed inset-0 z-[var(--z-overlay)] bg-ink/30"
      @click="$emit('update:sidebarOpen', false)"
    ></div>
    <div
      v-if="sidebarOpen"
      class="lg:hidden fixed inset-y-0 left-0 z-[var(--z-drawer)] w-[232px] bg-bg border-r border-line shadow-float flex flex-col"
    >
      <slot name="sidebar" :docked="false" />
    </div>

    <!-- Inspector：覆盖式（桌面+移动统一），不挤压写作栏 -->
    <div
      v-if="inspectorOpen"
      class="fixed inset-0 z-[var(--z-overlay)] bg-ink/25"
      @click="$emit('update:inspectorOpen', false)"
    ></div>
    <div
      v-if="inspectorOpen"
      class="fixed inset-y-0 right-0 z-[var(--z-drawer)] w-[min(100%,320px)] bg-surface border-l border-line shadow-float flex flex-col"
    >
      <slot name="inspector" />
    </div>

    <!-- 功能面板 / 隐藏 runtime（PluginHost、MvuJsRuntime 等） -->
    <slot name="panels" />
  </div>
</template>
