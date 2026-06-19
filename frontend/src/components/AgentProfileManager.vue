<script setup>
import { ref, computed, onMounted } from 'vue'
import {
  listAgentProfileConfigs,
  getAgentProfileConfig,
  getActiveAgentProfileConfig,
  saveAgentProfileConfig,
  deleteAgentProfileConfig,
  setActiveAgentProfileConfig,
} from '../tauri-api.js'
import { promptDialog, confirmDialog } from './base/BaseDialog.js'

defineProps({
  // 嵌入调试抽屉时去掉外层卡片
  embedded: { type: Boolean, default: false },
})

// ─── 可配置角色（与后端 AgentRole 序列化字符串对齐）──────────────────────
// Subagent 用通配符 "*" 覆盖所有子 Agent（后端 run_config_for 支持 Subagent:* 回退）
const ROLES = [
  { key: 'Director', label: '导演' },
  { key: 'Editor', label: '编剧' },
  { key: 'Subagent:*', label: '子 Agent（通配）' },
  { key: 'Summarizer', label: '总结器' },
  { key: 'PostProcessor', label: '后处理' },
]

// 这些角色有可用工具，tool_whitelist 才有意义
const ROLES_WITH_TOOLS = new Set(['Director', 'Subagent:*', 'PostProcessor'])
const KNOWN_TOOLS = {
  Director: ['search_world_info', 'get_character', 'emit_plan', 'search_vectors', 'get_recent_summary'],
  'Subagent:*': ['get_character'],
  PostProcessor: ['emit_postprocess'],
}

const summaries = ref([]) // AgentProfileConfigSummaryDto[]
const editing = ref(null) // 当前编辑的完整 config（深拷贝）
const loading = ref(false)
const saving = ref(false)
const errorMsg = ref('')

const expanded = ref(false) // 折叠面板（默认收起，避免占太多右侧空间）

const activeId = computed(() => summaries.value.find((s) => s.is_active)?.id || null)

onMounted(async () => {
  await refresh()
})

async function refresh() {
  loading.value = true
  errorMsg.value = ''
  try {
    summaries.value = (await listAgentProfileConfigs()) || []
    // 默认选中 active 的完整配置用于展示
    if (!editing.value && summaries.value.length) {
      const active = summaries.value.find((s) => s.is_active)
      const target = active || summaries.value[0]
      editing.value = await getAgentProfileConfig(target.id)
    }
  } catch (e) {
    errorMsg.value = '加载失败: ' + e
  } finally {
    loading.value = false
  }
}

async function selectProfile(id) {
  try {
    editing.value = await getAgentProfileConfig(id)
  } catch (e) {
    errorMsg.value = '读取配置失败: ' + e
  }
}

async function makeActive(id) {
  saving.value = true
  errorMsg.value = ''
  try {
    await setActiveAgentProfileConfig(id)
    await refresh()
  } catch (e) {
    errorMsg.value = '切换活跃配置失败: ' + e
  } finally {
    saving.value = false
  }
}

/// 复制 built-in（或任意）配置为新 custom profile
async function duplicate(id) {
  try {
    const src = await getAgentProfileConfig(id)
    if (!src) return
    const name = await promptDialog('新配置名称：', `${src.name} 副本`, { title: '复制配置' })
    if (!name) return
    const newConfig = {
      ...src,
      id: `profile-${Date.now()}`,
      name,
      description: src.description || '',
      source: 'UserCreated',
      config_version: 1,
    }
    await saveAgentProfileConfig(JSON.stringify(newConfig))
    await refresh()
    editing.value = await getAgentProfileConfig(newConfig.id)
  } catch (e) {
    errorMsg.value = '复制失败: ' + e
  }
}

async function removeProfile(id) {
  const s = summaries.value.find((x) => x.id === id)
  if (s && s.source === 'BuiltIn') {
    errorMsg.value = '内置默认配置不可删除'
    return
  }
  if (!await confirmDialog('确认删除该配置？', { title: '删除确认' })) return
  saving.value = true
  try {
    await deleteAgentProfileConfig(id)
    if (editing.value && editing.value.id === id) editing.value = null
    await refresh()
  } catch (e) {
    errorMsg.value = '删除失败: ' + e
  } finally {
    saving.value = false
  }
}

