<script setup>
import { currentCharacter } from '../mock.js'
import { useTheme } from '../useTheme.js'
defineProps({
  powerMode: { type: Boolean, default: false },
  activeCharName: { type: String, default: null },
})
const emit = defineEmits(['toggle-power'])
const { theme, toggle: toggleTheme } = useTheme()
</script>

<template>
  <header class="sticky top-0 z-20 backdrop-blur-md bg-surface/80 border-b border-line">
    <div class="px-4 py-3 flex items-center gap-2">
      <!-- 角色头像与名字 -->
      <div class="flex items-center gap-3 flex-1 min-w-0">
        <div class="w-10 h-10 rounded-full bg-accent-soft flex items-center justify-center text-xl shrink-0">
          {{ activeCharName ? activeCharName.charAt(0) : currentCharacter.avatar }}
        </div>
        <div class="min-w-0">
          <div class="font-medium text-ink truncate">{{ activeCharName || currentCharacter.name }}</div>
          <div class="text-xs text-ink-soft truncate">{{ activeCharName ? '已选择' : currentCharacter.tagline }}</div>
        </div>
      </div>

      <!-- 自定义操作按钮（导入等） -->
      <slot name="actions" />

      <!-- 主题切换 -->
      <button
        @click="toggleTheme"
        class="w-8 h-8 flex items-center justify-center rounded-full text-ink-soft hover:bg-accent-soft transition-colors shrink-0"
        :title="theme === 'dark' ? '切到亮色' : '切到暗色'"
      >
        <span v-if="theme === 'dark'">☀️</span>
        <span v-else>🌙</span>
      </button>

      <!-- 高玩模式开关 -->
      <button
        @click="emit('toggle-power')"
        class="px-3 py-1.5 rounded-full text-xs font-medium transition-all shrink-0"
        :class="powerMode
          ? 'bg-accent text-white'
          : 'bg-accent-soft text-accent'"
      >
        {{ powerMode ? '⚡ 高玩' : '普通' }}
      </button>
    </div>
  </header>
</template>
