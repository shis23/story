<script setup>
/**
 * ST 兼容性徽章（新增小组件）
 *
 * 标识某项 SillyTavern 兼容特性当前是否被 StoryForge 原生支持。
 * - supported=true：Badge 用 ok variant 显示「已支持 / 兼容」。
 * - supported=false：Badge 用 warn variant 显示「降级 / WebView 渲染」。
 *
 * 典型 feature 值：'regex' / 'html' / 'tavernHelper' / 'slash' / 'promptHook'
 * 其余未知 feature 走 default 标签，避免破坏性显示。
 */
import { computed } from 'vue'
import Badge from '../ui/Badge.vue'

const props = defineProps({
  feature: { type: String, default: '' },
  supported: { type: Boolean, default: false },
})

// feature → 中文标签映射
const FEATURE_LABELS = {
  regex: '正则脚本',
  html: 'HTML 渲染',
  tavernHelper: 'TavernHelper',
  slash: 'Slash 命令',
  promptHook: 'Prompt Hook',
}

// feature → 支持态/降级态的简短描述
const SUPPORTED_HINT = {
  regex: '原生执行',
  html: '原生渲染',
  tavernHelper: '原生事件',
  slash: '原生路由',
  promptHook: '原生回调',
}
const DEGRADED_HINT = {
  regex: 'WebView 兜底',
  html: 'WebView 兜底',
  tavernHelper: '不可用',
  slash: '不可用',
  promptHook: '不可用',
}

const label = computed(() => FEATURE_LABELS[props.feature] || props.feature || '未知特性')

const variant = computed(() => (props.supported ? 'ok' : 'warn'))

const hint = computed(() =>
  props.supported
    ? SUPPORTED_HINT[props.feature] || '已支持'
    : DEGRADED_HINT[props.feature] || '降级'
)
</script>

<template>
  <Badge :variant="variant" size="sm">
    {{ label }}：{{ supported ? '✓' : '⚠' }} {{ hint }}
  </Badge>
</template>