async function save() {
  if (!editing.value) return
  saving.value = true
  errorMsg.value = ''
  try {
    const cfg = editing.value
    if (cfg.source === 'BuiltIn' || isBuiltinId(cfg.id)) {
      errorMsg.value = '内置默认配置不可覆盖，请先「复制为新配置」再编辑'
      return
    }
    // 清洗：空字符串 model_override 转 null；空白名单行忽略
    const cleaned = cleanForSave(cfg)
    await saveAgentProfileConfig(JSON.stringify(cleaned))
    editing.value = await getAgentProfileConfig(cleaned.id)
    await refresh()
  } catch (e) {
    errorMsg.value = '保存失败: ' + e
  } finally {
    saving.value = false
  }
}

function isBuiltinId(id) {
  return id === 'builtin-default-agent-v1'
}
function isBuiltin(cfg) {
  return cfg && (cfg.source === 'BuiltIn' || isBuiltinId(cfg.id))
}

/// 保存前清洗：空串→null，移除完全空的角色条目
function cleanForSave(cfg) {
  const out = { ...cfg }
  const cleanedConfigs = {}
  for (const role of Object.keys(cfg.agent_configs || {})) {
    const c = cfg.agent_configs[role]
    const model = (c.model_override || '').trim()
    const rounds = c.max_tool_rounds === '' || c.max_tool_rounds === null ? null : Number(c.max_tool_rounds)
    const wl = parseWhitelist(c.tool_whitelistRaw)
    // 全空则不存该角色（让后端用默认）
    if (!model && rounds === null && wl === null) continue
    cleanedConfigs[role] = {
      model_override: model || null,
      max_tool_rounds: rounds,
      tool_whitelist: wl,
    }
  }
  out.agent_configs = cleanedConfigs
  out.max_concurrent_subagents = Number(out.max_concurrent_subagents) || 1
  delete out.tool_whitelistRaw
  return out
}

/// tool_whitelist 输入（逗号分隔字符串）→ Option<Vec<String>>
function parseWhitelist(raw) {
  if (raw === null || raw === undefined) return null
  const s = String(raw).trim()
  if (s === '') return [] // 空输入 = 禁用全部工具（Some([])）
  const names = s.split(',').map((x) => x.trim()).filter(Boolean)
  return names
}

/// 把后端 tool_whitelist 反向渲染成输入框可编辑的逗号分隔字符串
function whitelistToText(role) {
  const cfg = editing.value
  if (!cfg || !cfg.agent_configs || !cfg.agent_configs[role]) return ''
  const wl = cfg.agent_configs[role].tool_whitelist
  if (wl === null || wl === undefined) return ''
  return wl.join(', ')
}

/// 确保某角色的 agent_config 条目存在（编辑时按需创建）
function ensureRole(role) {
  if (!editing.value) return
  if (!editing.value.agent_configs) editing.value.agent_configs = {}
  if (!editing.value.agent_configs[role]) {
    editing.value.agent_configs[role] = {
      model_override: null,
      max_tool_rounds: null,
      tool_whitelist: null,
      tool_whitelistRaw: '',
    }
  }
}

function onWhitelistInput(role, text) {
  ensureRole(role)
  editing.value.agent_configs[role].tool_whitelistRaw = text
}
function onModelInput(role, text) {
  ensureRole(role)
  editing.value.agent_configs[role].model_override = text
}
function onRoundsInput(role, val) {
  ensureRole(role)
  editing.value.agent_configs[role].max_tool_rounds = val === '' ? null : val
}

