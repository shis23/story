<script setup>
/**
 * MessageItem — 稿纸段落（重设计，纯展示）。
 *
 * 结构（对齐 selected 图②③）：
 *   - 用户消息=淡色意图注；助手消息=文稿正文段
 *   - 变体：并排卡（VariantStrip）；操作：段落底部悬停浮现
 *   - 重 roll：整体 / 编剧 / 子 Agent 三级菜单（内联，不依赖 store）
 *
 * 契约：emit 8 个事件，名称/payload 与 useMessageVariants 完全对齐。
 * RichContent：可选 contentComponent 由 adapter 注入（design 层不 import 功能层）。
 */
import { ref, computed } from 'vue'
import VariantStrip from './VariantStrip.vue'

const props = defineProps({
  message: { type: Object, required: true },
  busy: { type: Boolean, default: false },
  canBranch: { type: Boolean, default: false },
  /** 质量门禁采纳提示文案（adapter 从 pipeline.quality 派生） */
  qualityAcceptHint: { type: String, default: null },
  /** 可选内容渲染组件（生产注入 RichContent；预览默认纯文本） */
  contentComponent: { type: [Object, Function, String], default: null },
  /** 子 Agent 角色列表：[{ id, label }]，用于重 roll 菜单 */
  subagentRoles: { type: Array, default: () => [] },
})

const emit = defineEmits([
  'reroll',
  'reroll-user',
  'switch-variant',
  'edit-variant',
  'accept-variant',
  'delete-variant',
  'add-variant',
  'branch',
])

const isUser = computed(() => props.message.role === 'user')
const currentVariant = computed(() => props.message.variants[props.message.active_variant])
const displayContent = computed(
  () => currentVariant.value?.display_content ?? currentVariant.value?.content ?? '',
)
const sourceContent = computed(() => currentVariant.value?.content ?? '')
const seed = computed(() => currentVariant.value?.provenance?.seed)
const hasVariants = computed(() => (props.message.variants?.length || 0) > 1)
const isFinal = computed(() => currentVariant.value?.status === 'final')

// ── 内联编辑 ──
const editing = ref(false)
const editContent = ref('')
function startEdit() {
  editContent.value = currentVariant.value?.content ?? ''
  editing.value = true
}
function saveEdit() {
  emit('edit-variant', { nodeId: props.message.id, newContent: editContent.value })
  editing.value = false
}

// ── 重 roll 菜单 ──
const showRerollMenu = ref(false)
const showHint = ref(false)
const pendingKind = ref('all')
const hint = ref('')

function pickReroll(kind) {
  showRerollMenu.value = false
  pendingKind.value = kind
  hint.value = ''
  showHint.value = true
}
function confirmReroll() {
  showHint.value = false
  emit('reroll', {
    messageId: props.message.id,
    nodeId: props.message.id,
    kind: pendingKind.value,
    hint: hint.value.trim() || null,
  })
  hint.value = ''
  pendingKind.value = 'all'
}

function paragraphs(text) {
  return String(text || '').split(/\n{2,}/).filter(Boolean)
}
</script>

