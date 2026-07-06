<script setup>
import { ref, computed } from 'vue'
import { formatContent } from '../utils/formatContent.js'
import BaseDropdown from './base/BaseDropdown.vue'

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
  return prov.subagent_results
    .filter(s => s.character_id)
    .map(s => ({
      id: s.character_id,        // 稳定 ID，用于 reroll target
      label: s.display_name || s.character_id,  // 显示名
    }))
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

// 分支功能（Campaign.fork）后端已实现，前端入口未就绪。
// 原分支按钮只弹"开发中"提示，属死路交互，已移除避免误导。
// 待 fork 前端入口落地后在此恢复 emit('branch', ...)。

const isUser = computed(() => props.message.role === 'user')

// user 消息重 roll（用同样 intent 重新写作）
function rerollUser() {
  emit('reroll-user', { messageId: props.message.id })
}

</script>

<template>
  <div class="group px-4 sm:px-6 py-4 rounded-xl transition-colors duration-200 hover:bg-surface/40">
    <!-- 角色标签行（阅读器化：标签轻量，靠留白区分轮次） -->
    <div class="flex items-center gap-2 mb-3">
      <span class="text-xs font-medium px-2 py-0.5 rounded-md"
        :class="isUser ? 'text-ink-soft' : 'text-accent'">
        {{ message.role_label }}
      </span>
      <!-- 版本切换条（多于1个版本才显示） -->
      <div v-if="variantCount > 1" class="flex items-center gap-1 text-xs text-ink-soft">
        <button @click="switchVariant(-1)" class="w-9 h-9 flex items-center justify-center rounded-md hover:bg-accent-soft transition-colors" aria-label="上一版本">‹</button>
        <span>{{ message.active_variant + 1 }}/{{ variantCount }}</span>
        <button @click="switchVariant(1)" class="w-9 h-9 flex items-center justify-center rounded-md hover:bg-accent-soft transition-colors" aria-label="下一版本">›</button>
        <span v-if="currentVariant.status === 'discarded'" class="text-warn ml-1">· 旧版</span>
      </div>
    </div>

    <!-- 消息正文：去气泡阅读器化。
         AI = 纯衬线文本块（无边框无底色，靠 prose-fiction + 留白），user = 弱化缩进块 -->
    <div v-if="!editing"
      class="text-[15px] leading-loose"
      :class="isUser
        ? 'text-ink-soft pl-3 border-l-2 border-line'
        : 'text-ink prose-fiction'"
    >
      <span v-html="formatContent(currentVariant.content)"></span>
    </div>

    <!-- 内联编辑模式 -->
    <div v-else class="rounded-xl border border-accent bg-surface px-4 py-3">
      <textarea
        v-model="editContent"
        rows="4"
        class="w-full text-[15px] leading-relaxed bg-transparent resize-none focus:outline-none"
      ></textarea>
      <div class="flex justify-end gap-2 mt-2">
        <button @click="cancelEdit" class="min-h-[44px] px-4 text-sm rounded-lg hover:bg-accent-soft transition-colors">取消</button>
        <button @click="saveEdit" class="min-h-[44px] px-4 text-sm rounded-lg bg-accent text-white hover:opacity-90 transition-colors">保存</button>
      </div>
    </div>

    <!-- 操作栏（user 消息：编辑 + 重 roll） -->
    <div v-if="isUser" class="flex items-center gap-2 mt-3 text-ink-soft opacity-60 sm:opacity-0 sm:group-hover:opacity-100 transition-opacity duration-200">
      <button @click="startEdit" :disabled="busy"
        class="min-h-[44px] px-3 text-sm rounded-lg hover:bg-accent-soft disabled:opacity-40 transition-colors">✏️ 编辑</button>
      <button @click="rerollUser" :disabled="busy"
        class="min-h-[44px] px-3 text-sm rounded-lg hover:bg-accent-soft disabled:opacity-40 transition-colors">🔄 重roll</button>
    </div>

    <!-- 操作栏（仅 AI 消息显示完整操作；chrome 隐退：桌面 hover 浮现，移动端半显） -->
    <div v-if="!isUser" class="flex flex-wrap items-center gap-2 mt-3 text-sm text-ink-soft opacity-60 sm:opacity-0 sm:group-hover:opacity-100 transition-opacity duration-200">
      <button @click="startEdit" class="min-h-[44px] px-3 rounded-lg hover:bg-accent-soft transition-colors">✏️ 编辑</button>
      <button @click="acceptVariant" class="min-h-[44px] px-3 rounded-lg hover:bg-accent-soft transition-colors"
        :class="currentVariant.status === 'final' ? 'text-ok' : ''">
        {{ currentVariant.status === 'final' ? '✅ 已采纳' : '☑️ 采纳' }}
      </button>

      <!-- 重 roll（BaseDropdown：click-outside + ESC 关闭） -->
      <BaseDropdown v-model="showRerollMenu" align="left" :min-width="220">
        <template #trigger>
          <button
            :disabled="busy"
            class="min-h-[44px] px-3 rounded-lg hover:bg-accent-soft disabled:opacity-40 transition-colors"
          >
            🔄 重roll ▾
          </button>
        </template>
        <template #default="{ close }">
          <button @click="close(); pickReroll('all')" class="w-full text-left min-h-[44px] px-3 hover:bg-accent-soft transition-colors">整体重 roll</button>
          <button @click="close(); pickReroll('editor')" class="w-full text-left min-h-[44px] px-3 hover:bg-accent-soft transition-colors">只重跑 · 编剧</button>
          <template v-if="subagentRoles.length">
            <div class="border-t border-line my-1"></div>
            <button
              v-for="role in subagentRoles"
              :key="role.id"
              @click="close(); pickReroll('subagent:' + role.id)"
              class="w-full text-left min-h-[44px] px-3 text-accent hover:bg-accent-soft transition-colors"
            >
              ⭐ 只重跑 · {{ role.label }}（子Agent）
            </button>
            <div class="px-3 py-1.5 text-[11px] text-ink-soft">省 60% token</div>
          </template>
          <div v-if="!subagentRoles.length" class="px-3 py-2 text-[11px] text-ink-soft/60">
            无溯源信息，仅支持整体/编剧重 roll
          </div>
        </template>
      </BaseDropdown>

      <!-- 高玩模式额外信息（seed） -->
      <span v-if="currentVariant.provenance" class="text-[11px] text-ink-soft/70">
        seed {{ currentVariant.provenance.seed }}
      </span>

      <!-- 危险操作单独右对齐，拉开间距防误触 -->
      <button @click="deleteVariant" class="min-h-[44px] px-3 rounded-lg hover:bg-err/10 text-ink-soft hover:text-err transition-colors ml-auto">🗑 删除</button>
    </div>

    <!-- Hint 输入弹层 -->
    <div v-if="showHintBox" class="mt-3 rounded-xl border border-accent/30 bg-accent-soft/40 p-3">
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
        <button @click="cancelHint" class="min-h-[44px] px-4 text-sm rounded-lg hover:bg-accent-soft transition-colors">取消</button>
        <button @click="confirmReroll" class="min-h-[44px] px-4 text-sm rounded-lg bg-accent text-white hover:opacity-90 transition-colors">
          开始重 roll
        </button>
      </div>
    </div>
  </div>
</template>