function getModel(role) {
  const c = editing.value?.agent_configs?.[role]
  return c ? (c.model_override ?? '') : ''
}
function getRounds(role) {
  const c = editing.value?.agent_configs?.[role]
  return c ? (c.max_tool_rounds ?? '') : ''
}
function getWhitelistRaw(role) {
  // 优先编辑中的临时字段，否则从已保存的 tool_whitelist 反渲染
  const c = editing.value?.agent_configs?.[role]
  if (!c) return ''
  if (c.tool_whitelistRaw !== undefined) return c.tool_whitelistRaw
  if (c.tool_whitelist === null || c.tool_whitelist === undefined) return ''
  return c.tool_whitelist.join(', ')
}

function readonly() {
  return isBuiltin(editing.value)
}
</script>

<template>
  <div :class="embedded ? '' : 'mx-4 mt-3 bg-surface rounded-2xl border border-line p-4'">
    <!-- 标题栏 -->
    <div class="flex items-center gap-2 mb-2">
      <span class="text-base">🧩</span>
      <span class="text-sm font-medium text-ink">Agent Profile 配置</span>
      <button
        @click="expanded = !expanded"
        class="ml-auto text-xs text-ink-soft hover:text-ink"
      >{{ expanded ? '收起 ▴' : '展开 ▾' }}</button>
    </div>

    <div v-if="errorMsg" class="mb-2 text-xs text-err">{{ errorMsg }}</div>

    <div v-show="expanded">
      <!-- 列表 + 活跃切换 -->
      <div v-if="loading" class="text-center text-ink-soft text-xs py-2">加载中…</div>
      <div v-else class="space-y-1 mb-3">
        <div
          v-for="s in summaries"
          :key="s.id"
          class="flex items-center gap-2 px-2 py-1.5 rounded-lg"
          :class="s.id === editing?.id ? 'bg-accent-soft' : 'hover:bg-line/40'"
        >
          <button
            @click="selectProfile(s.id)"
            class="flex-1 text-left text-sm"
            :class="s.id === editing?.id ? 'text-accent font-medium' : 'text-ink'"
          >
            {{ s.name }}
            <span v-if="s.source === 'BuiltIn'" class="text-[10px] text-ink-soft ml-1">内置</span>
            <span v-if="s.is_active" class="text-[10px] text-accent ml-1">● 活跃</span>
          </button>
          <button
            v-if="!s.is_active"
            @click="makeActive(s.id)"
            class="text-[11px] text-accent hover:underline"
          >设为活跃</button>
          <button
            @click="duplicate(s.id)"
            class="text-[11px] text-ink-soft hover:text-ink"
            title="复制为新配置"
          >复制</button>
          <button
            v-if="s.source !== 'BuiltIn'"
            @click="removeProfile(s.id)"
            class="text-[11px] text-err/70 hover:text-err"
            title="删除"
          >删除</button>
        </div>
      </div>

      <!-- 编辑区 -->
      <div v-if="editing" class="space-y-3 border-t border-line pt-3">
        <div v-if="readonly()" class="text-[11px] text-warn bg-warn/10 rounded-lg px-2 py-1.5">
          这是内置默认配置（只读）。点上方「复制」可创建可编辑副本。
        </div>

        <!-- 基础字段 -->
        <div class="space-y-2">
          <div>
            <label class="text-[11px] text-ink-soft block mb-0.5">名称</label>
            <input
              v-model="editing.name"
              :disabled="readonly()"
              class="w-full px-2.5 py-1.5 bg-bg rounded-lg text-sm border border-line focus:border-accent outline-none disabled:opacity-60"
            />
          </div>
          <div>
            <label class="text-[11px] text-ink-soft block mb-0.5">描述</label>
            <input
              v-model="editing.description"
              :disabled="readonly()"
              class="w-full px-2.5 py-1.5 bg-bg rounded-lg text-sm border border-line focus:border-accent outline-none disabled:opacity-60"
            />
          </div>
          <div class="grid grid-cols-3 gap-2">
            <div>
              <label class="text-[11px] text-ink-soft block mb-0.5">子 Agent 并发</label>
              <input
                type="number"
                min="1"
                v-model.number="editing.max_concurrent_subagents"
                :disabled="readonly()"
                class="w-full px-2.5 py-1.5 bg-bg rounded-lg text-sm border border-line focus:border-accent outline-none disabled:opacity-60"
              />
            </div>
            <div>
              <label class="text-[11px] text-ink-soft block mb-0.5">后处理</label>
              <button
                @click="!readonly() && (editing.enable_postprocess = !editing.enable_postprocess)"
                :disabled="readonly()"
                class="w-full px-2.5 py-1.5 rounded-lg text-sm border"
                :class="editing.enable_postprocess
                  ? 'bg-accent text-white border-accent'
                  : 'bg-bg text-ink-soft border-line'"
              >{{ editing.enable_postprocess ? '开' : '关' }}</button>
            </div>
            <div>
              <label class="text-[11px] text-ink-soft block mb-0.5">剧情总结</label>
              <button
                @click="!readonly() && (editing.enable_summarizer = !editing.enable_summarizer)"
                :disabled="readonly()"
                class="w-full px-2.5 py-1.5 rounded-lg text-sm border"
                :class="editing.enable_summarizer
                  ? 'bg-accent text-white border-accent'
                  : 'bg-bg text-ink-soft border-line'"
              >{{ editing.enable_summarizer ? '开' : '关' }}</button>
            </div>
          </div>
        </div>

        <!-- 各角色运行参数 -->
        <div class="border-t border-line pt-2">
          <div class="text-[11px] text-ink-soft mb-1.5">各角色运行参数覆盖（留空 = 用默认）</div>
          <div class="space-y-2">
            <div
              v-for="role in ROLES"
              :key="role.key"
              class="bg-bg rounded-lg p-2 border border-line/60"
            >
              <div class="text-xs text-ink font-medium mb-1.5">{{ role.label }} <span class="text-ink-soft">({{ role.key }})</span></div>
              <div class="grid grid-cols-2 gap-2">
                <div>
                  <label class="text-[10px] text-ink-soft block">模型覆盖</label>
                  <input
                    :value="getModel(role.key)"
                    @input="onModelInput(role.key, $event.target.value)"
                    :disabled="readonly()"
                    :placeholder="'默认模型'"
                    class="w-full px-2 py-1 bg-surface rounded text-xs border border-line/60 outline-none focus:border-accent disabled:opacity-60"
                  />
                </div>
                <div>
                  <label class="text-[10px] text-ink-soft block">最大工具轮次</label>
                  <input
                    type="number"
                    min="1"
                    :value="getRounds(role.key)"
                    @input="onRoundsInput(role.key, $event.target.value)"
                    :disabled="readonly()"
                    :placeholder="'默认'"
                    class="w-full px-2 py-1 bg-surface rounded text-xs border border-line/60 outline-none focus:border-accent disabled:opacity-60"
                  />
                </div>
              </div>
              <div v-if="ROLES_WITH_TOOLS.has(role.key)" class="mt-1.5">
                <label class="text-[10px] text-ink-soft block">
                  工具白名单（逗号分隔；留空=默认全部；<code class="text-accent">不填任何值保存=禁用全部</code>）
                </label>
                <input
                  :value="getWhitelistRaw(role.key)"
                  @input="onWhitelistInput(role.key, $event.target.value)"
                  :disabled="readonly()"
                  :placeholder="(KNOWN_TOOLS[role.key] || []).join(', ')"
                  class="w-full px-2 py-1 bg-surface rounded text-xs border border-line/60 outline-none focus:border-accent disabled:opacity-60"
                />
              </div>
            </div>
          </div>
        </div>

        <!-- 保存 -->
        <div class="flex gap-2 pt-1">
          <button
            v-if="!readonly()"
            @click="save"
            :disabled="saving"
            class="flex-1 py-1.5 text-xs rounded-lg bg-accent text-white hover:opacity-90 disabled:opacity-50"
          >{{ saving ? '保存中…' : '💾 保存' }}</button>
          <button
            v-if="readonly()"
            @click="duplicate(editing.id)"
            class="flex-1 py-1.5 text-xs rounded-lg border border-dashed border-accent-border text-accent hover:bg-accent-soft"
          >📋 复制为新配置</button>
        </div>
      </div>
    </div>
  </div>
</template>
