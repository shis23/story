<script setup>
import { ref, onMounted, nextTick, computed } from 'vue'
import {
  metaStartConversation, metaChat, metaListPendingPatches,
  metaAcceptPatch, metaDismissPatch,
  metaAnalyzeMvuCard, metaListMvuTranslations,
  listCharacters,
} from '../tauri-api.js'

const emit = defineEmits(['close'])

// ─── 状态 ───
const conversationId = ref(null)
const messages = ref([]) // MetaMessage[]
const pendingPatches = ref([]) // Patch[]
const userInput = ref('')
const loading = ref(false)
const mvuTranslations = ref([]) // 已分析的 MVU 翻译列表
const analyzingCardId = ref(null) // 正在分析的卡 ID
const activeMvuDetail = ref(null) // 展开的 MVU 详情
const expandedPatchId = ref(null) // 展开的 patch
const error = ref('')
const messagesEl = ref(null)

// 角色卡列表（用于 MVU 分析选择）
const characters = ref([])

onMounted(async () => {
  // 初始化对话
  try {
    conversationId.value = await metaStartConversation()
  } catch (e) {
    error.value = '初始化 Meta 对话失败: ' + e
  }
  await refreshPatches()
  await refreshMvuList()
  // 加载角色卡（给 MVU 分析按钮用）
  try {
    characters.value = await listCharacters()
  } catch {}
})

async function refreshPatches() {
  try {
    pendingPatches.value = await metaListPendingPatches()
  } catch {}
}

async function refreshMvuList() {
  try {
    mvuTranslations.value = await metaListMvuTranslations()
  } catch {}
}

