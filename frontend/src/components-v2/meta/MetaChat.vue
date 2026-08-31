<script setup>
import { ref, onMounted, nextTick } from 'vue'
import { metaStartConversation, metaChat } from '../../tauri-api.js'
import {
  hasMetaToolResult,
  worldInfoReportFromToolResult,
  cardReportFromToolResult,
  patchProposalFromToolResult,
} from '../../utils/metaToolResults.js'
import Button from '../ui/Button.vue'
import Input from '../ui/Input.vue'
import EmptyState from '../ui/EmptyState.vue'
import { errorText } from '../../utils/errorText.js'

// Meta 与用户对话区：流式消息 + 工具结果折叠卡。
// 复用 utils/metaToolResults.js 渲染结构化结果（世界书诊断 / 角色卡诊断 / Patch 提议）。
// 当对话产生 new_patch / new_typed_patches 时冒泡给 MetaPanel 刷新对应子 tab。

const emit = defineEmits(['error', 'new-patch', 'new-typed-patches'])

// ─── 状态 ───
const conversationId = ref(null)
const messages = ref([]) // MetaMessage[]
const userInput = ref('')
const loading = ref(false)
const messagesEl = ref(null)

// ─── 初始化 ───
onMounted(async () => {
  try {
    conversationId.value = await metaStartConversation()
  } catch (e) {
    emit('error', '初始化 Meta 对话失败: ' + errorText(e))
  }
})

// ─── 发送 ───
async function handleSend() {
  const text = userInput.value.trim()
  if (!text || loading.value) return
  loading.value = true
  // 先把用户输入加进消息列表（即时反馈）。用稳定 id 作 v-for key
  messages.value.push({ id: `meta-user-${Date.now()}`, role: 'user', content: text })
  userInput.value = ''
  await scrollToBottom()

  try {
    // 流式：先 push 一条空的 agent 消息，token 增量累积进去
    const agentId = `meta-agent-${Date.now()}`
    messages.value.push({ id: agentId, role: 'agent', content: '' })
    await scrollToBottom()

    const result = await metaChat(conversationId.value, text, (delta) => {
      const msg = messages.value.find((m) => m.id === agentId)
      if (msg) {
        msg.content += delta
        scrollToBottom()
      }
    })

    // 用最终聚合结果校正（流式累积可能因工具调用产生中间文本）
    if (result.agent_message) {
      const msg = messages.value.find((m) => m.id === agentId)
      if (msg) {
        const finalContent = result.agent_message.content
        if (finalContent) msg.content = finalContent
        // 附带 tool_result（结构化结果卡渲染）
        if (result.agent_message.tool_result) {
          msg.tool_result = result.agent_message.tool_result
        }
      }
    }
    if (result.new_patch) {
      emit('new-patch')
    }
    if (result.new_typed_patches && result.new_typed_patches.length > 0) {
      // Agent 提议了 typed patch，通知 HealthCheckPanel 刷新列表
      emit('new-typed-patches')
    }
    await scrollToBottom()
  } catch (e) {
    emit('error', '对话失败: ' + errorText(e))
  } finally {
    loading.value = false
  }
}

async function scrollToBottom() {
  await nextTick()
  if (messagesEl.value) {
    messagesEl.value.scrollTop = messagesEl.value.scrollHeight
  }
}

// ─── 工具结果渲染辅助（复用 utils/metaToolResults.js）───
function hasToolResult(msg) {
  return hasMetaToolResult(msg)
}
function worldInfoResult(msg) {
  return worldInfoReportFromToolResult(msg?.tool_result)
}
function cardResult(msg) {
  return cardReportFromToolResult(msg?.tool_result)
}
function patchProposalResult(msg) {
  return patchProposalFromToolResult(msg?.tool_result)
}
</script>

