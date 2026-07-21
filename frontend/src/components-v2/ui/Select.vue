<script setup>
import { computed } from 'vue'
import {
  Listbox,
  ListboxButton,
  ListboxOptions,
  ListboxOption,
} from '@headlessui/vue'

const props = defineProps({
  modelValue: { type: [String, Number, null], default: null },
  options: { type: Array, default: () => [] },
  placeholder: { type: String, default: '' },
  disabled: { type: Boolean, default: false },
  loading: { type: Boolean, default: false },
})
const emit = defineEmits(['update:modelValue'])

const normalized = computed(() =>
  props.options.map((o) =>
    typeof o === 'string' ? { label: o, value: o } : o
  )
)

const isDisabled = computed(() => props.disabled || props.loading)

const selectedLabel = computed(() => {
  const hit = normalized.value.find((o) => o.value === props.modelValue)
  return hit ? hit.label : null
})
</script>

<template>
  <Listbox
    :model-value="modelValue"
    :disabled="isDisabled"
    @update:model-value="emit('update:modelValue', $event)"
  >
    <div class="relative">
      <!-- open 态用 aria-expanded 驱动（headlessui 自动维护），slot prop 不跨元素作用域 -->
      <ListboxButton
        class="group w-full flex items-center justify-between gap-2 bg-surface border border-line rounded-md px-3 py-1.5 text-sm text-left transition-colors duration-150 outline-none hover:border-accent-border aria-[expanded=true]:border-accent-border disabled:bg-surface-2 disabled:cursor-not-allowed disabled:hover:border-line"
        :class="isDisabled ? 'text-ink-faint' : selectedLabel === null ? 'text-ink-faint' : 'text-ink'"
      >
        <span class="truncate">
          {{ loading ? '加载中…' : (selectedLabel ?? placeholder) }}
        </span>
        <svg
          class="shrink-0 text-ink-soft transition-transform duration-150 group-aria-[expanded=true]:rotate-180"
          width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"
          aria-hidden="true"
        >
          <path d="M6 9l6 6 6-6" />
        </svg>
      </ListboxButton>

      <!-- Vue 原生 Transition：ListboxOptions 关闭时自行卸载，leave 动画由 Transition 接管 -->
      <Transition
        enter-active-class="transition ease-soft duration-150"
        enter-from-class="opacity-0 -translate-y-1"
        enter-to-class="opacity-100 translate-y-0"
        leave-active-class="transition ease-soft duration-100"
        leave-from-class="opacity-100 translate-y-0"
        leave-to-class="opacity-0 -translate-y-1"
      >
        <ListboxOptions
          class="absolute left-0 z-[var(--z-overlay)] mt-1 w-full max-h-60 overflow-auto bg-surface border border-line rounded-lg shadow-rise py-1 focus:outline-none"
        >
          <ListboxOption
            v-for="opt in normalized"
            :key="opt.value"
            :value="opt.value"
            v-slot="{ active, selected }"
          >
            <li
              class="flex items-center justify-between gap-2 px-3 py-1.5 text-sm cursor-pointer select-none transition-colors duration-100"
              :class="[
                selected
                  ? 'bg-accent-soft text-accent-bright font-medium'
                  : active
                    ? 'bg-surface-2 text-ink'
                    : 'text-ink',
              ]"
            >
              <span class="truncate">{{ opt.label }}</span>
              <svg
                v-if="selected"
                class="shrink-0 text-accent"
                width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"
                aria-hidden="true"
              >
                <path d="M5 12.5l4.5 4.5L19 7" />
              </svg>
            </li>
          </ListboxOption>
        </ListboxOptions>
      </Transition>
    </div>
  </Listbox>
</template>