// ─── 对话 ───
async function handleSend() {
  const text = userInput.value.trim()
  if (!text || loading.value) return
  loading.value = true
  error.value = ''
  // 先把用户输入加进消息列表（即时反馈）
  messages.value.push({ role: 'user', content: text })
  userInput.value = ''
  await scrollToBottom()

  try {
    const result = await metaChat(conversationId.value, text)
    if (result.agent_message) {
      messages.value.push(result.agent_message)
    }
    if (result.new_patch) {
      await refreshPatches()
    }
    await scrollToBottom()
  } catch (e) {
    error.value = '对话失败: ' + e
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

// ─── Patch 操作 ───
async function handleAcceptPatch(patchId) {
  try {
    await metaAcceptPatch(patchId)
    await refreshPatches()
  } catch (e) {
    error.value = '采纳失败: ' + e
  }
}

async function handleDismissPatch(patchId) {
  try {
    await metaDismissPatch(patchId)
    await refreshPatches()
  } catch (e) {
    error.value = '忽略失败: ' + e
  }
}

// ─── MVU 分析 ───
async function handleAnalyzeMvu(cardId) {
  analyzingCardId.value = cardId
  error.value = ''
  try {
    const detail = await metaAnalyzeMvuCard(cardId)
    activeMvuDetail.value = detail
    await refreshMvuList()
  } catch (e) {
    error.value = 'MVU 分析失败: ' + e
  } finally {
    analyzingCardId.value = null
  }
}

// 工具：判断消息是否有结构化工具结果
function hasToolResult(msg) {
  return msg.tool_result && msg.tool_result.kind && msg.tool_result.kind !== 'none'
}

// 工具：patch actions 摘要
function patchActionSummary(patch) {
  if (!patch.actions || patch.actions.length === 0) return '（无操作）'
  return patch.actions.map(a => {
    if (a.Create) return `创建 ${a.Create.target}`
    if (a.Update) return `改 ${a.Update.target}.${a.Update.field}`
    if (a.Delete) return `删 ${a.Delete.target}`
    return JSON.stringify(a)
  }).join('； ')
}

// 工具：routing 显示
function routingText(routing) {
  if (!routing) return ''
  if (routing.Native || routing.kind === 'native') return '原生'
  const reason = routing.webview_reason || (routing.Hybrid && routing.Hybrid.webview_reason) || ''
  return `混合（${reason}）`
}
</script>

<template>
  <!-- 弹层外壳 -->
  <div class="fixed inset-0 z-50 bg-black/40 backdrop-blur-sm flex items-end sm:items-center justify-center" @click.self="emit('close')">
    <div class="bg-bg w-full max-w-2xl max-h-[90vh] overflow-hidden rounded-t-2xl sm:rounded-2xl border border-line flex flex-col">

      <!-- 顶栏 -->
      <div class="sticky top-0 z-10 bg-bg border-b border-line px-4 py-3 flex items-center justify-between shrink-0">
        <button @click="emit('close')" class="text-ink-soft hover:text-ink text-sm">← 返回</button>
        <span class="font-medium text-ink text-sm">🔧 Meta 配置助手</span>
        <div class="w-12"></div>
      </div>

      <!-- 工具栏（快速操作） -->
      <div class="border-b border-line px-4 py-2 flex items-center gap-2 shrink-0 overflow-x-auto bg-surface/50">
        <span class="text-xs text-ink-soft shrink-0">快速：</span>

        <!-- MVU 分析下拉 -->
        <div class="relative shrink-0">
          <details class="group">
            <summary class="cursor-pointer px-2.5 py-1 rounded-full text-xs bg-accent-soft text-accent list-none">
              📊 分析状态栏 (MVU)
            </summary>
            <div class="absolute top-full left-0 mt-1 bg-bg border border-line rounded-lg shadow-lg p-2 min-w-[200px] max-h-60 overflow-y-auto z-20">
              <div v-if="characters.length === 0" class="text-xs text-ink-soft px-2 py-1">无角色卡</div>
              <button
                v-for="c in characters" :key="c.id"
                @click="handleAnalyzeMvu(c.id)"
                :disabled="analyzingCardId === c.id"
                class="w-full text-left px-2 py-1.5 rounded text-xs hover:bg-surface disabled:opacity-50 truncate"
              >
                {{ analyzingCardId === c.id ? '⏳ ' : '' }}{{ c.name }}
              </button>
            </div>
          </details>
        </div>

        <!-- 待采纳 patch 计数 -->
        <span v-if="pendingPatches.length > 0" class="text-xs px-2 py-0.5 rounded-full bg-warn/10 text-warn shrink-0">
          {{ pendingPatches.length }} 个 Patch 待采纳
        </span>

        <span v-if="error" class="text-xs text-error shrink-0 ml-auto">{{ error }}</span>
      </div>

      <!-- 主体：左右分栏 -->
      <div class="flex-1 flex overflow-hidden">

        <!-- 左：聊天区 -->
        <div class="flex-1 flex flex-col overflow-hidden">
          <!-- 消息流 -->
          <div ref="messagesEl" class="flex-1 overflow-y-auto p-4 space-y-3">
            <div v-if="messages.length === 0" class="text-center text-ink-soft text-sm py-8">
              问 Meta 助手任何配置问题：<br>
              「看看世界书有没有冲突」「这张卡的状态栏怎么分析」
            </div>

            <div
              v-for="(msg, i) in messages" :key="i"
              class="flex"
              :class="msg.role === 'user' ? 'justify-end' : 'justify-start'"
            >
              <div
                class="max-w-[80%] rounded-2xl px-3.5 py-2 text-sm"
                :class="msg.role === 'user'
                  ? 'bg-accent text-white rounded-br-md'
                  : 'bg-surface text-ink rounded-bl-md'"
              >
                <div class="whitespace-pre-wrap">{{ msg.content }}</div>

                <!-- 内嵌工具结果（诊断报告 / MVU 摘要） -->
                <template v-if="hasToolResult(msg)">
                  <!-- 世界书诊断报告 -->
                  <div v-if="msg.tool_result.WorldInfoReport" class="mt-2 pt-2 border-t border-line/50 text-xs space-y-1">
                    <div class="font-medium text-ink">📊 世界书诊断</div>
                    <div class="text-ink-soft">
                      共 {{ msg.tool_result.WorldInfoReport.total_entries }} 条 / 蓝灯 {{ msg.tool_result.WorldInfoReport.constant_count }} / 绿灯 {{ msg.tool_result.WorldInfoReport.selective_count }}
                    </div>
                    <div v-if="msg.tool_result.WorldInfoReport.conflicts.length > 0" class="text-warn">
                      ⚠ {{ msg.tool_result.WorldInfoReport.conflicts.length }} 处冲突
                    </div>
                  </div>

                  <!-- 角色卡诊断报告 -->
                  <div v-else-if="msg.tool_result.CardReport" class="mt-2 pt-2 border-t border-line/50 text-xs space-y-1">
                    <div class="font-medium text-ink">📋 角色卡「{{ msg.tool_result.CardReport.name }}」</div>
                    <div v-if="msg.tool_result.CardReport.issues.length > 0" class="text-warn">
                      ⚠ {{ msg.tool_result.CardReport.issues.length }} 个问题
                    </div>
                    <div v-else class="text-ok">✓ 未发现问题</div>
                  </div>

                  <!-- Patch 提议卡片 -->
                  <div v-else-if="msg.tool_result.PatchProposed" class="mt-2 pt-2 border-t border-line/50 text-xs">
                    <div class="font-medium text-accent">📝 提议 Patch（待采纳）</div>
                    <div class="text-ink-soft mt-0.5">{{ msg.tool_result.PatchProposed.description }}</div>
                    <div class="text-ink-soft">{{ msg.tool_result.PatchProposed.action_count }} 个操作</div>
                  </div>
                </template>
              </div>
            </div>

            <div v-if="loading" class="flex justify-start">
              <div class="bg-surface text-ink-soft rounded-2xl rounded-bl-md px-3.5 py-2 text-sm">
                <span class="inline-block animate-pulse">●●●</span>
              </div>
            </div>
          </div>

          <!-- 输入栏 -->
          <div class="border-t border-line p-3 shrink-0">
            <div class="flex gap-2">
              <input
                v-model="userInput"
                @keyup.enter="handleSend"
                placeholder="问 Meta 助手…（如：看看世界书有没有冲突）"
                class="flex-1 px-3 py-2 text-sm rounded-lg border border-line bg-surface focus:outline-none focus:border-accent"
                :disabled="loading"
              />
              <button
                @click="handleSend"
                :disabled="loading || !userInput.trim()"
                class="px-4 py-2 rounded-lg text-sm font-medium bg-accent text-white disabled:opacity-50"
              >发送</button>
            </div>
          </div>
        </div>

        <!-- 右：Patch 列表 + MVU 列表（高玩侧栏） -->
        <div class="w-56 border-l border-line overflow-y-auto p-3 space-y-3 shrink-0 hidden sm:block">
          <!-- 待采纳 Patch -->
          <div>
            <div class="text-xs font-medium text-ink mb-2">📝 待采纳 Patch</div>
            <div v-if="pendingPatches.length === 0" class="text-xs text-ink-soft">无</div>
            <div
              v-for="patch in pendingPatches" :key="patch.id"
              class="bg-surface rounded-lg border border-line p-2 mb-2"
            >
              <div class="text-xs text-ink font-medium mb-1">{{ patch.description }}</div>
              <button
                @click="expandedPatchId = expandedPatchId === patch.id ? null : patch.id"
                class="text-[10px] text-ink-soft underline"
              >{{ expandedPatchId === patch.id ? '收起' : '查看操作' }}</button>
              <div v-if="expandedPatchId === patch.id" class="text-[10px] text-ink-soft mt-1 break-all">
                {{ patchActionSummary(patch) }}
              </div>
              <div class="flex gap-1 mt-2">
                <button
                  @click="handleAcceptPatch(patch.id)"
                  class="flex-1 py-1 rounded text-[10px] font-medium bg-ok/10 text-ok hover:bg-ok/20"
                >采纳</button>
                <button
                  @click="handleDismissPatch(patch.id)"
                  class="flex-1 py-1 rounded text-[10px] font-medium bg-ink-soft/10 text-ink-soft hover:bg-ink-soft/20"
                >忽略</button>
              </div>
            </div>
          </div>

          <!-- MVU 翻译列表 -->
          <div>
            <div class="text-xs font-medium text-ink mb-2">📊 已分析的 MVU</div>
            <div v-if="mvuTranslations.length === 0" class="text-xs text-ink-soft">无（用上方「分析状态栏」按钮触发）</div>
            <div
              v-for="m in mvuTranslations" :key="m.source_character_id"
              class="bg-surface rounded-lg border border-line p-2 mb-2 text-xs"
            >
              <div class="text-ink font-medium truncate">{{ m.character_name }}</div>
              <div class="text-ink-soft mt-0.5">
                {{ routingText(m.routing) }} · {{ m.ui_binding_count }} 绑定 · {{ m.fallback_count }} 兜底
              </div>
              <div class="text-[10px] text-ink-soft mt-0.5">置信度 {{ Math.round(m.analysis_confidence * 100) }}%</div>
            </div>
          </div>
        </div>
      </div>

      <!-- MVU 分析详情浮层 -->
      <div
        v-if="activeMvuDetail"
        class="absolute inset-0 z-30 bg-black/40 flex items-center justify-center p-4"
        @click.self="activeMvuDetail = null"
      >
        <div class="bg-bg rounded-2xl border border-line max-w-md w-full max-h-[80vh] overflow-y-auto p-4">
          <div class="flex items-center justify-between mb-3">
            <div class="font-medium text-ink text-sm">{{ activeMvuDetail.character_name }} · MVU 分析结果</div>
            <button @click="activeMvuDetail = null" class="text-ink-soft text-sm">✕</button>
          </div>

          <div class="space-y-2 text-xs">
            <!-- 路由 + 置信度 -->
            <div class="flex gap-2">
              <span class="px-2 py-0.5 rounded-full" :class="activeMvuDetail.translation.routing.kind === 'hybrid' ? 'bg-warn/10 text-warn' : 'bg-ok/10 text-ok'">
                {{ routingText(activeMvuDetail.translation.routing) }}
              </span>
              <span class="px-2 py-0.5 rounded-full bg-surface text-ink-soft">
                置信度 {{ Math.round(activeMvuDetail.analysis_confidence * 100) }}%
              </span>
            </div>

            <!-- 启发式打分 -->
            <div v-if="activeMvuDetail.complexity && activeMvuDetail.complexity.classification" class="text-ink-soft">
              启发式分类：{{ activeMvuDetail.complexity.classification }} · {{ activeMvuDetail.complexity.reasoning }}
            </div>

            <!-- 统计 -->
            <div class="grid grid-cols-2 gap-2 text-ink-soft">
              <div>变量字段：{{ activeMvuDetail.translation.variable_schema.length }}</div>
              <div>UI 绑定：{{ activeMvuDetail.translation.ui_bindings.length }}</div>
              <div>更新规则：{{ activeMvuDetail.translation.update_rules.length }}</div>
              <div>交互映射：{{ activeMvuDetail.translation.interactions.length }}</div>
              <div>兜底 JS：{{ activeMvuDetail.translation.fallback_fragments.length }}</div>
            </div>

            <!-- UI 绑定列表 -->
            <div v-if="activeMvuDetail.translation.ui_bindings.length > 0">
              <div class="font-medium text-ink mt-2 mb-1">UI 绑定</div>
              <div
                v-for="b in activeMvuDetail.translation.ui_bindings" :key="b.element"
                class="bg-surface rounded px-2 py-1 mb-1 flex justify-between"
              >
                <span class="text-ink">{{ b.element }}</span>
                <span class="text-ink-soft">{{ b.variable_key }} ({{ b.display.kind }})</span>
              </div>
            </div>

            <!-- 更新规则 -->
            <div v-if="activeMvuDetail.translation.update_rules.length > 0">
              <div class="font-medium text-ink mt-2 mb-1">更新规则（注入后处理 Agent）</div>
              <div
                v-for="(r, i) in activeMvuDetail.translation.update_rules" :key="i"
                class="bg-surface rounded px-2 py-1 mb-1 text-ink-soft"
              >{{ i + 1 }}. {{ r }}</div>
            </div>

            <!-- 兜底片段 -->
            <div v-if="activeMvuDetail.translation.fallback_fragments.length > 0">
              <div class="font-medium text-warn mt-2 mb-1">⚠ 兜底 JS（需共享 WebView，下一轮实现）</div>
              <div
                v-for="(f, i) in activeMvuDetail.translation.fallback_fragments" :key="i"
                class="bg-warn/5 border border-warn/20 rounded px-2 py-1 mb-1"
              >
                <div class="text-ink">{{ f.description }}</div>
                <div class="text-ink-soft text-[10px]">原因：{{ f.reason }}</div>
              </div>
            </div>

            <!-- 备注 -->
            <div v-if="activeMvuDetail.translation.notes.length > 0">
              <div class="font-medium text-ink mt-2 mb-1">备注</div>
              <div
                v-for="(n, i) in activeMvuDetail.translation.notes" :key="i"
                class="text-ink-soft text-[10px]"
              >• {{ n }}</div>
            </div>
          </div>
        </div>
      </div>

    </div>
  </div>
</template>
