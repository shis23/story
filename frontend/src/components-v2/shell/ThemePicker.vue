<script setup>
import { useId } from 'vue'
import { Check, Moon, Sun } from '@lucide/vue'
import { useTheme } from '../../useTheme.js'

const { theme, palette, palettes, toggle, setPalette } = useTheme()
const groupName = `palette-${useId()}`
</script>

<template>
  <div class="theme-picker">
    <fieldset class="min-w-0">
      <legend class="px-2.5 text-[11px] text-ink-faint">配色</legend>
      <div class="grid grid-cols-4 gap-1">
        <label v-for="option in palettes" :key="option.id" class="theme-option" :title="option.label">
          <input
            type="radio"
            :name="groupName"
            :value="option.id"
            :aria-label="option.label"
            :checked="palette === option.id"
            @change="setPalette(option.id)"
          >
          <span class="theme-swatch" :style="{ backgroundColor: `var(--palette-${option.id})` }" aria-hidden="true">
            <Check v-if="palette === option.id" :size="15" :stroke-width="2.5" />
          </span>
          <span class="text-[11px] leading-4" :class="palette === option.id ? 'text-ink font-medium' : 'text-ink-soft'">
            {{ option.label }}
          </span>
        </label>
      </div>
    </fieldset>
    <button
      type="button"
      role="switch"
      aria-label="夜读模式"
      :aria-checked="theme === 'dark'"
      :title="theme === 'dark' ? '切换为浅色' : '夜读模式'"
      class="theme-mode"
      @click="toggle"
    >
      <component :is="theme === 'dark' ? Sun : Moon" :size="16" aria-hidden="true" />
      <span class="flex-1 text-left">{{ theme === 'dark' ? '切换为浅色' : '夜读模式' }}</span>
      <span class="theme-switch" aria-hidden="true"><span /></span>
    </button>
  </div>
</template>

<style scoped>
.theme-option {
  position: relative;
  display: grid;
  justify-items: center;
  align-content: center;
  min-height: 64px;
  gap: 5px;
  cursor: pointer;
}
.theme-option input {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  margin: 0;
  opacity: 0;
  cursor: pointer;
}
.theme-option > span { pointer-events: none; }
.theme-swatch {
  display: grid;
  place-items: center;
  width: 26px;
  height: 26px;
  border-radius: 50%;
  color: white;
  transition: box-shadow var(--motion-fast) ease, transform var(--motion-fast) var(--ease-soft);
}
.theme-option input:checked + .theme-swatch {
  box-shadow: 0 0 0 3px var(--color-bg), 0 0 0 4px var(--color-ink-faint);
}
.theme-option input:focus-visible + .theme-swatch {
  outline: 2px solid var(--color-accent-bright);
  outline-offset: 5px;
}
.theme-option:hover .theme-swatch { transform: translateY(-1px); }
.theme-option:active .theme-swatch { transform: scale(0.94); }
.theme-mode {
  display: flex;
  align-items: center;
  width: 100%;
  min-height: 44px;
  gap: 10px;
  padding: 0 10px;
  border-radius: var(--radius-md);
  color: var(--color-ink-soft);
  font-size: 13px;
  transition: color var(--motion-fast) ease, background-color var(--motion-fast) ease;
}
.theme-mode:hover { color: var(--color-ink); background: var(--color-surface-2); }
.theme-switch {
  display: flex;
  align-items: center;
  flex-shrink: 0;
  width: 30px;
  height: 18px;
  padding: 3px;
  border-radius: 9px;
  background: var(--color-ink-faint);
  transition: background-color var(--motion-fast) ease;
}
.theme-switch > span {
  width: 12px;
  height: 12px;
  border-radius: 50%;
  background: white;
  transition: transform var(--motion-fast) var(--ease-soft);
}
.theme-mode[aria-checked='true'] .theme-switch { background: var(--color-accent); }
.theme-mode[aria-checked='true'] .theme-switch > span {
  transform: translateX(12px);
  background: var(--color-on-accent, white);
}
@media (prefers-reduced-motion: reduce) {
  .theme-option:hover .theme-swatch,
  .theme-option:active .theme-swatch { transform: none; }
}
</style>
