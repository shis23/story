<script setup>
import { ref, onMounted, nextTick, computed } from 'vue'
import BaseOverlay from './base/BaseOverlay.vue'
import {
  metaStartConversation, metaChat, metaListPendingPatches,
  metaAcceptPatch, metaDismissPatch,
  metaAnalyzeMvuCard, metaListMvuTranslations,
  metaPreviewMvuApply, metaApplyMvuSchema,
  listCharacters, metaHealthCheck,
  metaProposeCampaignRepairs, metaListTypedPatches, metaPreviewTypedPatch,
  metaAcceptTypedPatch, metaDismissTypedPatch,
  metaExplainGeneration,
} from '../tauri-api.js'
import { formatDiffValue, routingText } from '../utils/campaignDisplay.js'
import {
  hasMetaToolResult,
  worldInfoReportFromToolResult,
  cardReportFromToolResult,
  patchProposalFromToolResult,
} from '../utils/metaToolResults.js'
import {
  acceptTypedPatchFlow,
  dismissTypedPatchFlow,
  explainGenerationFlow,
  proposeRepairsFlow,
  refreshTypedPatchesFlow,
  sortHealthIssues,
} from '../utils/metaPanelFlow.js'

const props = defineProps({
  activeCampaign: { type: Object, default: null },
  lastConversationNode: { type: Object, default: null }, // { conversation_id, node_id } — 生成溯源入口
})

const emit = defineEmits(['close', 'mvu-applied'])

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

// ─── Campaign 健康检查 ───
const healthIssues = ref([]) // HealthIssue[]
const healthLoading = ref(false)
const healthRan = ref(false) // 区分「未检查」和「检查后无问题」

// ─── 类型化修复建议（第三轮） ───
const typedPatches = ref([]) // TypedPatch[]
const patchesLoading = ref(false)
const expandedTypedPatchId = ref(null) // 展开的 typed patch id

// ─── 生成溯源 ───
const explainLoading = ref(false)
const explainResult = ref(null) // GenerationExplanation

// ─── MVU Apply 流程 ───
const applyPreviewLoading = ref(false)
const applyPreviews = ref([]) // MvuApplyPreview[]
const applyPreviewSource = ref(null) // { id, name } — 正在预览的 source character
const applyingDefId = ref(null) // 正在 apply 的 definition_id

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
  } catch (e) { console.error('加载角色卡失败:', e) }
})

async function refreshPatches() {
  try {
    pendingPatches.value = await metaListPendingPatches()
  } catch (e) { console.error('加载待采纳 Patch 失败:', e) }
}

async function refreshMvuList() {
  try {
    mvuTranslations.value = await metaListMvuTranslations()
  } catch (e) { console.error('加载 MVU 列表失败:', e) }
}

