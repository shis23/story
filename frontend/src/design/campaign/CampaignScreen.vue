<script setup>
/**
 * CampaignScreen — 活动管理主屏（重设计 · 纯展示）。
 *
 * 对齐 selected 图④：
 *   - 左：我的活动列表
 *   - 右：当前活动详情 + tabs
 *
 * 不 import store/tauri。
 * 深度详情（MVU 实例编辑等）通过 #detail 插槽注入；未注入时用 props 列表只读展示。
 */
import { computed } from 'vue'
import { Menu, MenuButton, MenuItem, MenuItems } from '@headlessui/vue'
import { Ellipsis, FileDown, FileUp, ImageDown, X } from '@lucide/vue'

const props = defineProps({
  campaigns: { type: Array, default: () => [] },
  selectedCampaignId: { type: String, default: null },
  selectedCampaign: { type: Object, default: null },
  loadingCampaigns: { type: Boolean, default: false },
  detailTab: { type: String, default: 'instances' },
  instances: { type: Array, default: () => [] },
  knowledge: { type: Array, default: () => [] },
  tasks: { type: Array, default: () => [] },
  summaries: { type: Array, default: () => [] },
  loadingDetail: { type: Boolean, default: false },
  exportStatus: { type: String, default: '' },
  importStatus: { type: String, default: '' },
  exporting: { type: Boolean, default: false },
  importing: { type: Boolean, default: false },
  /** 顶层模式：manage(默认双栏) | cards（角色卡库区，由 #cards 插槽填充） */
  mode: { type: String, default: 'manage' }, // manage | cards
  showModeSwitch: { type: Boolean, default: true },
})

const emit = defineEmits([
  'close',
  'select-campaign',
  'set-active',
  'delete-campaign',
  'change-tab',
  'change-mode',
  'new-campaign',
  'export-st',
  'export-bundle',
  'import-bundle',
  'refresh',
])

const tabs = [
  { key: 'instances', label: '实例' },
  { key: 'variables', label: '变量' },
  { key: 'knowledge', label: '知识' },
  { key: 'worldinfo', label: '世界书' },
  { key: 'tasks', label: '任务' },
  { key: 'summaries', label: '总结' },
]

const title = computed(() => props.selectedCampaign?.name || '未选择活动')
const recentInstances = computed(() => (props.instances || []).slice(0, 6))
const knowledgePreview = computed(() => (props.knowledge || []).slice(0, 4))
const fileActions = computed(() => [
  { event: 'import-bundle', label: props.importing ? '导入中…' : '导入 Bundle', icon: FileUp, disabled: props.importing },
  { event: 'export-st', label: '导出 ST', icon: ImageDown, disabled: props.exporting || !props.selectedCampaignId || props.mode === 'cards' },
  { event: 'export-bundle', label: '导出 Bundle', icon: FileDown, disabled: props.exporting || !props.selectedCampaignId || props.mode === 'cards' },
])

function taskStatusClass(status) {
  const s = String(status || '').toLowerCase()
  if (s.includes('done') || s.includes('完成')) return 'text-ok border-ok/30 bg-ok/10'
  if (s.includes('run') || s.includes('进行')) return 'text-running border-running/30 bg-running/10'
  if (s.includes('fail') || s.includes('失败')) return 'text-err border-err/30 bg-err/10'
  return 'text-ink-soft border-line bg-surface-2'
}
</script>