<template>
  <!-- ── 用户意图：淡注（非聊天气泡） ── -->
  <section v-if="isUser" class="group py-4">
    <div class="flex items-center gap-3" aria-hidden="true">
      <span class="h-px flex-1 bg-line/70"></span>
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" class="text-ink-faint"><path d="M17 3a2.8 2.8 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5z"/></svg>
      <span class="h-px flex-1 bg-line/70"></span>
    </div>
    <div class="mt-2 text-center text-[13px] leading-relaxed text-ink-faint">
      <component
        :is="contentComponent"
        v-if="contentComponent"
        :content="displayContent"
        :source-content="sourceContent"
      />
      <template v-else>{{ displayContent }}</template>
    </div>

    <div class="mt-1 flex justify-center gap-0.5 text-xs text-ink-faint transition-opacity duration-200 opacity-100 sm:opacity-0 sm:group-hover:opacity-100">
      <button type="button" @click="startEdit" :disabled="busy" class="min-h-7 px-2 rounded-md hover:bg-accent-soft hover:text-accent-bright disabled:opacity-40 transition-colors">编辑</button>
      <button type="button" @click="emit('reroll-user', { messageId: message.id })" :disabled="busy" class="min-h-7 px-2 rounded-md hover:bg-accent-soft hover:text-accent-bright disabled:opacity-40 transition-colors">重 roll</button>
      <button type="button" @click="emit('delete-variant', { nodeId: message.id })" :disabled="busy" class="min-h-7 px-2 rounded-md hover:bg-err/10 hover:text-err disabled:opacity-40 transition-colors">删除</button>
    </div>

    <div v-if="editing" class="mt-2 rounded-lg border border-accent-border bg-surface p-4 shadow-card">
      <textarea v-model="editContent" rows="3" class="w-full bg-transparent resize-none text-sm leading-relaxed text-ink focus:outline-none"></textarea>
      <div class="flex justify-end gap-2 mt-2">
        <button type="button" @click="editing = false" class="min-h-8 px-3 text-[13px] rounded-md text-ink-soft hover:bg-surface-2 transition-colors">取消</button>
        <button type="button" @click="saveEdit" class="min-h-8 px-3 text-[13px] rounded-md bg-accent text-white hover:bg-accent-bright transition-colors">保存</button>
      </div>
    </div>
  </section>

  <!-- ── 助手成文：文稿段落 ── -->
  <section v-else class="group py-5">
    <div v-if="!editing" class="prose-fiction text-[15.5px] text-ink">
      <component
        :is="contentComponent"
        v-if="contentComponent"
        :content="displayContent"
        :source-content="sourceContent"
      />
      <template v-else>
        <p v-for="(p, i) in paragraphs(displayContent)" :key="i" class="mb-4 last:mb-0">{{ p }}</p>
      </template>
    </div>

    <div v-else class="rounded-lg border border-accent-border bg-surface p-4 shadow-card">
      <textarea v-model="editContent" rows="6" class="w-full bg-transparent resize-none text-[15px] leading-relaxed text-ink focus:outline-none"></textarea>
      <div class="flex justify-end gap-2 mt-3">
        <button type="button" @click="editing = false" class="min-h-8 px-3 text-[13px] rounded-md text-ink-soft hover:bg-surface-2 transition-colors">取消</button>
        <button type="button" @click="saveEdit" class="min-h-8 px-3 text-[13px] rounded-md bg-accent text-white hover:bg-accent-bright transition-colors">保存</button>
      </div>
    </div>

    <VariantStrip
      v-if="hasVariants && !editing"
      :message="message"
      :busy="busy"
      @switch-variant="emit('switch-variant', $event)"
      @accept-variant="emit('accept-variant', $event)"
    />

    <footer
      v-if="!editing"
      class="mt-3 flex flex-wrap items-center gap-0.5 text-xs text-ink-soft transition-opacity duration-200 opacity-100 sm:opacity-0 sm:group-hover:opacity-100"
    >
      <span class="text-ink-faint mr-1">{{ message.role_label }}<template v-if="seed != null"> · seed {{ seed }}</template></span>
      <button type="button" @click="startEdit" :disabled="busy" class="min-h-7 px-2 rounded-md hover:bg-accent-soft hover:text-accent-bright disabled:opacity-40 transition-colors">编辑</button>
      <button
        type="button"
        @click="emit('accept-variant', { nodeId: message.id })"
        :disabled="busy"
        class="min-h-7 px-2 rounded-md hover:bg-accent-soft transition-colors disabled:opacity-40"
        :class="isFinal ? 'text-ok' : (qualityAcceptHint ? 'text-warn' : 'hover:text-accent-bright')"
        :title="qualityAcceptHint || undefined"
      >{{ isFinal ? '已采纳' : '采纳' }}</button>
      <span v-if="qualityAcceptHint && !isFinal" class="text-[11px] text-warn">{{ qualityAcceptHint }}</span>

      <!-- 重 roll 三级菜单 -->
      <div class="relative">
        <button
          type="button"
          @click="showRerollMenu = !showRerollMenu"
          :disabled="busy"
          class="min-h-7 px-2 rounded-md hover:bg-accent-soft hover:text-accent-bright disabled:opacity-40 transition-colors"
        >重 roll ▾</button>
        <div
          v-if="showRerollMenu"
          class="absolute left-0 bottom-full mb-1 z-20 min-w-[200px] rounded-lg border border-line bg-surface shadow-float py-1"
        >
          <button type="button" @click="pickReroll('all')" class="w-full text-left min-h-9 px-3 text-[13px] hover:bg-accent-soft transition-colors">整体重 roll</button>
          <button type="button" @click="pickReroll('editor')" class="w-full text-left min-h-9 px-3 text-[13px] hover:bg-accent-soft transition-colors">只重跑 · 编剧</button>
          <template v-if="subagentRoles.length">
            <div class="border-t border-line my-1"></div>
            <button
              v-for="role in subagentRoles"
              :key="role.id"
              type="button"
              @click="pickReroll('subagent:' + role.id)"
              class="w-full text-left min-h-9 px-3 text-[13px] text-accent hover:bg-accent-soft transition-colors"
            >只重跑 · {{ role.label }}（子Agent）</button>
            <div class="px-3 py-1.5 text-[11px] text-ink-faint">省 60% token</div>
          </template>
          <div v-else class="px-3 py-2 text-[11px] text-ink-faint">无溯源信息，仅支持整体/编剧重 roll</div>
        </div>
      </div>

      <button type="button" @click="emit('add-variant', { messageId: message.id })" :disabled="busy" class="min-h-7 px-2 rounded-md hover:bg-accent-soft hover:text-accent-bright disabled:opacity-40 transition-colors">添变体</button>
      <button v-if="canBranch" type="button" @click="emit('branch', { nodeId: message.id })" :disabled="busy" class="min-h-7 px-2 rounded-md hover:bg-accent-soft hover:text-accent-bright disabled:opacity-40 transition-colors">分支</button>
      <button type="button" @click="emit('delete-variant', { nodeId: message.id })" :disabled="busy" class="min-h-7 px-2 rounded-md text-ink-faint hover:bg-err/10 hover:text-err disabled:opacity-40 transition-colors ml-auto">删除</button>
    </footer>

    <div v-if="showHint" class="mt-3 rounded-lg border border-accent-border bg-accent-soft/40 p-3">
      <div class="text-xs text-ink-soft mb-2">附加提示（可选）：告诉 Agent 上次哪里不满意</div>
      <textarea v-model="hint" rows="2" placeholder="例如：节奏太快；结尾太仓促；角色语气不对…" class="w-full text-sm rounded-md border border-line bg-surface px-3 py-2 resize-none focus:outline-none focus:border-accent-border"></textarea>
      <div class="flex justify-end gap-2 mt-2">
        <button type="button" @click="showHint = false" class="min-h-8 px-3 text-[13px] rounded-md text-ink-soft hover:bg-surface-2 transition-colors">取消</button>
        <button type="button" @click="confirmReroll" :disabled="busy" class="min-h-8 px-3 text-[13px] rounded-md bg-accent text-white hover:bg-accent-bright disabled:opacity-40 transition-colors">开始重 roll</button>
      </div>
    </div>
  </section>
</template>
