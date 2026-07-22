<script setup>
import { computed } from 'vue'
import { TabGroup, TabList, Tab } from '@headlessui/vue'

const props = defineProps({
  tabs: { type: Array, default: () => [] }, // [{ key, label }]
  modelValue: { type: [String, Number], default: null },
})
const emit = defineEmits(['update:modelValue'])

// 当前激活索引（modelValue 是 tab 的 key，需映射为索引）
const selectedIndex = computed(() => {
  const idx = props.tabs.findIndex((t) => t.key === props.modelValue)
  return idx >= 0 ? idx : 0
})

function handleChange(index) {
  const tab = props.tabs[index]
  if (tab) emit('update:modelValue', tab.key)
}

const tabClass = (selected) => {
  const base =
    'px-2.5 sm:px-3 py-2 -mb-px text-[13px] sm:text-sm border-b-2 transition-colors duration-150 select-none whitespace-nowrap shrink-0'
  return selected
    ? `${base} text-accent-bright border-accent`
    : `${base} text-ink-soft border-transparent hover:text-ink`
}
</script>

<template>
  <TabGroup
    :selected-index="selectedIndex"
    @change="handleChange"
  >
    <!-- 窄抽屉内横向滚动，标签不换行挤成「插件 / 事 / 件」 -->
    <TabList class="flex gap-0.5 border-b border-line overflow-x-auto min-w-0">
      <Tab
        v-for="tab in tabs"
        :key="tab.key"
        v-slot="{ selected }"
        as="template"
      >
        <button type="button" :class="tabClass(selected)">
          {{ tab.label }}
        </button>
      </Tab>
    </TabList>

    <div class="pt-3 focus:outline-none min-w-0">
      <slot />
    </div>
  </TabGroup>
</template>
