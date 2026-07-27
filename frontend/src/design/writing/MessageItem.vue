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
import { ref, computed, watch } from 'vue'
import VariantStrip from './VariantStrip.vue'
import { rerollPolicy } from '../../utils/rerollPolicy.js'

const props = defineProps({
  message: { type: Object, required: true },
  busy: { type: Boolean, default: false },
  canBranch: { type: Boolean, default: false },
  /** Product modes reroll the complete pipeline; legacy big-scene keeps artifact-level rerolls. */
  allowPartialReroll: { type: Boolean, default: true },
  /** Currently selected product pipeline. Provenance is checked separately per variant. */
  generationMode: { type: String, default: null },
  /** 质量门禁采纳提示文案（adapter 从 pipeline.quality 派生） */
  qualityAcceptHint: { type: String, default: null },
  turnReceipt: { type: Object, default: null },
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
  'retry-postprocess',
  'dismiss-receipt',
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
const replayPolicy = computed(() => rerollPolicy(
  props.generationMode,
  currentVariant.value?.provenance?.generation_mode ?? null,
  props.allowPartialReroll,
))
const hasVariants = computed(() => (props.message.variants?.length || 0) > 1)
const isFinal = computed(() => currentVariant.value?.status === 'final')
const receiptSelection = ref({})

watch(
  () => props.turnReceipt,
  (receipt) => {
    receiptSelection.value = Object.fromEntries(
      (receipt?.items || []).map((item) => [
        item.mutation_index,
        item.selected !== false && item.selected_by_default !== false,
      ]),
    )
  },
  { immediate: true, deep: true },
)

const selectedReceiptMutationIndices = computed(() =>
  (props.turnReceipt?.items || [])
    .filter((item) => receiptSelection.value[item.mutation_index] !== false)
    .map((item) => item.mutation_index),
)

function receiptKindLabel(kind) {
  return {
    chronicle: '纪要',
    knowledge: '知识',
    variable: '变量',
    task: '任务',
  }[kind] || '状态'
}

function confirmReceipt(forceAccept = false) {
  emit('accept-variant', {
    nodeId: props.message.id,
    forceAccept,
    selectedMutationIndices: selectedReceiptMutationIndices.value,
  })
}

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
          <button v-if="replayPolicy.editorOnly" type="button" @click="pickReroll('editor')" class="w-full text-left min-h-9 px-3 text-[13px] hover:bg-accent-soft transition-colors">只重跑 · 编剧</button>
          <template v-if="(replayPolicy.editorOnly || replayPolicy.sequentialSuffix) && subagentRoles.length">
            <div class="border-t border-line my-1"></div>
            <button
              v-for="role in subagentRoles"
              :key="role.id"
              type="button"
              @click="pickReroll('subagent:' + role.id)"
              class="w-full text-left min-h-9 px-3 text-[13px] text-accent hover:bg-accent-soft transition-colors"
            >{{ replayPolicy.sequentialSuffix ? '从此角色起重演' : '只重跑' }} · {{ role.label }}</button>
            <div class="px-3 py-1.5 text-[11px] text-ink-faint">
              {{ replayPolicy.sequentialSuffix ? '该角色及其后的演出、编剧都会重新执行' : '复用其余阶段产物' }}
            </div>
          </template>
          <div v-else-if="replayPolicy.editorOnly" class="px-3 py-2 text-[11px] text-ink-faint">无溯源信息，仅支持整体/编剧重 roll</div>
          <div v-else class="px-3 py-2 text-[11px] text-ink-faint">当前模式会整体重写，确保各阶段产物一致</div>
        </div>
      </div>

      <button type="button" @click="emit('add-variant', { messageId: message.id })" :disabled="busy" class="min-h-7 px-2 rounded-md hover:bg-accent-soft hover:text-accent-bright disabled:opacity-40 transition-colors">添变体</button>
      <button v-if="canBranch" type="button" @click="emit('branch', { nodeId: message.id })" :disabled="busy" class="min-h-7 px-2 rounded-md hover:bg-accent-soft hover:text-accent-bright disabled:opacity-40 transition-colors">分支</button>
      <button type="button" @click="emit('delete-variant', { nodeId: message.id })" :disabled="busy" class="min-h-7 px-2 rounded-md text-ink-faint hover:bg-err/10 hover:text-err disabled:opacity-40 transition-colors ml-auto">删除</button>
    </footer>

    <section
      v-if="turnReceipt"
      class="mt-4 rounded-xl border border-accent-border bg-accent-soft/25 p-4 shadow-card"
      aria-label="采纳前记账小票"
    >
      <div class="flex items-start gap-3">
        <div class="min-w-0 flex-1">
          <h3 class="text-sm font-medium text-ink">采纳前记账小票</h3>
          <p class="mt-1 text-xs leading-relaxed text-ink-soft">
            正文尚未提交。确认后，只写入下方勾选的纪要与状态变化。
          </p>
        </div>
        <span class="shrink-0 rounded-full border border-line bg-surface px-2 py-0.5 text-[10px] text-ink-faint">
          {{ turnReceipt.items?.length || 0 }} 项
        </span>
      </div>

      <p
        v-if="turnReceipt.notice"
        class="mt-3 rounded-md px-3 py-2 text-xs leading-relaxed"
        :class="turnReceipt.derivation_failed ? 'bg-warn/10 text-warn' : 'bg-surface-2 text-ink-soft'"
      >{{ turnReceipt.notice }}</p>

      <div v-if="turnReceipt.items?.length" class="mt-3 space-y-2">
        <label
          v-for="item in turnReceipt.items"
          :key="item.mutation_index"
          class="flex cursor-pointer items-start gap-3 rounded-lg border border-line bg-surface px-3 py-2.5 transition-colors hover:border-accent-border"
        >
          <input
            v-model="receiptSelection[item.mutation_index]"
            type="checkbox"
            class="mt-0.5 h-4 w-4 rounded border-line accent-[var(--color-accent)]"
          />
          <span class="min-w-0 flex-1">
            <span class="flex items-center gap-2">
              <span class="rounded bg-surface-2 px-1.5 py-0.5 text-[10px] text-ink-faint">{{ receiptKindLabel(item.kind) }}</span>
              <span class="text-xs font-medium text-ink">{{ item.title }}</span>
            </span>
            <span class="mt-1 block text-xs leading-relaxed text-ink-soft">{{ item.detail }}</span>
          </span>
        </label>
      </div>

      <p v-if="turnReceipt.retry_error" class="mt-3 text-xs text-err">
        重试失败：{{ turnReceipt.retry_error }}
      </p>

      <div class="mt-4 flex flex-wrap justify-end gap-2">
        <button
          type="button"
          class="min-h-8 px-3 text-xs rounded-md text-ink-soft hover:bg-surface-2 transition-colors"
          @click="emit('dismiss-receipt', { nodeId: message.id })"
        >暂不采纳</button>
        <button
          v-if="turnReceipt.can_retry"
          type="button"
          :disabled="turnReceipt.retrying"
          class="min-h-8 px-3 text-xs rounded-md border border-line bg-surface text-ink-soft hover:border-accent-border hover:text-accent disabled:opacity-40 transition-colors"
          @click="emit('retry-postprocess', { nodeId: message.id })"
        >{{ turnReceipt.retrying ? '重新提取中…' : '重新提取记账' }}</button>
        <button
          v-if="turnReceipt.ready"
          type="button"
          :disabled="turnReceipt.retrying"
          class="min-h-8 px-3.5 text-xs rounded-md text-white disabled:opacity-40 transition-colors"
          :class="turnReceipt.derivation_failed ? 'bg-warn hover:brightness-105' : 'bg-accent hover:bg-accent-bright'"
          @click="confirmReceipt(!!turnReceipt.derivation_failed)"
        >{{ turnReceipt.derivation_failed ? '降级采纳正文' : '确认采纳' }}</button>
      </div>
    </section>

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
