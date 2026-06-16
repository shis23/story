<script setup>
import { ref, computed } from 'vue'
import { formatContent } from '../utils/formatContent.js'

const props = defineProps({
  message: { type: Object, required: true },
  /** 当前消息所在对话 ID（重 roll 需要） */
  conversationId: { type: String, default: null },
  /** 是否有流水线正在运行（运行中禁用重 roll） */
  busy: { type: Boolean, default: false },
})

const emit = defineEmits(['reroll', 'reroll-user', 'switch-variant', 'edit-variant', 'accept-variant', 'delete-variant', 'add-variant'])

// 重 roll 菜单展开
const showRerollMenu = ref(false)
// hint 输入弹层
const showHintBox = ref(false)
// 待执行的重 roll 目标（kind 字符串）
const pendingRerollKind = ref('')
const hintInput = ref('')
const rerolling = ref(false)

// 内联编辑状态
const editing = ref(false)
const editContent = ref('')

const currentVariant = computed(() => {
  return props.message.variants[props.message.active_variant]
})

const variantCount = computed(() => props.message.variants.length)

// 从 Provenance 提取可重 roll 的子 Agent 角色名列表
const subagentRoles = computed(() => {
  const prov = currentVariant.value?.provenance
  if (!prov?.subagent_results) return []
  return prov.subagent_results.map((s) => s.character_id).filter(Boolean)
})

function switchVariant(delta) {
  let next = props.message.active_variant + delta
  if (next < 0) next = props.message.variants.length - 1
  if (next >= props.message.variants.length) next = 0
  emit('switch-variant', { messageId: props.message.id, index: next })
}

// 点击重 roll 菜单项 → 弹 hint 输入框
function pickReroll(kind) {
  showRerollMenu.value = false
  pendingRerollKind.value = kind
  hintInput.value = ''
  showHintBox.value = true
}

// 确认重 roll（带 hint）→ emit 给父组件执行
function confirmReroll() {
  showHintBox.value = false
  const kind = pendingRerollKind.value
  const hint = hintInput.value.trim() || null
  emit('reroll', {
    messageId: props.message.id,
    nodeId: props.message.id,
    kind, // 'all' | 'editor' | 'subagent:<角色名>'
    hint,
  })
  rerolling.value = true
}

function cancelHint() {
  showHintBox.value = false
  pendingRerollKind.value = ''
  hintInput.value = ''
}

// ─── 对话操作 ─────────────────────────────────────────────────────────────

// 开始编辑
function startEdit() {
  editContent.value = currentVariant.value.content
  editing.value = true
}

// 保存编辑
function saveEdit() {
  emit('edit-variant', {
    nodeId: props.message.id,
    newContent: editContent.value,
  })
  editing.value = false
}

// 取消编辑
function cancelEdit() {
  editing.value = false
  editContent.value = ''
}

// 采纳变体（Draft → Final）
function acceptVariant() {
  emit('accept-variant', { nodeId: props.message.id })
}

// 软删除变体（带确认，用 Tauri 原生对话框——WebView 的 window.confirm 不会弹窗）
async function deleteVariant() {
  const { ask } = await import('@tauri-apps/plugin-dialog')
  const ok = await ask('确定删除这条消息？删除后该消息从对话移除。', { title: '删除确认', kind: 'warning' })
  if (!ok) return
  emit('delete-variant', { nodeId: props.message.id })
}

// 分支：设计上分支 = 开新 Campaign 档（Campaign.fork_from 记录分叉点）。
// 当前未暴露 fork Tauri 命令，先提示用户在 Campaign 面板操作，避免误触导致消息消失。
function explainBranch() {
  window.alert('分支功能 = 从当前剧情另开一个新档（Campaign）。\n\n请在「Campaign 面板」中对当前游玩档使用「分叉」操作，选择从这里开始新的故事线。\n\n（该功能的后端 Campaign.fork 已实现，前端入口开发中）')
}

const isUser = computed(() => props.message.role === 'user')

// user 消息重 roll（用同样 intent 重新写作）
function rerollUser() {
  emit('reroll-user', { messageId: props.message.id })
}

</script>

