<script setup>
/**
 * MetaScreenDemo — 设计预览（#design-meta）。
 * 固定宽抽屉 + 五 tab 空/有内容态，验证切换 tab 不跳宽。
 */
import { ref } from 'vue'
import MetaScreen from './MetaScreen.vue'

const activeTab = ref('chat')
const pending = ref(2)
const globalError = ref('')

const tabs = [
  { key: 'chat', label: '对话' },
  { key: 'patches', label: 'Patch' },
  { key: 'health', label: '健康检查' },
  { key: 'mvu', label: 'MVU' },
  { key: 'explain', label: '生成解释' },
]

const chatEmpty = ref(false)

const patches = [
  {
    id: 'p1',
    description: '补全世界书「北门夜禁」条目冲突说明',
    actions: [
      { op: 'update', label: '改 lorebook.north_gate.note' },
      { op: 'create', label: '创建 lorebook.curfew_rule' },
    ],
  },
  {
    id: 'p2',
    description: '修正角色卡开场白索引越界',
    actions: [{ op: 'update', label: '改 card.greetings[3]' }],
  },
]

const health = {
  score: 78,
  issues: [
    { level: 'warn', text: '2 条世界书 selective 关键词重叠' },
    { level: 'ok', text: 'Campaign revision CAS 正常' },
    { level: 'warn', text: '1 个临时实例未 promote' },
  ],
}

</script>

<template>
  <div class="h-screen bg-bg flex flex-col">
    <div class="shrink-0 flex items-center gap-2 px-4 py-1.5 border-b border-line bg-surface-2/70">
      <span class="text-[11px] text-ink-faint">设计预览 · Meta · 外壳 --layout-drawer（380）· 切换 tab 宽度应不变</span>
      <button
        type="button"
        class="min-h-6 px-2 rounded text-[11px] text-ink-soft hover:bg-surface-2"
        @click="chatEmpty = !chatEmpty"
      >{{ chatEmpty ? '对话有内容' : '对话空态' }}</button>
      <button
        type="button"
        class="min-h-6 px-2 rounded text-[11px] text-ink-soft hover:bg-surface-2"
        @click="globalError = globalError ? '' : '示例：健康检查接口超时'"
      >切换错误条</button>
      <a href="#" class="ml-auto text-[11px] text-ink-faint hover:text-ink-soft">← 返回应用</a>
    </div>

    <!-- 仿真遮罩 + 固定宽抽屉（与生产 Overlay 一致） -->
    <div class="flex-1 relative bg-ink/10">
      <div class="absolute inset-y-0 left-0 w-[min(100vw,var(--layout-drawer))] border-r border-line shadow-float bg-bg flex flex-col">
        <MetaScreen
          class="h-full"
          :active-tab="activeTab"
          :tabs="tabs"
          :pending-patch-count="pending"
          :global-error="globalError"
          campaign-name="风起之地"
          @change-tab="activeTab = $event"
          @close="location.hash = ''"
        >
          <!-- 对话 -->
          <div v-if="activeTab === 'chat'" class="w-full min-w-0 space-y-3">
            <template v-if="chatEmpty">
              <div class="w-full min-w-0 flex flex-col items-center justify-center py-10 text-center">
                <p class="font-semibold text-base text-ink">问 Meta 助手任何配置问题</p>
                <p class="mt-2 text-sm text-ink-soft max-w-[280px] leading-relaxed">
                  「看看世界书有没有冲突」「这张卡的状态栏怎么分析」
                </p>
              </div>
            </template>
            <template v-else>
              <div class="flex justify-end">
                <div class="max-w-[85%] rounded-2xl rounded-br-md px-3.5 py-2 text-sm bg-accent text-white">
                  看看世界书有没有冲突
                </div>
              </div>
              <div class="flex justify-start">
                <div class="max-w-[85%] rounded-2xl rounded-bl-md px-3.5 py-2 text-sm bg-surface-2 border border-line text-ink">
                  发现 2 处 selective 关键词重叠，建议合并「夜禁」与「北门」条目。
                  <div class="mt-2 pt-2 border-t border-line text-xs space-y-1">
                    <div class="font-medium">世界书诊断</div>
                    <div class="text-ink-soft">共 48 条 / 蓝灯 6 / 绿灯 12</div>
                    <div class="text-warn">⚠ 2 处冲突</div>
                  </div>
                </div>
              </div>
            </template>
          </div>

          <!-- Patch -->
          <div v-else-if="activeTab === 'patches'" class="w-full min-w-0 space-y-2">
            <div
              v-for="p in patches"
              :key="p.id"
              class="rounded-lg border border-line bg-surface p-3"
            >
              <div class="text-sm font-medium text-ink">{{ p.description }}</div>
              <div class="mt-2 space-y-1">
                <div v-for="(a, i) in p.actions" :key="i" class="text-[11px] text-ink-soft">
                  <span class="text-accent">{{ a.op }}</span> · {{ a.label }}
                </div>
              </div>
              <div class="mt-3 flex gap-2">
                <button type="button" class="flex-1 min-h-8 rounded-md bg-accent text-white text-xs">采纳</button>
                <button type="button" class="flex-1 min-h-8 rounded-md border border-line text-xs text-ink-soft">忽略</button>
              </div>
            </div>
          </div>

          <!-- Health -->
          <div v-else-if="activeTab === 'health'" class="w-full min-w-0 space-y-3">
            <div class="rounded-xl border border-line bg-surface p-4">
              <div class="text-xs text-ink-faint">Campaign 健康分</div>
              <div class="mt-1 text-2xl font-semibold text-ink">{{ health.score }}</div>
            </div>
            <ul class="rounded-xl border border-line bg-surface divide-y divide-line">
              <li v-for="(it, i) in health.issues" :key="i" class="px-4 py-3 text-sm flex gap-2">
                <span :class="it.level === 'ok' ? 'text-ok' : 'text-warn'">{{ it.level === 'ok' ? '✓' : '!' }}</span>
                <span class="text-ink">{{ it.text }}</span>
              </li>
            </ul>
          </div>

          <!-- MVU -->
          <div v-else-if="activeTab === 'mvu'" class="w-full min-w-0">
            <div class="rounded-xl border border-line bg-surface p-4 space-y-2">
              <div class="text-sm font-medium text-ink">Schema 摘要</div>
              <div class="text-xs text-ink-soft font-mono">definitions: 4 · variables: 18 · paths ok</div>
              <button type="button" class="mt-2 min-h-8 px-3 rounded-md border border-accent-border text-xs text-accent-bright">
                预览合并
              </button>
            </div>
          </div>

          <!-- Explain -->
          <div v-else class="w-full min-w-0">
            <div class="rounded-xl border border-line bg-surface p-4 text-sm text-ink-soft leading-relaxed">
              上一轮生成：Director 规划悬念节拍 → 子 Agent「林述」声口采样 → Editor 成文 212 字。
              seed 42137 · quality passed。
            </div>
          </div>
        </MetaScreen>
      </div>
    </div>
  </div>
</template>
