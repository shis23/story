<script setup>
/**
 * MVU 状态栏（v2 迁移，对应 src/components/MvuStatusBar.vue）
 *
 * 接收 mvuState（声明式 ui_bindings + 变量值），按 BindingDisplay 原生画：
 *   - bar：进度条（血条），max 是满值
 *   - text：纯文本
 *   - tag：标签（状态 buff）
 *   - icon：图标映射
 *
 * 零 JS：纯数据绑定，不跑卡的脚本。复用 utils/mvuStatusBarModel.js。
 */
import { computed } from 'vue'
import {
  getMvuValue,
  hasMvuFallbackWarning,
  mvuBarColor,
  mvuBarPercent,
  mvuDisplayValue,
  mvuIconFor,
  toMvuNumber,
} from '../../utils/mvuStatusBarModel.js'

const props = defineProps({
  /**
   * 状态栏渲染所需的一切数据，形如：
   * {
   *   uiBindings: [{ element, variable_key, display: { kind, max, mapping } }, ...],
   *   variables:  [{ key, value }, ...],
   *   fallbackCount: number,
   * }
   */
  mvuState: { type: Object, default: () => ({}) },
})

// 兼容字段（缺省时退回空），保证渲染稳定
const uiBindings = computed(() => props.mvuState?.uiBindings ?? [])
const variables = computed(() => props.mvuState?.variables ?? [])
const fallbackCount = computed(() => props.mvuState?.fallbackCount ?? 0)
// 实例节上下文：模板键（{角色名} 段）按本实例名展开后取值
const instanceName = computed(() => props.mvuState?.instanceName ?? '')

// bar 进度百分比（0-100），max 默认 100
function barPercent(binding) {
  return mvuBarPercent(binding, variables.value, instanceName.value)
}

// bar 颜色：按百分比分级（>50 绿 / 25-50 黄 / <25 红）
function barColor(percent) {
  return mvuBarColor(percent)
}

// 数字化（容错：字符串/数字都转）
function toNum(value) {
  return toMvuNumber(value)
}

function getValue(key) {
  return getMvuValue(variables.value, key, instanceName.value)
}

// icon 映射查找
function iconFor(binding) {
  return mvuIconFor(binding, variables.value, '·', instanceName.value)
}

function displayValue(key) {
  return mvuDisplayValue(variables.value, key, '—', instanceName.value)
}

function showFallbackWarning() {
  return hasMvuFallbackWarning(fallbackCount.value)
}
</script>

<template>
  <div
    v-if="uiBindings.length > 0 || showFallbackWarning()"
    class="bg-surface/60 rounded-lg border border-line p-2.5 space-y-1.5"
  >
    <div class="flex items-center justify-between">
      <span class="text-[10px] font-medium text-ink-soft uppercase tracking-wide">状态栏</span>
      <span
        v-if="showFallbackWarning()"
        class="text-[10px] text-warn"
        title="此卡含未翻译 JS，完整渲染需共享 WebView（下一轮实现）"
      >⚠ {{ fallbackCount }} 项需 WebView</span>
    </div>

    <!-- 各绑定按 display 类型渲染 -->
    <div class="space-y-1">
      <div
        v-for="b in uiBindings"
        :key="b.element"
        class="flex items-center gap-2 text-xs"
      >
        <!-- bar：进度条 -->
        <template v-if="b.display?.kind === 'bar'">
          <span class="text-ink-soft w-14 sm:w-16 shrink-0 truncate" :title="b.variable_key">{{ b.variable_key }}</span>
          <div class="flex-1 h-2 bg-bg rounded-full overflow-hidden">
            <div
              class="h-full transition-all rounded-full"
              :class="barColor(barPercent(b))"
              :style="{ width: barPercent(b) + '%' }"
            ></div>
          </div>
          <span class="text-ink w-12 text-right shrink-0 tabular-nums">
            {{ toNum(getValue(b.variable_key)) }}/{{ b.display?.max ?? 100 }}
          </span>
        </template>

        <!-- text：纯文本 -->
        <template v-else-if="b.display?.kind === 'text'">
          <span class="text-ink-soft w-14 sm:w-16 shrink-0 truncate">{{ b.variable_key }}</span>
          <span class="text-ink flex-1 truncate">{{ displayValue(b.variable_key) }}</span>
        </template>

        <!-- tag：标签 -->
        <template v-else-if="b.display?.kind === 'tag'">
          <span class="text-ink-soft w-14 sm:w-16 shrink-0 truncate">{{ b.variable_key }}</span>
          <span class="px-1.5 py-0.5 rounded-full bg-accent-soft text-accent text-[10px]">
            {{ displayValue(b.variable_key) }}
          </span>
        </template>

        <!-- icon：图标映射 -->
        <template v-else-if="b.display?.kind === 'icon'">
          <span class="text-ink-soft w-14 sm:w-16 shrink-0 truncate">{{ b.variable_key }}</span>
          <span class="text-base">{{ iconFor(b) }}</span>
        </template>

        <!-- 兜底（未知 display） -->
        <template v-else>
          <span class="text-ink-soft w-14 sm:w-16 shrink-0 truncate">{{ b.variable_key }}</span>
          <span class="text-ink flex-1 truncate">{{ displayValue(b.variable_key) }}</span>
        </template>
      </div>
    </div>
  </div>
</template>
