<script setup>
/**
 * MessageItem — 稿纸小节（重设计，纯展示）。
 *
 * 结构变化（vs components-v2/writing/ChatMessage.vue）：
 *   - 去聊天 chrome：用户消息=淡色意图注；助手消息=文稿正文段
 *   - 变体：并排卡（VariantStrip）；操作：小节底部悬停浮现
 *   - 编辑/重 roll 提示内联展开
 *
 * 契约：emit 8 个事件，名称/payload 与 useMessageVariants 完全对齐。
 */
import { ref, computed } from 'vue'
import VariantStrip from './VariantStrip.vue'

const props = defineProps({
  message: { type: Object, required: true },
  busy: { type: Boolean, default: false },
  canBranch: { type: Boolean, default: false },
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
const seed = computed(() => currentVariant.value?.provenance?.seed)
const hasVariants = computed(() => (props.message.variants?.length || 0) > 1)

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

// ── 重 roll 附加提示 ──
const showHint = ref(false)
const hint = ref('')
function confirmReroll() {
  showHint.value = false
  emit('reroll', {
    messageId: props.message.id,
    nodeId: props.message.id,
    kind: 'all',
    hint: hint.value.trim() || null,
  })
  hint.value = ''
}

function paragraphs(text) {
  return text.split(/\n{2,}/).filter(Boolean)
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
    <p class="mt-2 text-center text-[13px] leading-relaxed text-ink-faint">{{ displayContent }}</p>

    <!-- 意图操作（悬停） -->
    <div class="mt-1 flex justify-center gap-0.5 text-xs text-ink-faint transition-opacity duration-200 opacity-100 sm:opacity-0 sm:group-hover:opacity-100">
      <button @click="startEdit" :disabled="busy" class="min-h-7 px-2 rounded-md hover:bg-accent-soft hover:text-accent-bright disabled:opacity-40 transition-colors">编辑</button>
      <button @click="emit('reroll-user', { messageId: message.id })" :disabled="busy" class="min-h-7 px-2 rounded-md hover:bg-accent-soft hover:text-accent-bright disabled:opacity-40 transition-colors">重 roll</button>
      <button @click="emit('delete-variant', { nodeId: message.id })" :disabled="busy" class="min-h-7 px-2 rounded-md hover:bg-err/10 hover:text-err disabled:opacity-40 transition-colors">删除</button>
    </div>

    <!-- 编辑意图 -->
    <div v-if="editing" class="mt-2 rounded-lg border border-accent-border bg-surface p-4 shadow-card">
      <textarea v-model="editContent" rows="3" class="w-full bg-transparent resize-none text-sm leading-relaxed text-ink focus:outline-none"></textarea>
      <div class="flex justify-end gap-2 mt-2">
        <button @click="editing = false" class="min-h-8 px-3 text-[13px] rounded-md text-ink-soft hover:bg-surface-2 transition-colors">取消</button>
        <button @click="saveEdit" class="min-h-8 px-3 text-[13px] rounded-md bg-accent text-white hover:bg-accent-bright transition-colors">保存</button>
      </div>
    </div>
  </section>

  <!-- ── 助手成文：文稿小节 ── -->
  <section v-else class="group py-5">
    <div v-if="!editing" class="prose-fiction text-[15.5px] text-ink">
      <p v-for="(p, i) in paragraphs(displayContent)" :key="i" class="mb-4 last:mb-0">{{ p }}</p>
    </div>

    <!-- 内联编辑 -->
    <div v-else class="rounded-lg border border-accent-border bg-surface p-4 shadow-card">
      <textarea v-model="editContent" rows="6" class="w-full bg-transparent resize-none text-[15px] leading-relaxed text-ink focus:outline-none"></textarea>
      <div class="flex justify-end gap-2 mt-3">
        <button @click="editing = false" class="min-h-8 px-3 text-[13px] rounded-md text-ink-soft hover:bg-surface-2 transition-colors">取消</button>
        <button @click="saveEdit" class="min-h-8 px-3 text-[13px] rounded-md bg-accent text-white hover:bg-accent-bright transition-colors">保存</button>
      </div>
    </div>

    <!-- 变体卡 -->
    <VariantStrip
      v-if="hasVariants && !editing"
      :message="message"
      :busy="busy"
      @switch-variant="emit('switch-variant', $event)"
      @accept-variant="emit('accept-variant', $event)"
    />

    <!-- 小节操作行（悬停浮现；role/seed 降为 faint 元信息） -->
    <footer
      v-if="!editing"
      class="mt-3 flex items-center gap-0.5 text-xs text-ink-soft transition-opacity duration-200 opacity-100 sm:opacity-0 sm:group-hover:opacity-100"
    >
      <span class="text-ink-faint mr-1">{{ message.role_label }}<template v-if="seed != null"> · seed {{ seed }}</template></span>
      <button @click="startEdit" :disabled="busy" class="min-h-7 px-2 rounded-md hover:bg-accent-soft hover:text-accent-bright disabled:opacity-40 transition-colors">编辑</button>
      <button @click="showHint = !showHint" :disabled="busy" class="min-h-7 px-2 rounded-md hover:bg-accent-soft hover:text-accent-bright disabled:opacity-40 transition-colors">重 roll</button>
      <button @click="emit('add-variant', { messageId: message.id })" :disabled="busy" class="min-h-7 px-2 rounded-md hover:bg-accent-soft hover:text-accent-bright disabled:opacity-40 transition-colors">添变体</button>
      <button v-if="canBranch" @click="emit('branch', { nodeId: message.id })" :disabled="busy" class="min-h-7 px-2 rounded-md hover:bg-accent-soft hover:text-accent-bright disabled:opacity-40 transition-colors">分支</button>
      <button @click="emit('delete-variant', { nodeId: message.id })" :disabled="busy" class="min-h-7 px-2 rounded-md text-ink-faint hover:bg-err/10 hover:text-err disabled:opacity-40 transition-colors ml-auto">删除</button>
    </footer>

    <!-- 重 roll 附加提示 -->
    <div v-if="showHint" class="mt-3 rounded-lg border border-accent-border bg-accent-soft/40 p-3">
      <div class="text-xs text-ink-soft mb-2">附加提示（可选）：告诉 Agent 上次哪里不满意</div>
      <textarea v-model="hint" rows="2" placeholder="例如：节奏太快；结尾太仓促；角色语气不对…" class="w-full text-sm rounded-md border border-line bg-surface px-3 py-2 resize-none focus:outline-none focus:border-accent-border"></textarea>
      <div class="flex justify-end gap-2 mt-2">
        <button @click="showHint = false" class="min-h-8 px-3 text-[13px] rounded-md text-ink-soft hover:bg-surface-2 transition-colors">取消</button>
        <button @click="confirmReroll" :disabled="busy" class="min-h-8 px-3 text-[13px] rounded-md bg-accent text-white hover:bg-accent-bright disabled:opacity-40 transition-colors">开始重 roll</button>
      </div>
    </div>
  </section>
</template>
