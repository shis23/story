<script setup>
import { ref, computed } from 'vue'
import BaseDropdown from '../../components/base/BaseDropdown.vue'
import RichContent from '../st/RichContent.vue'
import { subagentRolesFromProvenance } from '../../utils/pipelineTrace.js'
import { useWritingStore } from '../../stores/writing.js'

// ⚠️ 契约红线组件:保持 props(4 个)+ emit(8 个)+ message shape 不变。
// AppV2.vue 通过这 8 个 emit 接到 useMessageVariants composable。
const props = defineProps({
  message: { type: Object, required: true },
  conversationId: { type: String, default: null },
  busy: { type: Boolean, default: false },
  canBranch: { type: Boolean, default: false },
})

const emit = defineEmits(['reroll', 'reroll-user', 'switch-variant', 'edit-variant', 'accept-variant', 'delete-variant', 'add-variant', 'branch'])

const writing = useWritingStore()
const showRerollMenu = ref(false)
const showHintBox = ref(false)
const pendingRerollKind = ref('')
const hintInput = ref('')
const editing = ref(false)
const editContent = ref('')

const currentVariant = computed(() => props.message.variants[props.message.active_variant])
const currentDisplayContent = computed(() => currentVariant.value?.display_content ?? currentVariant.value?.content ?? '')
const currentSourceContent = computed(() => currentVariant.value?.content ?? '')
const variantCount = computed(() => props.message.variants.length)
const subagentRoles = computed(() => subagentRolesFromProvenance(currentVariant.value?.provenance))
const isUser = computed(() => props.message.role === 'user')
// Error 拦截（需强制采纳）；Warning 仅提示
const qualityAcceptHint = computed(() => {
  const q = writing.pipeline.quality
  if (!q || q.passed) return null
  const errors = q.errorCount || 0
  const n = q.warningCount || (Array.isArray(q.warnings) ? q.warnings.length : 0)
  if (errors > 0) return `质量 Error ${errors}（采纳将确认）`
  if (!n) return null
  return `质量警告 ${n}`
})

function switchVariant(delta) {
  let next = props.message.active_variant + delta
  if (next < 0) next = props.message.variants.length - 1
  if (next >= props.message.variants.length) next = 0
  emit('switch-variant', { messageId: props.message.id, index: next })
}

function pickReroll(kind) {
  showRerollMenu.value = false
  pendingRerollKind.value = kind
  hintInput.value = ''
  showHintBox.value = true
}

function confirmReroll() {
  showHintBox.value = false
  const kind = pendingRerollKind.value
  const hint = hintInput.value.trim() || null
  emit('reroll', { messageId: props.message.id, nodeId: props.message.id, kind, hint })
}

function cancelHint() {
  showHintBox.value = false
  pendingRerollKind.value = ''
  hintInput.value = ''
}

function startEdit() {
  editContent.value = currentVariant.value.content
  editing.value = true
}

function saveEdit() {
  emit('edit-variant', { nodeId: props.message.id, newContent: editContent.value })
  editing.value = false
}

function cancelEdit() {
  editing.value = false
  editContent.value = ''
}

function acceptVariant() {
  emit('accept-variant', { nodeId: props.message.id })
}

async function deleteVariant() {
  const { ask } = await import('@tauri-apps/plugin-dialog')
  const ok = await ask('确定删除这条消息？删除后该消息从对话移除。', { title: '删除确认', kind: 'warning' })
  if (!ok) return
  emit('delete-variant', { nodeId: props.message.id })
}

function branchMessage() {
  emit('branch', { nodeId: props.message.id })
}

function rerollUser() {
  emit('reroll-user', { messageId: props.message.id })
}
</script>