<template>
  <div class="px-4 py-4" :class="isUser ? '' : ''">
    <!-- 角色标签 -->
    <div class="flex items-center gap-2 mb-2">
      <span class="text-xs font-medium px-2 py-0.5 rounded-md"
        :class="isUser ? 'bg-bg text-ink-soft' : 'bg-accent-soft text-accent'">
        {{ message.role_label }}
      </span>
      <!-- 版本切换条（多于1个版本才显示） -->
      <div v-if="variantCount > 1" class="flex items-center gap-1 text-xs text-ink-soft">
        <button @click="switchVariant(-1)" class="w-5 h-5 flex items-center justify-center rounded hover:bg-bg">‹</button>
        <span>{{ message.active_variant + 1 }}/{{ variantCount }}</span>
        <button @click="switchVariant(1)" class="w-5 h-5 flex items-center justify-center rounded hover:bg-bg">›</button>
        <!-- discarded 版本标记 -->
        <span v-if="currentVariant.status === 'discarded'" class="text-warn">· 旧版</span>
      </div>
    </div>

    <!-- 消息正文（AI 用衬线，用户用无衬线） -->
    <div v-if="!editing"
      class="rounded-2xl px-4 py-3 text-[15px] leading-relaxed"
      :class="isUser
        ? 'bg-accent-soft text-ink rounded-tr-sm'
        : 'bg-surface border border-line text-ink rounded-tl-sm prose-fiction'"
    >
      <span v-html="formatContent(currentVariant.content)"></span>
    </div>

    <!-- 内联编辑模式 -->
    <div v-else class="rounded-2xl border border-accent bg-surface px-4 py-3">
      <textarea
        v-model="editContent"
        rows="4"
        class="w-full text-[15px] leading-relaxed bg-transparent resize-none focus:outline-none"
      ></textarea>
      <div class="flex justify-end gap-2 mt-2">
        <button @click="cancelEdit" class="px-3 py-1 text-xs rounded-md hover:bg-bg">取消</button>
        <button @click="saveEdit" class="px-3 py-1 text-xs rounded-md bg-accent text-white hover:opacity-90">保存</button>
      </div>
    </div>

    <!-- 操作栏（user 消息：仅重 roll） -->
    <div v-if="isUser" class="flex items-center gap-1 mt-2 text-xs text-ink-soft">
      <button @click="rerollUser" :disabled="busy"
        class="px-2.5 py-1 rounded-md hover:bg-bg disabled:opacity-40">🔄 重roll</button>
    </div>

    <!-- 操作栏（仅 AI 消息显示完整操作） -->
    <div v-if="!isUser" class="flex items-center gap-1 mt-2 text-xs text-ink-soft">
      <button @click="startEdit" class="px-2.5 py-1 rounded-md hover:bg-bg">✏️ 编辑</button>
      <button @click="acceptVariant" class="px-2.5 py-1 rounded-md hover:bg-bg"
        :class="currentVariant.status === 'final' ? 'text-green-600' : ''">
        {{ currentVariant.status === 'final' ? '✅ 已采纳' : '☑️ 采纳' }}
      </button>

      <!-- 重 roll（带下拉菜单，含部分重 roll） -->
      <div class="relative">
        <button
          @click="showRerollMenu = !showRerollMenu"
          :disabled="busy"
          class="px-2.5 py-1 rounded-md hover:bg-bg disabled:opacity-40"
        >
          🔄 重roll ▾
        </button>
        <div
          v-if="showRerollMenu && !busy"
          class="absolute left-0 top-full mt-1 bg-surface border border-line rounded-xl shadow-lg py-1 min-w-[200px] z-10"
        >
          <button @click="pickReroll('all')" class="w-full text-left px-3 py-2 hover:bg-accent-soft">整体重 roll</button>
          <button @click="pickReroll('editor')" class="w-full text-left px-3 py-2 hover:bg-accent-soft">只重跑 · 编剧</button>
          <template v-if="subagentRoles.length">
            <div class="border-t border-line my-1"></div>
            <button
              v-for="role in subagentRoles"
              :key="role"
              @click="pickReroll('subagent:' + role)"
              class="w-full text-left px-3 py-2 text-accent hover:bg-accent-soft"
            >
              ⭐ 只重跑 · {{ role }}（子Agent）
            </button>
            <div class="px-3 py-1.5 text-[11px] text-ink-soft">省 60% token</div>
          </template>
          <div v-if="!subagentRoles.length" class="px-3 py-1.5 text-[11px] text-ink-soft/60">
            无溯源信息，仅支持整体/编剧重 roll
          </div>
        </div>
      </div>

      <button @click="deleteVariant" class="px-2.5 py-1 rounded-md hover:bg-bg">🗑 删除</button>
      <button @click="explainBranch" class="px-2.5 py-1 rounded-md hover:bg-bg">📑 分支</button>

      <!-- 高玩模式额外信息 -->
      <span v-if="currentVariant.provenance" class="ml-auto text-[11px] text-ink-soft/70">
        seed {{ currentVariant.provenance.seed }}
      </span>
    </div>

    <!-- Hint 输入弹层 -->
    <div v-if="showHintBox" class="mt-2 rounded-xl border border-accent/30 bg-accent-soft/40 p-3">
      <div class="text-xs text-ink-soft mb-2">
        附加提示（可选）：告诉 Agent 上次哪里有问题
      </div>
      <textarea
        v-model="hintInput"
        rows="2"
        placeholder="例如：角色 B 语气太冷；节奏太快；结尾太仓促…"
        class="w-full text-sm rounded-lg border border-line bg-bg px-3 py-2 resize-none focus:outline-none focus:border-accent"
      ></textarea>
      <div class="flex justify-end gap-2 mt-2">
        <button @click="cancelHint" class="px-3 py-1 text-xs rounded-md hover:bg-bg">取消</button>
        <button @click="confirmReroll" class="px-3 py-1 text-xs rounded-md bg-accent text-white hover:opacity-90">
          开始重 roll
        </button>
      </div>
    </div>
  </div>
</template>