// ─── 对话 ───
async function handleSend() {
  const text = userInput.value.trim()
  if (!text || loading.value) return
  loading.value = true
  error.value = ''
  // 先把用户输入加进消息列表（即时反馈）。用稳定 id 作 v-for key，避免 index 复用错位
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
      }
    }
    if (result.new_patch) {
      await refreshPatches()
    }
    if (result.new_typed_patches && result.new_typed_patches.length > 0) {
      // Agent 提议了 typed patch，刷新列表让用户看到
      await refreshTypedPatches()
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

// ─── MVU Apply 预览 ───
async function handlePreviewMvuApply(sourceId, sourceName) {
  applyPreviewLoading.value = true
  applyPreviews.value = []
  applyPreviewSource.value = { id: sourceId, name: sourceName }
  error.value = ''
  try {
    const previews = await metaPreviewMvuApply(sourceId)
    applyPreviews.value = previews || []
  } catch (e) {
    error.value = 'MVU apply 预览失败: ' + e
    applyPreviewSource.value = null
  } finally {
    applyPreviewLoading.value = false
  }
}

// ─── MVU Apply 执行 ───
async function handleApplyMvuSchema(definitionId) {
  if (!applyPreviewSource.value) return
  applyingDefId.value = definitionId
  error.value = ''
  try {
    await metaApplyMvuSchema(applyPreviewSource.value.id, definitionId)
    // 从预览列表中移除已应用的 definition
    applyPreviews.value = applyPreviews.value.filter(p => p.definition_id !== definitionId)
    // 通知父组件刷新 Campaign 变量
    emit('mvu-applied')
  } catch (e) {
    error.value = '应用 schema 失败: ' + e
  } finally {
    applyingDefId.value = null
  }
}

function closeApplyPreview() {
  applyPreviews.value = []
  applyPreviewSource.value = null
}

// 工具：格式化 VariableField 摘要
function fieldSummary(field) {
  const type = field.value_type ? (typeof field.value_type === 'string' ? field.value_type : JSON.stringify(field.value_type)) : '?'
  const def = field.default !== undefined && field.default !== null ? JSON.stringify(field.default) : ''
  return `${field.key} (${field.label}) — ${type}${def ? ' = ' + def : ''}`
}

// ─── Campaign 健康检查 ───
async function handleHealthCheck() {
  if (!props.activeCampaign?.id) return
  healthLoading.value = true
  error.value = ''
  try {
    const result = await metaHealthCheck(props.activeCampaign.id)
    healthIssues.value = sortHealthIssues(result)
    healthRan.value = true
  } catch (e) {
    error.value = '体检失败: ' + e
  } finally {
    healthLoading.value = false
  }
}

// ─── 类型化修复建议 ───
async function handleProposeRepairs() {
  if (!props.activeCampaign?.id) return
  patchesLoading.value = true
  error.value = ''
  try {
    typedPatches.value = await proposeRepairsFlow({
      campaignId: props.activeCampaign.id,
      proposeCampaignRepairs: metaProposeCampaignRepairs,
      previewTypedPatch: metaPreviewTypedPatch,
    })
  } catch (e) {
    error.value = '生成修复方案失败: ' + e
  } finally {
    patchesLoading.value = false
  }
}

async function refreshTypedPatches() {
  try {
    typedPatches.value = await refreshTypedPatchesFlow({
      campaignId: props.activeCampaign?.id,
      listTypedPatches: metaListTypedPatches,
      previewTypedPatch: metaPreviewTypedPatch,
    })
  } catch (e) {
    // 静默失败，不影响主流程
    console.warn('刷新 typed patches 失败:', e)
  }
}

async function handleAcceptTypedPatch(patchId) {
  error.value = ''
  try {
    const result = await acceptTypedPatchFlow({
      campaignId: props.activeCampaign.id,
      patchId,
      patches: typedPatches.value,
      acceptTypedPatch: metaAcceptTypedPatch,
      refreshHealth: async () => {
        const issues = await metaHealthCheck(props.activeCampaign.id)
        return sortHealthIssues(issues)
      },
    })
    typedPatches.value = result.patches
    if (result.healthIssues) {
      healthIssues.value = result.healthIssues
      healthRan.value = true
    }
    if (result.healthError) {
      error.value = '体检失败: ' + result.healthError
    }
  } catch (e) {
    error.value = '接受修复失败: ' + e
  }
}

async function handleDismissTypedPatch(patchId) {
  try {
    typedPatches.value = await dismissTypedPatchFlow({
      patchId,
      patches: typedPatches.value,
      dismissTypedPatch: metaDismissTypedPatch,
    })
  } catch (e) {
    error.value = '忽略修复失败: ' + e
  }
}

// ─── 生成溯源 ───
async function handleExplainGeneration() {
  if (!props.lastConversationNode) return
  explainLoading.value = true
  explainResult.value = null
  error.value = ''
  try {
    explainResult.value = await explainGenerationFlow({
      lastConversationNode: props.lastConversationNode,
      explainGeneration: metaExplainGeneration,
    })
  } catch (e) {
    error.value = '生成溯源失败: ' + e
  } finally {
    explainLoading.value = false
  }
}

// 工具：判断消息是否有结构化工具结果
function hasToolResult(msg) {
  return hasMetaToolResult(msg)
}

function worldInfoToolResult(msg) {
  return worldInfoReportFromToolResult(msg?.tool_result)
}

function cardToolResult(msg) {
  return cardReportFromToolResult(msg?.tool_result)
}

function patchProposalToolResult(msg) {
  return patchProposalFromToolResult(msg?.tool_result)
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

</script>

<template>
  <!-- 弹层外壳 -->
  <BaseOverlay :model-value="true" title="🔧 Meta 配置助手" size="lg" position="left" :body-scroll="false" @close="emit('close')">

      <!-- 工具栏（快速操作） -->
      <div class="border-b border-line px-4 py-2 flex items-center gap-2 shrink-0 overflow-x-auto bg-surface/50">
        <span class="text-xs text-ink-soft shrink-0">快速：</span>

        <!-- MVU 分析下拉 -->
        <div class="relative shrink-0">
          <details class="group">
            <summary class="cursor-pointer min-h-[36px] px-3 rounded-full text-xs bg-accent-soft text-accent list-none flex items-center">
              📊 分析状态栏 (MVU)
            </summary>
            <div class="absolute top-full left-0 mt-1 bg-bg border border-line rounded-lg shadow-lg p-2 min-w-[200px] max-h-60 overflow-y-auto z-20">
              <div v-if="characters.length === 0" class="text-xs text-ink-soft px-2 py-1">无角色卡</div>
              <button
                v-for="c in characters" :key="c.id"
                @click="handleAnalyzeMvu(c.id)"
                :disabled="analyzingCardId === c.id"
                class="w-full text-left min-h-[36px] px-2 rounded text-xs hover:bg-surface disabled:opacity-50 truncate"
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

        <span v-if="error" class="text-xs text-err shrink-0 ml-auto">{{ error }}</span>
      </div>

      <!-- 主体：左右分栏 -->
      <div class="flex-1 flex min-h-0">

        <!-- 左：聊天区 -->
        <div class="flex-1 flex flex-col min-h-0">
          <!-- 消息流 -->
          <div ref="messagesEl" class="flex-1 overflow-y-auto p-4 space-y-3">
            <div v-if="messages.length === 0" class="text-center text-ink-soft text-sm py-8">
              问 Meta 助手任何配置问题：<br>
              「看看世界书有没有冲突」「这张卡的状态栏怎么分析」
            </div>

            <div
              v-for="msg in messages" :key="msg.id"
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
                  <div v-if="worldInfoToolResult(msg)" class="mt-2 pt-2 border-t border-line/50 text-xs space-y-1">
                    <div class="font-medium text-ink">📊 世界书诊断</div>
                    <div class="text-ink-soft">
                      共 {{ worldInfoToolResult(msg).total_entries }} 条 / 蓝灯 {{ worldInfoToolResult(msg).constant_count }} / 绿灯 {{ worldInfoToolResult(msg).selective_count }}
                    </div>
                    <div v-if="worldInfoToolResult(msg).conflicts.length > 0" class="text-warn">
                      ⚠ {{ worldInfoToolResult(msg).conflicts.length }} 处冲突
                    </div>
                  </div>

                  <!-- 角色卡诊断报告 -->
                  <div v-else-if="cardToolResult(msg)" class="mt-2 pt-2 border-t border-line/50 text-xs space-y-1">
                    <div class="font-medium text-ink">📋 角色卡「{{ cardToolResult(msg).name }}」</div>
                    <div v-if="cardToolResult(msg).issues.length > 0" class="text-warn">
                      ⚠ {{ cardToolResult(msg).issues.length }} 个问题
                    </div>
                    <div v-else class="text-ok">✓ 未发现问题</div>
                  </div>

                  <!-- Patch 提议卡片 -->
                  <div v-else-if="patchProposalToolResult(msg)" class="mt-2 pt-2 border-t border-line/50 text-xs">
                    <div class="font-medium text-accent">📝 提议 Patch（待采纳）</div>
                    <div class="text-ink-soft mt-0.5">{{ patchProposalToolResult(msg).description }}</div>
                    <div class="text-ink-soft">{{ patchProposalToolResult(msg).action_count }} 个操作</div>
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
                class="flex-1 min-h-[44px] px-3 text-sm rounded-lg border border-line bg-surface focus:outline-none focus:border-accent"
                :disabled="loading"
              />
              <button
                @click="handleSend"
                :disabled="loading || !userInput.trim()"
                class="min-h-[44px] px-4 rounded-lg text-sm font-medium bg-accent text-white disabled:opacity-50 transition-colors"
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
                  class="flex-1 min-h-[36px] rounded text-[10px] font-medium bg-ok/10 text-ok hover:bg-ok/20 transition-colors"
                >采纳</button>
                <button
                  @click="handleDismissPatch(patch.id)"
                  class="flex-1 min-h-[36px] rounded text-[10px] font-medium bg-ink-soft/10 text-ink-soft hover:bg-ink-soft/20 transition-colors"
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
              <button
                @click="handlePreviewMvuApply(m.source_character_id, m.character_name)"
                :disabled="applyPreviewLoading"
                class="mt-1.5 w-full min-h-[36px] rounded text-[10px] font-medium bg-accent/10 text-accent hover:bg-accent/20 disabled:opacity-40 transition-colors"
              >{{ applyPreviewLoading && applyPreviewSource?.id === m.source_character_id ? '加载中…' : '应用 Schema' }}</button>
            </div>
          </div>

          <!-- Campaign 健康检查 -->
          <div>
            <div class="text-xs font-medium text-ink mb-2">🩺 Campaign 体检</div>
            <button
              @click="handleHealthCheck"
              :disabled="healthLoading || !activeCampaign"
              class="w-full min-h-[36px] rounded text-xs font-medium bg-accent/10 text-accent hover:bg-accent/20 disabled:opacity-40 mb-2 transition-colors"
            >
              {{ healthLoading ? '检查中…' : '运行体检' }}
            </button>
            <div v-if="!activeCampaign" class="text-[10px] text-ink-soft">无活跃 Campaign</div>
            <div v-else-if="!healthRan" class="text-[10px] text-ink-soft">点击上方按钮检查数据完整性</div>
            <div v-else-if="healthIssues.length === 0" class="text-[10px] text-ok">✓ 未发现问题</div>
            <div v-else class="space-y-1.5">
              <div
                v-for="(issue, i) in healthIssues" :key="issue.category + '-' + i"
                class="rounded-lg border p-2 text-[10px]"
                :class="issue.severity === 'error'
                  ? 'border-err/30 bg-err/5'
                  : 'border-warn/30 bg-warn/5'"
              >
                <div class="flex items-center gap-1 mb-0.5">
                  <span
                    class="px-1.5 py-0.5 rounded-full text-[9px] font-medium"
                    :class="issue.severity === 'error'
                      ? 'bg-err/15 text-err'
                      : 'bg-warn/15 text-warn'"
                  >{{ issue.severity === 'error' ? 'Error' : 'Warning' }}</span>
                  <span class="text-ink-soft">{{ issue.category }}</span>
                </div>
                <div class="text-ink">{{ issue.message }}</div>
                <div v-if="issue.affected_id" class="text-ink-soft mt-0.5 break-all">ID: {{ issue.affected_id }}</div>
              </div>
            </div>
          </div>

          <!-- 🔧 修复建议（第三轮：类型化 Patch 闭环） -->
          <div>
            <div class="text-xs font-medium text-ink mb-2">🔧 修复建议</div>
            <button
              v-if="healthRan && healthIssues.length > 0"
              @click="handleProposeRepairs"
              :disabled="patchesLoading || !activeCampaign"
              class="w-full min-h-[36px] rounded text-xs font-medium bg-accent/10 text-accent hover:bg-accent/20 disabled:opacity-40 mb-2 transition-colors"
            >
              {{ patchesLoading ? '生成中…' : '生成修复方案' }}
            </button>
            <div v-if="!healthRan || healthIssues.length === 0" class="text-[10px] text-ink-soft">
              先运行体检，有问题时可生成修复方案
            </div>
            <div v-else-if="typedPatches.length === 0 && !patchesLoading" class="text-[10px] text-ink-soft">
              点击上方按钮生成修复方案
            </div>
            <div v-else class="space-y-2">
              <div
                v-for="patch in typedPatches" :key="patch.id"
                class="bg-surface rounded-lg border p-2 text-[10px]"
                :class="patch._stale ? 'border-warn/30 opacity-70' : 'border-line'"
              >
                <div class="flex items-center gap-1 mb-1">
                  <span class="px-1.5 py-0.5 rounded-full text-[9px] font-medium bg-accent/15 text-accent">
                    {{ patch.source_issue_category }}
                  </span>
                  <span v-if="patch._stale" class="text-warn text-[9px]">⚠️ 已过期</span>
                </div>
                <div class="text-ink font-medium mb-1">{{ patch.description }}</div>
                <div v-if="patch.affected_id" class="text-ink-soft mb-1 break-all">ID: {{ patch.affected_id }}</div>
                <!-- 展开/折叠 diff -->
                <button
                  @click="expandedTypedPatchId = expandedTypedPatchId === patch.id ? null : patch.id"
                  class="text-ink-soft underline"
                >{{ expandedTypedPatchId === patch.id ? '收起 diff' : '查看 diff' }}</button>
                <div v-if="expandedTypedPatchId === patch.id && patch.diff && patch.diff.length > 0" class="mt-1.5 space-y-1">
                  <div
                    v-for="(d, di) in patch.diff" :key="di"
                    class="bg-bg rounded px-1.5 py-1 border border-line/50"
                  >
                    <div class="text-ink-soft font-medium mb-0.5">{{ d.path }}</div>
                    <div class="flex gap-1 items-start">
                      <div class="flex-1 min-w-0">
                        <div class="text-[9px] text-ink-soft mb-0.5">Before</div>
                        <pre class="text-[9px] overflow-x-auto text-err/70 whitespace-pre-wrap break-all">{{ formatDiffValue(d.before) }}</pre>
                      </div>
                      <div class="text-ink-soft shrink-0 px-0.5">→</div>
                      <div class="flex-1 min-w-0">
                        <div class="text-[9px] text-ink-soft mb-0.5">After</div>
                        <pre class="text-[9px] overflow-x-auto text-ok/70 whitespace-pre-wrap break-all">{{ formatDiffValue(d.after) }}</pre>
                      </div>
                    </div>
                  </div>
                </div>
                <!-- 操作按钮 -->
                <div class="flex gap-1 mt-2">
                  <button
                    @click="handleAcceptTypedPatch(patch.id)"
                    :disabled="patch._stale"
                    class="flex-1 min-h-[36px] rounded text-[10px] font-medium bg-ok/10 text-ok hover:bg-ok/20 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                  >接受</button>
                  <button
                    @click="handleDismissTypedPatch(patch.id)"
                    class="flex-1 min-h-[36px] rounded text-[10px] font-medium bg-ink-soft/10 text-ink-soft hover:bg-ink-soft/20 transition-colors"
                  >忽略</button>
                </div>
              </div>
            </div>
          </div>

          <!-- 🔍 生成溯源（prop 就绪即可用） -->
          <div v-if="lastConversationNode">
            <div class="text-xs font-medium text-ink mb-2">🔍 生成溯源</div>
            <button
              @click="handleExplainGeneration"
              :disabled="explainLoading"
              class="w-full min-h-[36px] rounded text-xs font-medium bg-accent/10 text-accent hover:bg-accent/20 disabled:opacity-40 mb-2 transition-colors"
            >
              {{ explainLoading ? '查询中…' : '解释上一条生成' }}
            </button>
            <div v-if="explainResult" class="bg-surface rounded-lg border border-line p-2 text-[10px] space-y-1">
              <div v-if="explainResult.scene_brief" class="text-ink">
                <span class="text-ink-soft">场景：</span>{{ explainResult.scene_brief }}
              </div>
              <div v-if="explainResult.profile_id" class="text-ink-soft">
                Profile: {{ explainResult.profile_id }}
              </div>
              <div v-if="explainResult.seed !== undefined && explainResult.seed !== null" class="text-ink-soft">
                Seed: {{ explainResult.seed }}
              </div>
              <div v-if="explainResult.last_hint" class="text-ink-soft">
                Hint: {{ explainResult.last_hint }}
              </div>
              <div v-if="explainResult.subagents && explainResult.subagents.length > 0">
                <div class="font-medium text-ink mt-1 mb-0.5">子 Agent</div>
                <div
                  v-for="(sa, si) in explainResult.subagents" :key="si"
                  class="bg-bg rounded px-1.5 py-1 mb-1 border border-line/50"
                >
                  <div class="text-ink font-medium">{{ sa.display_name || 'Agent ' + (si + 1) }}</div>
                  <div v-if="sa.task_brief" class="text-ink-soft">{{ sa.task_brief }}</div>
                  <div v-if="sa.output_preview" class="text-ink-soft mt-0.5 line-clamp-3">{{ sa.output_preview }}</div>
                </div>
              </div>
            </div>
          </div>

        </div>
      </div>

      <!-- MVU 分析详情浮层 -->
      <BaseOverlay
        v-if="activeMvuDetail"
        :model-value="true"
        :title="activeMvuDetail.character_name + ' · MVU 分析结果'"
        size="sm"
        position="center"
        @close="activeMvuDetail = null"
      >
        <div class="p-4 space-y-2 text-xs">
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
      </BaseOverlay>

      <!-- MVU Apply 预览浮层 -->
      <BaseOverlay
        v-if="applyPreviewSource"
        :model-value="true"
        :title="'📦 ' + applyPreviewSource.name + ' · Schema 合并预览'"
        size="md"
        position="center"
        @close="closeApplyPreview"
      >
          <div v-if="applyPreviewLoading" class="p-6 text-center text-ink-soft text-sm">加载预览中…</div>
          <div v-else-if="applyPreviews.length === 0" class="p-6 text-center text-ink-soft text-sm">无可用 definition</div>
          <div v-else class="p-4 space-y-3">
            <div
              v-for="p in applyPreviews" :key="p.definition_id"
              class="bg-surface rounded-lg border border-line p-3 text-xs"
              :class="p.has_changes ? '' : 'opacity-50'"
            >
              <!-- header -->
              <div class="flex items-center justify-between mb-2">
                <div class="font-medium text-ink">
                  {{ p.character_name }}
                  <span class="text-ink-soft font-normal ml-1 text-[10px]">({{ p.definition_id.slice(0, 8) }}…)</span>
                </div>
                <span v-if="p.has_changes" class="px-1.5 py-0.5 rounded-full text-[9px] font-medium bg-accent/15 text-accent">有变更</span>
                <span v-else class="px-1.5 py-0.5 rounded-full text-[9px] font-medium bg-ok/15 text-ok">无变化</span>
              </div>

              <!-- diff 详情 -->
              <div v-if="p.has_changes" class="space-y-1.5 mb-2">
                <!-- 新增字段 -->
                <div v-if="p.added_fields.length > 0">
                  <div class="text-accent font-medium mb-0.5">+ 新增 {{ p.added_fields.length }} 个字段</div>
                  <div v-for="(f, i) in p.added_fields" :key="'a'+i" class="bg-bg rounded px-1.5 py-0.5 border border-line/50 text-[10px] text-ink-soft">
                    {{ fieldSummary(f) }}
                  </div>
                </div>
                <!-- 覆盖字段 -->
                <div v-if="p.overwritten_fields.length > 0">
                  <div class="text-warn font-medium mb-0.5">↻ 覆盖 {{ p.overwritten_fields.length }} 个字段</div>
                  <div v-for="(f, i) in p.overwritten_fields" :key="'o'+i" class="bg-bg rounded px-1.5 py-0.5 border border-line/50 text-[10px] text-ink-soft">
                    {{ fieldSummary(f) }}
                  </div>
                </div>
              </div>
              <div class="text-ink-soft text-[10px]">
                合并后共 {{ p.merged_schema.length }} 个字段 · {{ p.unchanged_count }} 个不变
              </div>

              <!-- 操作按钮 -->
              <div class="flex justify-end mt-2">
                <button
                  @click="handleApplyMvuSchema(p.definition_id)"
                  :disabled="!p.has_changes || applyingDefId === p.definition_id"
                  class="min-h-[36px] px-3 rounded text-[10px] font-medium bg-ok/10 text-ok hover:bg-ok/20 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                >{{ applyingDefId === p.definition_id ? '应用中…' : '应用' }}</button>
              </div>
            </div>
          </div>
      </BaseOverlay>

  </BaseOverlay>
</template>
