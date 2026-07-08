<script setup>
import {
  Menu,
  MenuButton,
  MenuItems,
  MenuItem,
  TransitionChild,
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
          class="inline-flex items-center justify-center gap-1.5 px-3.5 py-1.5 text-sm rounded-lg bg-surface-2 text-ink border border-line hover:border-accent-border transition-colors duration-150 select-none"
        >
          {{ label }}
          <span class="text-ink-soft text-xs">▾</span>
        </button>
      </slot>
    </MenuButton>

    <!-- 下拉面板（进出动画） -->
    <TransitionChild
      as="template"
      enter="transition ease-[cubic-bezier(0.22,1,0.36,1)] duration-150"
      enterFrom="opacity-0 -translate-y-1"
      enterTo="opacity-100 translate-y-0"
      leave="transition ease-[cubic-bezier(0.22,1,0.36,1)] duration-100"
      leaveFrom="opacity-100 translate-y-0"
      leaveTo="opacity-0 -translate-y-1"
    >
      <MenuItems
        class="absolute right-0 mt-1 w-48 bg-surface rounded-lg shadow-float border border-line py-1 focus:outline-none"
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
    </TransitionChild>
  </Menu>
</template>
