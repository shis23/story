<script setup>
/**
 * AppFrame — 应用壳（重设计 · 纯展示）。
 * 侧栏 / 顶栏 / 主区 / 调试抽屉均由 slots 或子纯展示组件填充。
 * 不读 store；展开态经 props 注入。
 */
defineProps({
  sidebarOpen: { type: Boolean, default: false },
  inspectorOpen: { type: Boolean, default: false },
  powerMode: { type: Boolean, default: false },
})

defineEmits(['update:sidebarOpen', 'update:inspectorOpen'])
</script>

<template>
  <div class="h-screen flex flex-col bg-bg overflow-hidden">
    <div class="flex-1 flex min-h-0">
      <!-- 桌面侧栏 -->
      <aside class="hidden lg:flex w-[232px] shrink-0 border-r border-line bg-bg flex-col">
        <slot name="sidebar" />
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

      <aside v-if="powerMode && inspectorOpen" class="hidden lg:block w-[320px] shrink-0 border-l border-line bg-surface">
        <slot name="inspector" />
      </aside>
    </div>

    <!-- 移动侧栏遮罩 -->
    <div
      v-if="sidebarOpen"
      class="lg:hidden fixed inset-0 z-[var(--z-overlay)] bg-ink/30"
      @click="$emit('update:sidebarOpen', false)"
    ></div>
    <div
      v-if="sidebarOpen"
      class="lg:hidden fixed inset-y-0 left-0 z-[var(--z-drawer)] w-[232px] bg-bg border-r border-line shadow-float"
    >
      <slot name="sidebar" />
    </div>

    <slot name="panels" />
  </div>
</template>
