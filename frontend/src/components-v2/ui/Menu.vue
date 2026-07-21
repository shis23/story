<script setup>
import {
  Menu,
  MenuButton,
  MenuItems,
  MenuItem,
} from '@headlessui/vue'

defineProps({
  label: { type: String, default: '' },
})
defineEmits(['select'])
</script>

<template>
  <Menu as="div" class="relative inline-block">
    <!-- 触发器：优先用 trigger slot，否则回退到默认 label 按钮 -->
    <MenuButton
      v-slot="{ open }"
      as="template"
    >
      <slot name="trigger" :open="open">
        <button
          type="button"
          class="inline-flex items-center justify-center gap-1.5 px-3.5 py-1.5 text-sm rounded-md bg-surface text-ink border border-line shadow-card hover:border-accent-border transition-colors duration-150 select-none"
        >
          {{ label }}
          <svg
            class="text-ink-soft transition-transform duration-150"
            :class="open ? 'rotate-180' : ''"
            width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"
          >
            <path d="M6 9l6 6 6-6" />
          </svg>
        </button>
      </slot>
    </MenuButton>

    <!-- 下拉面板（进出动画）：Vue 原生 Transition，MenuItems 关闭时自行卸载 -->
    <Transition
      enter-active-class="transition ease-[cubic-bezier(0.22,1,0.36,1)] duration-150"
      enter-from-class="opacity-0 -translate-y-1"
      enter-to-class="opacity-100 translate-y-0"
      leave-active-class="transition ease-[cubic-bezier(0.22,1,0.36,1)] duration-100"
      leave-from-class="opacity-100 translate-y-0"
      leave-to-class="opacity-0 -translate-y-1"
    >
      <MenuItems
        class="absolute right-0 z-[var(--z-overlay)] mt-1 w-48 bg-surface rounded-lg shadow-rise border border-line py-1 focus:outline-none"
      >
        <!-- 默认 slot：渲染 MenuItem 列表，每项可拿到 active 与 select(key) -->
        <slot>
          <!-- 未提供 slot 时，回退渲染 label-only 菜单项 -->
          <MenuItem v-slot="{ active }">
            <button
              type="button"
              :class="[
                'w-full text-left px-3 py-2 text-sm cursor-pointer transition-colors',
                active ? 'bg-surface-2 text-ink' : 'text-ink',
              ]"
            >
              {{ label }}
            </button>
          </MenuItem>
        </slot>
      </MenuItems>
    </Transition>
  </Menu>
</template>