<template>
  <!-- 列布局：消息占满中间，输入条始终贴抽屉底 -->
  <div class="flex flex-col h-full min-h-0 min-w-0 bg-bg">
    <div
      ref="messagesEl"
      class="sf-drawer-scroll flex-1 min-h-0 overflow-y-auto overscroll-y-contain"
    >
      <!-- 空态：在消息区垂直居中，不把输入条顶飞 -->
      <div
        v-if="messages.length === 0"
        class="h-full min-h-[200px] flex items-center justify-center px-4 py-8"
      >
        <EmptyState
          title="问 Meta 助手任何配置问题"
          description="「看看世界书有没有冲突」「这张卡的状态栏怎么分析」"
        />
      </div>

      <div v-else class="p-3 pb-4 space-y-3">
        <div
          v-for="msg in messages"
          :key="msg.id"
          class="flex"
          :class="msg.role === 'user' ? 'justify-end' : 'justify-start'"
        >
          <div
            class="max-w-[85%] rounded-2xl px-3.5 py-2 text-sm break-words"
            :class="msg.role === 'user'
              ? 'bg-accent text-bg rounded-br-md'
              : 'bg-surface-2 text-ink rounded-bl-md border border-line'"
          >
            <div class="whitespace-pre-wrap">{{ msg.content }}</div>

            <template v-if="hasToolResult(msg)">
              <div
                v-if="worldInfoResult(msg)"
                class="mt-2 pt-2 border-t border-line text-xs space-y-1"
              >
                <div class="font-medium text-ink">世界书诊断</div>
                <div class="text-ink-soft">
                  共 {{ worldInfoResult(msg).total_entries }} 条 / 蓝灯 {{ worldInfoResult(msg).constant_count }} / 绿灯 {{ worldInfoResult(msg).selective_count }}
                </div>
                <div v-if="worldInfoResult(msg).conflicts.length > 0" class="text-warn">
                  ⚠ {{ worldInfoResult(msg).conflicts.length }} 处冲突
                </div>
              </div>

              <div
                v-else-if="cardResult(msg)"
                class="mt-2 pt-2 border-t border-line text-xs space-y-1"
              >
                <div class="font-medium text-ink">角色卡「{{ cardResult(msg).name }}」</div>
                <div v-if="cardResult(msg).issues.length > 0" class="text-warn">
                  ⚠ {{ cardResult(msg).issues.length }} 个问题
                </div>
                <div v-else class="text-ok">✓ 未发现问题</div>
              </div>

              <div
                v-else-if="patchProposalResult(msg)"
                class="mt-2 pt-2 border-t border-line text-xs"
              >
                <div class="font-medium text-accent">提议 Patch（待采纳）</div>
                <div class="text-ink-soft mt-0.5">{{ patchProposalResult(msg).description }}</div>
                <div class="text-ink-soft">{{ patchProposalResult(msg).action_count }} 个操作</div>
              </div>
            </template>
          </div>
        </div>

        <div
          v-if="loading && messages.length > 0 && messages[messages.length - 1].role === 'user'"
          class="flex justify-start"
        >
          <div class="bg-surface-2 text-ink-soft rounded-2xl rounded-bl-md px-3.5 py-2 text-sm border border-line">
            <span class="inline-block animate-pulse">●●●</span>
          </div>
        </div>
      </div>
    </div>

    <!-- 输入条：固定在对话区底部（= 抽屉底） -->
    <div class="shrink-0 border-t border-line bg-surface p-3 pb-[max(0.75rem,env(safe-area-inset-bottom))]">
      <div class="flex gap-2 items-end">
        <Input
          v-model="userInput"
          placeholder="问 Meta 助手…（如：看看世界书有没有冲突）"
          :disabled="loading"
          class="flex-1 min-w-0"
          @keyup.enter="handleSend"
        />
        <Button
          variant="primary"
          class="shrink-0"
          :disabled="loading || !userInput.trim()"
          :loading="loading"
          @click="handleSend"
        >发送</Button>
      </div>
    </div>
  </div>
</template>
