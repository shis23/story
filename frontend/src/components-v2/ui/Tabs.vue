<script setup>
import { computed } from 'vue'
import { TabGroup, TabList, Tab, TabPanel } from '@headlessui/vue'

const props = defineProps({
  tabs: { type: Array, default: () => [] }, // [{ key, label }]
  modelValue: { type: [String, Number], default: null },
})
const emit = defineEmits(['update:modelValue'])

// 当前激活索引（modelValue 是 tab 的 key，需映射为索引）
const selectedIndex = computed(() => {
  const idx = props.tabs.findIndex((t) => t.key === props.modelValue)
  if (idx >= 0) return idx
  // F-21：modelValue 非法时静默回落第 0 项会让"选中态"与调用方的
  // `v-if="activeTab === 'x'"` 内容不一致，且无任何信号。
  if (import.meta.env?.DEV && props.tabs.length > 0) {
    console.warn(`[Tabs] modelValue "${props.modelValue}" 不在 tabs 中，已回退到第 0 项`)
  }
  return 0
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

    <!-- F-37：内容包在 TabPanel 里，Tab 的 aria-controls 才有真实指向 -->
    <TabPanel class="pt-3 focus:outline-none min-w-0">
      <slot />
    </TabPanel>
  </TabGroup>
</template>
