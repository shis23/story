<script setup>
/**
 * 写作面 MVU 原生状态面板（WritingScreen after-messages 槽）
 *
 * 每个绑定到卡 definition 的实例一节：实例名 + MvuStatusBar（ui_bindings 原生渲染）；
 * 下方按卡的 InteractionMapping 渲染原生交互按钮，点击 emit('interact', mapping)。
 * 纯 props 渲染，零 tauri 调用；数据与分发由 composables/useMvuStatusPanel 承担。
 */
import MvuStatusBar from './MvuStatusBar.vue'

defineProps({
  /** [{ instanceId, instanceName, mvuState }] */
  sections: { type: Array, default: () => [] },
  /** InteractionMapping[]：[{ element_label, actions }] */
  interactions: { type: Array, default: () => [] },
  /** 分发中 / 写作中：按钮禁用 */
  busy: { type: Boolean, default: false },
})

const emit = defineEmits(['interact'])
</script>

<template>
  <div
    v-if="sections.length || interactions.length"
    class="space-y-2"
    data-testid="mvu-status-panel"
  >
    <div v-for="s in sections" :key="s.instanceId" class="space-y-1">
      <div v-if="s.instanceName" class="text-[10px] text-ink-soft px-0.5">{{ s.instanceName }}</div>
      <MvuStatusBar :mvu-state="s.mvuState" />
    </div>

    <div
      v-if="interactions.length"
      class="flex flex-wrap gap-1.5"
      data-testid="mvu-interaction-bar"
    >
      <button
        v-for="(mapping, i) in interactions"
        :key="`${mapping.element_label}-${i}`"
        type="button"
        class="px-2 py-1 rounded-md border border-line bg-surface/60 text-xs text-ink hover:bg-surface disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
        :disabled="busy"
        @click="emit('interact', mapping)"
      >
        {{ mapping.element_label }}
      </button>
    </div>
  </div>
</template>