<template>
  <div class="group -mx-3 px-3 py-5 rounded-lg transition-colors duration-200 hover:bg-surface/70">
    <div class="flex items-center gap-2.5 mb-2.5">
      <span class="text-xs font-medium tracking-wide"
        :class="isUser ? 'text-ink-soft' : 'text-accent'">
        {{ message.role_label }}
      </span>
      <div v-if="variantCount > 1" class="flex items-center gap-0.5 text-xs text-ink-soft border border-line rounded-full px-1 py-0.5">
        <button @click="switchVariant(-1)" class="w-6 h-6 flex items-center justify-center rounded-full hover:bg-accent-soft hover:text-accent-bright transition-colors" aria-label="上一版本">‹</button>
        <span class="px-1 tabular-nums">{{ message.active_variant + 1 }}/{{ variantCount }}</span>
        <button @click="switchVariant(1)" class="w-6 h-6 flex items-center justify-center rounded-full hover:bg-accent-soft hover:text-accent-bright transition-colors" aria-label="下一版本">›</button>
        <span v-if="currentVariant.status === 'discarded'" class="text-warn px-1">· 旧版</span>
      </div>
    </div>

    <div v-if="!editing"
      class="text-[15.5px] leading-loose"
      :class="isUser ? 'text-ink-soft pl-4 border-l-2 border-line' : 'text-ink prose-fiction'">
      <RichContent :content="currentDisplayContent" :source-content="currentSourceContent" />
    </div>

    <div v-else class="rounded-lg border border-accent-border bg-surface px-4 py-3 shadow-card">
      <textarea v-model="editContent" rows="4"
        class="w-full text-[15px] leading-relaxed bg-transparent resize-none focus:outline-none"></textarea>
      <div class="flex justify-end gap-2 mt-2">
        <button @click="cancelEdit" class="min-h-9 px-3.5 text-[13px] rounded-md text-ink-soft hover:bg-surface-2 transition-colors">取消</button>
        <button @click="saveEdit" class="min-h-9 px-3.5 text-[13px] rounded-md bg-accent text-white hover:bg-accent-bright transition-colors">保存</button>
      </div>
    </div>

    <div v-if="isUser" class="flex items-center gap-1 mt-3 text-ink-soft opacity-60 sm:opacity-0 sm:group-hover:opacity-100 transition-opacity duration-200">
      <button @click="startEdit" :disabled="busy" class="min-h-9 px-2.5 text-[13px] rounded-md hover:bg-accent-soft hover:text-accent-bright disabled:opacity-40 transition-colors">编辑</button>
      <button @click="rerollUser" :disabled="busy" class="min-h-9 px-2.5 text-[13px] rounded-md hover:bg-accent-soft hover:text-accent-bright disabled:opacity-40 transition-colors">重 roll</button>
    </div>

    <div v-if="!isUser" class="flex flex-wrap items-center gap-1 mt-3 text-[13px] text-ink-soft opacity-60 sm:opacity-0 sm:group-hover:opacity-100 transition-opacity duration-200">
      <button @click="startEdit" class="min-h-9 px-2.5 rounded-md hover:bg-accent-soft hover:text-accent-bright transition-colors">编辑</button>
      <button @click="acceptVariant" class="min-h-9 px-2.5 rounded-md hover:bg-accent-soft transition-colors"
        :class="currentVariant.status === 'final' ? 'text-ok' : (qualityAcceptHint ? 'text-warn' : 'hover:text-accent-bright')"
        :title="qualityAcceptHint || undefined">
        {{ currentVariant.status === 'final' ? '已采纳' : '采纳' }}
      </button>
      <span
        v-if="qualityAcceptHint && currentVariant.status !== 'final'"
        class="text-[11px] text-warn"
        :title="(writing.pipeline.quality?.warnings || []).join('\n')"
      >{{ qualityAcceptHint }}</span>
      <button v-if="canBranch" @click="branchMessage" :disabled="busy" class="min-h-9 px-2.5 rounded-md hover:bg-accent-soft hover:text-accent-bright disabled:opacity-40 transition-colors">分支</button>

      <BaseDropdown v-model="showRerollMenu" align="left" :min-width="220">
        <template #trigger>
          <button :disabled="busy" class="min-h-9 px-2.5 rounded-md hover:bg-accent-soft hover:text-accent-bright disabled:opacity-40 transition-colors">重 roll ▾</button>
        </template>
        <template #default="{ close }">
          <button @click="close(); pickReroll('all')" class="w-full text-left min-h-10 px-3 text-[13px] hover:bg-accent-soft transition-colors">整体重 roll</button>
          <button @click="close(); pickReroll('editor')" class="w-full text-left min-h-10 px-3 text-[13px] hover:bg-accent-soft transition-colors">只重跑 · 编剧</button>
          <template v-if="subagentRoles.length">
            <div class="border-t border-line my-1"></div>
            <button v-for="role in subagentRoles" :key="role.id"
              @click="close(); pickReroll('subagent:' + role.id)"
              class="w-full text-left min-h-10 px-3 text-[13px] text-accent hover:bg-accent-soft transition-colors">
              只重跑 · {{ role.label }}（子Agent）
            </button>
            <div class="px-3 py-1.5 text-[11px] text-ink-faint">省 60% token</div>
          </template>
          <div v-if="!subagentRoles.length" class="px-3 py-2 text-[11px] text-ink-faint">无溯源信息，仅支持整体/编剧重 roll</div>
        </template>
      </BaseDropdown>

      <span v-if="currentVariant.provenance" class="text-[11px] text-ink-faint font-mono">seed {{ currentVariant.provenance.seed }}</span>
      <button @click="deleteVariant" class="min-h-9 px-2.5 rounded-md text-ink-faint hover:bg-err/10 hover:text-err transition-colors ml-auto">删除</button>
    </div>

    <div v-if="showHintBox" class="mt-3 rounded-lg border border-accent-border bg-accent-soft/40 p-3">
      <div class="text-xs text-ink-soft mb-2">附加提示（可选）：告诉 Agent 上次哪里有问题</div>
      <textarea v-model="hintInput" rows="2" placeholder="例如：角色 B 语气太冷；节奏太快；结尾太仓促…"
        class="w-full text-sm rounded-md border border-line bg-surface px-3 py-2 resize-none focus:outline-none focus:border-accent-border"></textarea>
      <div class="flex justify-end gap-2 mt-2">
        <button @click="cancelHint" class="min-h-9 px-3.5 text-[13px] rounded-md text-ink-soft hover:bg-surface-2 transition-colors">取消</button>
        <button @click="confirmReroll" class="min-h-9 px-3.5 text-[13px] rounded-md bg-accent text-white hover:bg-accent-bright transition-colors">开始重 roll</button>
      </div>
    </div>
  </div>
</template>
