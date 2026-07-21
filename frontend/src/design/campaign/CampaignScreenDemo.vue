<script setup>
import { ref } from 'vue'
import CampaignScreen from './CampaignScreen.vue'

const detailTab = ref('instances')
const selectedId = ref('camp-1')

const campaigns = [
  { id: 'camp-1', name: '风起之地', instance_count: 5, updated_at: '2026-07-20T13:20:00Z' },
  { id: 'camp-2', name: '白夜行歌', instance_count: 3, updated_at: '2026-07-18T09:00:00Z' },
]

const selected = campaigns[0]
const instances = [
  { id: 'i1', name: '叙述者·苏澜', role_type: '旁白', summary: '擅长铺陈叙述与氛围描写' },
  { id: 'i2', name: '艾琳', role_type: '主角', summary: '旅者 · 寻找真相' },
  { id: 'i3', name: '卡列尔', role_type: '配角', summary: '守夜人 · 沉默寡言' },
]
const knowledge = [
  { id: 'k1', title: '世界观设定', content: '24 条知识' },
  { id: 'k2', title: '地理与势力', content: '15 条知识' },
]
const tasks = [
  { id: 't1', title: '第一章 · 风的低语', status: '已完成' },
  { id: 't2', title: '序章 · 黑夜之前', status: '已完成' },
]
const summaries = [
  { id: 's1', round: 1, summary: '夜色像一块柔软的绒布，轻轻覆盖住了这座沉睡的城市。', created_at: '2026-07-20T12:00:00Z' },
]
</script>

<template>
  <div class="h-screen">
    <div class="shrink-0 flex items-center gap-2 px-4 py-1.5 border-b border-line bg-surface-2/70">
      <span class="text-[11px] text-ink-faint">设计预览 · 活动管理</span>
      <a href="#" class="ml-auto text-[11px] text-ink-faint hover:text-ink-soft">← 返回应用</a>
    </div>
    <div class="h-[calc(100vh-36px)]">
      <CampaignScreen
        :campaigns="campaigns"
        :selected-campaign-id="selectedId"
        :selected-campaign="selected"
        :detail-tab="detailTab"
        :instances="instances"
        :knowledge="knowledge"
        :tasks="tasks"
        :summaries="summaries"
        @change-tab="detailTab = $event"
        @select-campaign="(c) => { selectedId = c.id }"
        @close="location.hash = ''"
      />
    </div>
  </div>
</template>
