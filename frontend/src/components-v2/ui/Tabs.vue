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
  return idx >= 0 ? idx : 0
})

function handleChange(index) {
  const tab = props.tabs[index]
  if (tab) emit('update:modelValue', tab.key)
}

const tabClass = computed(() => (selected) => {
  const base =
    'px-3 py-2 text-sm border-b-2 transition-colors duration-150 select-none'
  return selected
    ? `${base} text-accent border-accent`
    : `${base} text-ink-soft border-transparent hover:text-ink`
})
</script>

<template>
  <TabGroup
    :selected-index="selectedIndex"
    @change="handleChange"
  >
    <TabList class="flex gap-1 border-b border-line">
      <Tab
        v-for="tab in tabs"
        :key="tab.key"
        v-slot="{ selected }"
        :class="tabClass(selected)"
      >
        {{ tab.label }}
      </Tab>
    </TabList>

    <TabPanel class="pt-3 focus:outline-none">
      <slot />
    </TabPanel>
  </TabGroup>
</template>
