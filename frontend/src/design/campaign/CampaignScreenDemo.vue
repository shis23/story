<script setup>
/**
 * CampaignScreenDemo — 设计预览（#design-campaign）。
 * 多状态 fixture：活动列表、实例/知识/任务/总结 tab、空态。
 * 不接 store；仅展示 design 层信息架构。
 */
import { ref, computed } from 'vue'
import CampaignScreen from './CampaignScreen.vue'

const modes = [
  { key: 'filled', label: '有数据' },
  { key: 'empty', label: '空列表' },
]
const current = ref('filled')
const detailTab = ref('instances')
const selectedId = ref('camp-1')
const mode = ref('manage')

const campaignsFilled = [
  { id: 'camp-1', name: '风起之地', instance_count: 5, updated_at: '2026-07-20T13:20:00Z', story_clock: '第 12 轮' },
  { id: 'camp-2', name: '白夜行歌', instance_count: 3, updated_at: '2026-07-18T09:00:00Z', story_clock: '第 4 轮' },
  { id: 'camp-3', name: '长风渡', instance_count: 8, updated_at: '2026-07-15T18:00:00Z' },
]

const dataByCamp = {
  'camp-1': {
    instances: [
      { id: 'i1', name: '叙述者·苏澜', role_type: '旁白', summary: '擅长铺陈叙述与氛围描写，语调克制。' },
      { id: 'i2', name: '艾琳', role_type: '主角', summary: '旅者 · 寻找失踪兄长与真相。' },
      { id: 'i3', name: '卡列尔', role_type: '配角', summary: '守夜人 · 沉默寡言，熟悉城墙外的风声。' },
      { id: 'i4', name: '老馆长', role_type: '配角', summary: '档案馆唯一钥匙的持有者。' },
    ],
    knowledge: [
      { id: 'k1', title: '世界观设定', content: '双月历法；风能驱动的城邦联邦。' },
      { id: 'k2', title: '地理与势力', content: '北境哨站、中央档案馆、南港商会。' },
      { id: 'k3', title: '私密：艾琳家史', content: '兄长失踪前最后一封信藏在第 7 层。' },
    ],
    tasks: [
      { id: 't1', title: '第一章 · 风的低语', status: '已完成', description: '建立城邦与艾琳入城动机。' },
      { id: 't2', title: '序章 · 黑夜之前', status: '已完成' },
      { id: 't3', title: '调查档案馆第 7 层', status: '进行中', description: '需卡列尔协助通过夜禁。' },
    ],
    summaries: [
      { id: 's1', round: 11, summary: '艾琳在雨夜抵达北门，卡列尔未放行，却留下一句「明天日落后来」。', created_at: '2026-07-20T12:00:00Z' },
      { id: 's2', round: 12, summary: '苏澜以旁白点出双月将合，城邦律法中这夜禁止打开档案库。', created_at: '2026-07-20T13:10:00Z' },
    ],
  },
  'camp-2': {
    instances: [
      { id: 'j1', name: '白夜', role_type: '主角', summary: '歌者' },
      { id: 'j2', name: '行歌', role_type: '配角', summary: '旧识' },
    ],
    knowledge: [{ id: 'k', title: '曲谱残页', content: '三节佚失的副歌。' }],
    tasks: [{ id: 't', title: '完成副歌', status: '待办' }],
    summaries: [],
  },
  'camp-3': {
    instances: [],
    knowledge: [],
    tasks: [],
    summaries: [],
  },
}

const campaigns = computed(() => (current.value === 'empty' ? [] : campaignsFilled))
const selected = computed(() => campaigns.value.find((c) => c.id === selectedId.value) || null)
const bag = computed(() => dataByCamp[selectedId.value] || { instances: [], knowledge: [], tasks: [], summaries: [] })
</script>

<template>
  <div class="h-screen flex flex-col bg-bg">
    <div class="shrink-0 flex items-center gap-2 px-4 py-1.5 border-b border-line bg-surface-2/70 z-10">
      <span class="text-[11px] text-ink-faint mr-1">设计预览 · 活动管理</span>
      <button
        v-for="s in modes"
        :key="s.key"
        type="button"
        class="min-h-6 px-2 rounded text-[11px] transition-colors"
        :class="current === s.key ? 'bg-accent-soft text-accent-bright font-medium' : 'text-ink-soft hover:bg-surface-2'"
        @click="current = s.key; if (s.key === 'filled' && !selectedId) selectedId = 'camp-1'"
      >{{ s.label }}</button>
      <a href="#" class="ml-auto text-[11px] text-ink-faint hover:text-ink-soft">← 返回应用</a>
    </div>
    <div class="flex-1 min-h-0">
      <CampaignScreen
        :mode="mode"
        :campaigns="campaigns"
        :selected-campaign-id="selected?.id || null"
        :selected-campaign="selected"
        :detail-tab="detailTab"
        :instances="bag.instances"
        :knowledge="bag.knowledge"
        :tasks="bag.tasks"
        :summaries="bag.summaries"
        @change-tab="detailTab = $event"
        @change-mode="mode = $event"
        @select-campaign="(c) => { selectedId = c.id; detailTab = 'instances' }"
        @close="location.hash = ''"
        @new-campaign="() => {}"
        @set-active="() => {}"
        @refresh="() => {}"
      />
    </div>
  </div>
</template>
