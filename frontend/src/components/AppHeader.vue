<script setup>
import { currentCharacter } from '../mock.js'
import { useTheme } from '../useTheme.js'
defineProps({
  powerMode: { type: Boolean, default: false },
  activeCharName: { type: String, default: null },
  activeCampaignName: { type: String, default: null },
  writingMode: { type: String, default: 'none' }, // 'campaign' | 'legacy' | 'none'
})
const emit = defineEmits(['toggle-power', 'open-campaign', 'open-meta'])
const { theme, toggle: toggleTheme } = useTheme()
</script>

<template>
  <header class="sticky top-0 z-20 backdrop-blur-md bg-surface/80 border-b border-line">
    <div class="px-4 py-3 flex items-center gap-2">
      <!-- 角色/Campaign 头像与名字 -->
      <div class="flex items-center gap-3 flex-1 min-w-0">
        <div class="w-10 h-10 rounded-full flex items-center justify-center text-xl shrink-0"
          :class="writingMode === 'campaign' ? 'bg-green-500/20' : 'bg-accent-soft'">
          {{ writingMode === 'campaign' ? '🎪' : (activeCharName ? activeCharName.charAt(0) : currentCharacter.avatar) }}
        </div>
        <div class="min-w-0">
          <div class="font-medium text-ink truncate">
            {{ writingMode === 'campaign' ? activeCampaignName : (activeCharName || currentCharacter.name) }}
          </div>
          <div class="text-xs text-ink-soft truncate">
            <template v-if="writingMode === 'campaign'">Campaign 写作</template>
            <template v-else-if="writingMode === 'legacy'">已选择 · 兼容模式</template>
            <template v-else>{{ currentCharacter.tagline }}</template>
          </div>
        </div>
      </div>

      <!-- 自定义操作按钮（导入等） -->
      <slot name="actions" />

      <!-- Campaign 管理 -->
      <button
        @click="emit('open-campaign')"
        class="w-8 h-8 flex items-center justify-center rounded-full text-ink-soft hover:bg-accent-soft transition-colors shrink-0"
        title="Campaign 管理"
      >
        🎪
      </button>

      <!-- Meta 配置助手（仅高玩模式可见，P3 新增） -->
      <button
        v-if="powerMode"
        @click="emit('open-meta')"
        class="w-8 h-8 flex items-center justify-center rounded-full text-ink-soft hover:bg-accent-soft transition-colors shrink-0"
        title="Meta 配置助手"
      >
        🔧
      </button>

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
