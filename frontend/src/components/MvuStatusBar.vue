<script setup>
/**
 * MVU 状态栏原生渲染（对应设计 §19.3 / §8.2「渲染」路）
 *
 * 接收 ui_bindings（声明式绑定）+ variables（当前变量值），按 BindingDisplay 原生画：
 *   - bar：进度条（血条），max 是满值
 *   - text：纯文本
 *   - tag：标签（状态 buff）
 *   - icon：图标映射
 *
 * 零 JS：纯数据绑定，不跑卡的脚本。
 */
import { computed } from 'vue'

const props = defineProps({
  /** MvuTranslation.ui_bindings */
  uiBindings: { type: Array, default: () => [] },
  /**
   * 变量值，形如 [{ key: 'hp', value: 80 }, ...]
   * 来源：getCharacterVariables / getCampaignVariables
   */
  variables: { type: Array, default: () => [] },
  /** fallback_fragments 数量（>0 时显示提示） */
  fallbackCount: { type: Number, default: 0 },
})

// 变量值查表：key -> value
function getValue(key) {
  const v = props.variables.find(x => x.key === key)
  return v ? v.value : null
}

// 数字化（容错：字符串/数字都转）
function toNum(v) {
  if (v == null) return 0
  const n = Number(v)
  return isNaN(n) ? 0 : n
}

// bar 进度百分比（0-100），max 默认 100
function barPercent(binding) {
  const val = toNum(getValue(binding.variable_key))
  const max = binding.display?.max ?? 100
  if (max <= 0) return 0
  return Math.max(0, Math.min(100, (val / max) * 100))
}

// bar 颜色：按百分比分级（>50 绿 / 25-50 黄 / <25 红）
function barColor(percent) {
  if (percent > 50) return 'bg-ok'
  if (percent > 25) return 'bg-warn'
  return 'bg-error'
}

// icon 映射查找
function iconFor(binding) {
  const val = getValue(binding.variable_key)
  const mapping = binding.display?.mapping || {}
  const key = String(val)
  return mapping[key] || mapping['_default'] || '·'
}
</script>

<template>
  <div v-if="uiBindings.length > 0 || fallbackCount > 0" class="mvu-status-bar bg-surface/60 rounded-lg border border-line p-2.5 space-y-1.5">
    <div class="flex items-center justify-between">
      <span class="text-[10px] font-medium text-ink-soft uppercase tracking-wide">状态栏</span>
      <span v-if="fallbackCount > 0" class="text-[10px] text-warn" title="此卡含未翻译 JS，完整渲染需共享 WebView（下一轮实现）">
        ⚠ {{ fallbackCount }} 项需 WebView
      </span>
    </div>

    <!-- 各绑定按 display 类型渲染 -->
    <div class="space-y-1">
      <div
        v-for="b in uiBindings" :key="b.element"
        class="flex items-center gap-2 text-xs"
      >
        <!-- bar：进度条 -->
        <template v-if="b.display?.kind === 'bar'">
          <span class="text-ink-soft w-16 shrink-0 truncate" :title="b.variable_key">{{ b.variable_key }}</span>
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
          <span class="text-ink-soft w-16 shrink-0 truncate">{{ b.variable_key }}</span>
          <span class="text-ink flex-1 truncate">{{ getValue(b.variable_key) ?? '—' }}</span>
        </template>

        <!-- tag：标签 -->
        <template v-else-if="b.display?.kind === 'tag'">
          <span class="text-ink-soft w-16 shrink-0 truncate">{{ b.variable_key }}</span>
          <span class="px-1.5 py-0.5 rounded-full bg-accent-soft text-accent text-[10px]">
            {{ getValue(b.variable_key) ?? '—' }}
          </span>
        </template>

        <!-- icon：图标映射 -->
        <template v-else-if="b.display?.kind === 'icon'">
          <span class="text-ink-soft w-16 shrink-0 truncate">{{ b.variable_key }}</span>
          <span class="text-base">{{ iconFor(b) }}</span>
        </template>

        <!-- 兜底（未知 display） -->
        <template v-else>
          <span class="text-ink-soft w-16 shrink-0 truncate">{{ b.variable_key }}</span>
          <span class="text-ink flex-1 truncate">{{ getValue(b.variable_key) ?? '—' }}</span>
        </template>
      </div>
    </div>
  </div>
</template>