<template>
  <div class="h-full flex flex-col bg-bg">
    <header class="sf-toolbar px-3 sm:px-4 border-b border-line bg-surface flex items-center gap-2">
      <div class="min-w-0 flex-1">
        <div class="text-[11px] text-ink-faint">活动管理</div>
        <h1 class="text-sm font-semibold text-ink truncate">{{ mode === 'cards' ? '角色卡库' : title }}</h1>
      </div>

      <div v-if="showModeSwitch" class="hidden sm:flex items-center rounded-md border border-line p-0.5 bg-surface-2/40">
        <button
          type="button"
          class="min-h-7 px-2.5 rounded text-xs transition-colors"
          :class="mode === 'manage' ? 'bg-surface text-ink shadow-card' : 'text-ink-soft hover:text-ink'"
          @click="emit('change-mode', 'manage')"
        >活动</button>
        <button
          type="button"
          class="min-h-7 px-2.5 rounded text-xs transition-colors"
          :class="mode === 'cards' ? 'bg-surface text-ink shadow-card' : 'text-ink-soft hover:text-ink'"
          @click="emit('change-mode', 'cards')"
        >角色卡</button>
      </div>

      <div class="hidden sm:flex shrink-0 items-center gap-1">
        <button
          v-for="action in fileActions"
          :key="action.event"
          type="button"
          class="sf-toolbar-icon disabled:opacity-40"
          :aria-label="action.label"
          :title="action.label"
          :disabled="action.disabled"
          @click="emit(action.event)"
        >
          <component :is="action.icon" :size="18" aria-hidden="true" />
        </button>
      </div>
      <Menu as="div" class="relative shrink-0 sm:hidden">
        <MenuButton class="sf-toolbar-icon" aria-label="导入与导出" title="导入与导出">
          <Ellipsis :size="20" aria-hidden="true" />
        </MenuButton>
        <MenuItems class="absolute right-0 top-full z-20 w-48 max-w-[calc(100vw-2rem)] rounded-md border border-line bg-surface p-1 shadow-float focus:outline-none">
          <MenuItem v-for="action in fileActions" :key="action.event" v-slot="{ active }" :disabled="action.disabled">
            <button
              type="button"
              class="flex min-h-11 w-full items-center gap-3 rounded px-3 text-left text-sm disabled:opacity-40"
              :class="active ? 'bg-surface-2 text-ink' : 'text-ink-soft'"
              :disabled="action.disabled"
              @click="emit(action.event)"
            >
              <component :is="action.icon" :size="18" aria-hidden="true" />
              {{ action.label }}
            </button>
          </MenuItem>
        </MenuItems>
      </Menu>
      <button
        type="button"
        class="sf-toolbar-icon transition-colors"
        title="关闭"
        aria-label="关闭"
        @click="emit('close')"
      >
        <X :size="18" aria-hidden="true" />
      </button>
    </header>

    <div v-if="exportStatus || importStatus" class="shrink-0 px-4 py-2 text-xs text-ink-soft border-b border-line bg-surface-2/40">
      {{ exportStatus || importStatus }}
    </div>

    <!-- 角色卡模式：完全由插槽填充（CardLibrary 等） -->
    <div v-if="mode === 'cards'" class="flex-1 min-h-0 overflow-y-auto p-4 sm:p-6">
      <slot name="cards" />
    </div>

    <div v-else class="flex-1 min-h-0 flex">
      <aside class="w-[240px] shrink-0 border-r border-line bg-surface hidden sm:flex flex-col">
        <div class="px-3 py-3 flex items-center justify-between">
          <span class="text-xs font-medium text-ink-soft">我的活动</span>
          <button
            type="button"
            class="min-h-7 px-2 rounded-md text-xs text-accent hover:bg-accent-soft transition-colors"
            @click="emit('new-campaign')"
          >新建</button>
        </div>
        <div v-if="loadingCampaigns" class="px-3 py-6 text-xs text-ink-faint">加载中…</div>
        <div v-else-if="!campaigns.length" class="px-3 py-6 text-xs text-ink-faint">还没有活动档</div>
        <ul v-else class="flex-1 overflow-y-auto px-2 pb-3 space-y-1">
          <li v-for="c in campaigns" :key="c.id">
            <button
              type="button"
              class="w-full text-left rounded-lg px-3 py-2.5 border transition-colors"
              :class="selectedCampaignId === c.id
                ? 'border-accent-border bg-accent-soft/40'
                : 'border-transparent hover:bg-surface-2/70'"
              @click="emit('select-campaign', c)"
            >
              <div class="text-[13px] font-medium text-ink truncate">{{ c.name }}</div>
              <div class="mt-0.5 text-[11px] text-ink-faint truncate">
                {{ c.instance_count ?? '—' }} 角色
                <template v-if="c.updated_at || c.created_at"> · {{ new Date(c.updated_at || c.created_at).toLocaleDateString() }}</template>
              </div>
            </button>
          </li>
        </ul>
      </aside>

      <main class="flex-1 min-w-0 overflow-y-auto">
        <div v-if="!selectedCampaign" class="h-full flex items-center justify-center p-8 text-center">
          <div>
            <h2 class="font-semibold text-[18px] text-ink">选择或创建一个活动</h2>
            <p class="mt-2 text-sm text-ink-soft">活动是长线故事的真相源：实例、变量、知识、任务与总结都挂在这里。</p>
            <div class="mt-5 flex items-center justify-center gap-2">
              <button
                type="button"
                class="min-h-9 px-4 rounded-md bg-accent text-white text-[13px] font-medium hover:bg-accent-bright transition-colors"
                @click="emit('new-campaign')"
              >新建活动</button>
              <button
                type="button"
                class="min-h-9 px-4 rounded-md border border-line text-[13px] text-ink-soft hover:bg-surface-2 transition-colors"
                @click="emit('change-mode', 'cards')"
              >从角色卡开始</button>
            </div>
          </div>
        </div>

        <div v-else class="p-4 sm:p-6 space-y-5">
          <div class="flex flex-col gap-3 lg:flex-row lg:items-start">
            <div class="min-w-0 flex-1">
              <h2 class="font-semibold text-lg text-ink leading-snug [overflow-wrap:anywhere]">{{ selectedCampaign.name }}</h2>
              <p class="mt-1 text-xs text-ink-faint">
                <span v-if="selectedCampaign.story_clock">故事时间 · {{ selectedCampaign.story_clock }}</span>
                <span v-if="selectedCampaign.story_clock && selectedCampaign.id" class="mx-1">·</span>
                <span class="font-mono">{{ selectedCampaign.id?.slice?.(0, 8) }}</span>
              </p>
            </div>
            <div class="flex shrink-0 flex-wrap items-center gap-2">
              <button
                type="button"
                class="min-h-8 px-3 rounded-md text-xs border border-accent-border text-accent-bright hover:bg-accent-soft transition-colors"
                @click="emit('set-active', selectedCampaign.id)"
              >设为当前活动</button>
              <button
                type="button"
                class="min-h-8 px-3 rounded-md text-xs border border-line text-ink-soft hover:bg-surface-2 transition-colors"
                @click="emit('refresh')"
              >刷新</button>
              <button
                type="button"
                class="min-h-8 px-3 rounded-md text-xs border border-err/40 text-err hover:bg-err/10 transition-colors"
                title="删除整局活动（含对话与总结）"
                @click="emit('delete-campaign', selectedCampaign)"
              >删除活动</button>
            </div>
          </div>

          <div class="flex items-center gap-1 border-b border-line overflow-x-auto">
            <button
              v-for="t in tabs"
              :key="t.key"
              type="button"
              class="shrink-0 whitespace-nowrap min-h-10 px-3 text-[13px] border-b-2 -mb-px transition-colors"
              :class="detailTab === t.key
                ? 'border-accent text-accent-bright font-medium'
                : 'border-transparent text-ink-soft hover:text-ink'"
              @click="emit('change-tab', t.key)"
            >{{ t.label }}</button>
          </div>

          <!-- 深度详情插槽优先（生产注入 Instances/Knowledge/Tasks/Summaries Tab） -->
          <div v-if="$slots.detail">
            <slot name="detail" :detail-tab="detailTab" :campaign-id="selectedCampaignId" />
          </div>

          <template v-else>
            <div v-if="loadingDetail" class="py-10 text-center text-xs text-ink-faint">加载详情…</div>

            <section v-else-if="detailTab === 'instances'">
              <div v-if="!instances.length" class="rounded-xl border border-line bg-surface px-5 py-10 text-center text-sm text-ink-soft">还没有角色实例</div>
              <div v-else class="grid sm:grid-cols-2 lg:grid-cols-3 gap-3">
                <article v-for="inst in instances" :key="inst.id" class="rounded-xl border border-line bg-surface shadow-card p-4">
                  <div class="flex items-start gap-3">
                    <div class="w-10 h-10 rounded-full bg-surface-2 border border-line flex items-center justify-center text-xs text-ink-soft shrink-0">
                      {{ (inst.name || inst.character_name || '?').slice(0, 1) }}
                    </div>
                    <div class="min-w-0 flex-1">
                      <div class="text-sm font-medium text-ink truncate">{{ inst.name || inst.character_name || '未命名' }}</div>
                      <div class="mt-0.5 text-[11px] text-ink-faint truncate">{{ inst.role_type || inst.character_id || '角色' }}</div>
                    </div>
                  </div>
                </article>
              </div>
            </section>

            <section v-else-if="detailTab === 'knowledge'">
              <div v-if="!knowledge.length" class="rounded-xl border border-line bg-surface px-5 py-10 text-center text-sm text-ink-soft">暂无知识条目</div>
              <ul v-else class="rounded-xl border border-line bg-surface shadow-card divide-y divide-line overflow-hidden">
                <li v-for="k in knowledge" :key="k.id || k.key || k.title" class="px-4 py-3">
                  <div class="text-sm font-medium text-ink">{{ k.title || k.key || '知识' }}</div>
                  <div class="mt-1 text-xs text-ink-soft line-clamp-2">{{ k.content || k.summary || k.value || '' }}</div>
                </li>
              </ul>
            </section>

            <section v-else-if="detailTab === 'tasks'">
              <div v-if="!tasks.length" class="rounded-xl border border-line bg-surface px-5 py-10 text-center text-sm text-ink-soft">暂无任务</div>
              <ul v-else class="space-y-2">
                <li v-for="t in tasks" :key="t.id || t.title" class="rounded-xl border border-line bg-surface shadow-card px-4 py-3 flex items-start gap-3">
                  <span class="mt-0.5 shrink-0 px-2 py-0.5 rounded-full border text-[10px]" :class="taskStatusClass(t.status)">{{ t.status || '待办' }}</span>
                  <div class="min-w-0 flex-1">
                    <div class="text-sm font-medium text-ink">{{ t.title || t.name || '任务' }}</div>
                    <div v-if="t.description" class="mt-1 text-xs text-ink-soft line-clamp-2">{{ t.description }}</div>
                  </div>
                </li>
              </ul>
            </section>

            <section v-else>
              <div v-if="!summaries.length" class="rounded-xl border border-line bg-surface px-5 py-10 text-center text-sm text-ink-soft">暂无摘要</div>
              <ul v-else class="space-y-2">
                <li v-for="s in summaries" :key="s.id || s.round || s.title" class="rounded-xl border border-line bg-surface shadow-card px-4 py-3">
                  <div class="flex items-center gap-2 text-xs text-ink-faint mb-1">
                    <span v-if="s.round != null">第 {{ s.round }} 轮</span>
                    <span v-if="s.created_at">{{ new Date(s.created_at).toLocaleString() }}</span>
                  </div>
                  <div class="text-sm text-ink leading-relaxed">{{ s.summary || s.content || s.title || '' }}</div>
                </li>
              </ul>
            </section>

            <div class="grid lg:grid-cols-2 gap-4 pt-2">
              <section class="rounded-xl border border-line bg-surface shadow-card p-4">
                <div class="flex items-center justify-between mb-3">
                  <h3 class="text-sm font-medium text-ink">最近实例</h3>
                  <span class="text-[11px] text-ink-faint">{{ instances.length }} 个</span>
                </div>
                <ul class="space-y-2">
                  <li v-for="inst in recentInstances" :key="'r-' + inst.id" class="flex items-center gap-2 text-xs">
                    <span class="w-6 h-6 rounded-full bg-surface-2 border border-line flex items-center justify-center text-[10px] text-ink-soft">{{ (inst.name || '?').slice(0, 1) }}</span>
                    <span class="text-ink truncate">{{ inst.name || inst.character_name }}</span>
                  </li>
                  <li v-if="!recentInstances.length" class="text-xs text-ink-faint">暂无</li>
                </ul>
              </section>
              <section class="rounded-xl border border-line bg-surface shadow-card p-4">
                <div class="flex items-center justify-between mb-3">
                  <h3 class="text-sm font-medium text-ink">知识库概览</h3>
                  <span class="text-[11px] text-ink-faint">{{ knowledge.length }} 条</span>
                </div>
                <ul class="space-y-2">
                  <li v-for="k in knowledgePreview" :key="'kp-' + (k.id || k.title)" class="text-xs text-ink-soft truncate">
                    {{ k.title || k.key || '知识条目' }}
                  </li>
                  <li v-if="!knowledgePreview.length" class="text-xs text-ink-faint">暂无</li>
                </ul>
              </section>
            </div>
          </template>
        </div>
      </main>
    </div>
  </div>
</template>
