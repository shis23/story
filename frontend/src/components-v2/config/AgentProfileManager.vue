<script setup>
import { ref, computed, onMounted } from 'vue'
import {
  listAgentProfileConfigs,
  getAgentProfileConfig,
  saveAgentProfileConfig,
  exportAgentProfileConfig,
  importAgentProfileConfig,
  deleteAgentProfileConfig,
  setActiveAgentProfileConfig,
} from '../../tauri-api.js'
import { promptDialog, confirmDialog } from '../../components/base/BaseDialog.js'
import { cleanForSave, safeFileName } from '../../utils/agentProfileConfig.js'
import { errorText } from '../../utils/errorText.js'
import PanelHost from '../shell/PanelHost.vue'
import Tabs from '../ui/Tabs.vue'
import Input from '../ui/Input.vue'
import Toggle from '../ui/Toggle.vue'
import Button from '../ui/Button.vue'
import Badge from '../ui/Badge.vue'
import LoadingState from '../ui/LoadingState.vue'
import EmptyState from '../ui/EmptyState.vue'

const props = defineProps({
  // 嵌入调试抽屉时去掉外层卡片
  embedded: { type: Boolean, default: false },
})

const emit = defineEmits(['close'])

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

// ─── 内部 Tab：list / edit ───
const activeTab = ref('list')
const innerTabs = [
  { key: 'list', label: '配置列表' },
  { key: 'edit', label: '编辑' },
]

const summaries = ref([]) // AgentProfileConfigSummaryDto[]
const editing = ref(null) // 当前编辑的完整 config（深拷贝）
const loading = ref(false)
const saving = ref(false)
const errorMsg = ref('')

const activeId = computed(() => summaries.value.find((s) => s.is_active)?.id || null)
void activeId // 保留用于将来展示活跃态汇总

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
    errorMsg.value = '加载失败: ' + errorText(e)
  } finally {
    loading.value = false
  }
}

async function selectProfile(id) {
  try {
    editing.value = await getAgentProfileConfig(id)
    activeTab.value = 'edit'
  } catch (e) {
    errorMsg.value = '读取配置失败: ' + errorText(e)
  }
}

async function makeActive(id) {
  saving.value = true
  errorMsg.value = ''
  try {
    await setActiveAgentProfileConfig(id)
    await refresh()
  } catch (e) {
    errorMsg.value = '切换活跃配置失败: ' + errorText(e)
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
      id: `profile-${crypto.randomUUID()}`,
      name,
      description: src.description || '',
      source: 'UserCreated',
      config_version: 1,
    }
    await saveAgentProfileConfig(JSON.stringify(newConfig))
    await refresh()
    editing.value = await getAgentProfileConfig(newConfig.id)
    activeTab.value = 'edit'
  } catch (e) {
    errorMsg.value = '复制失败: ' + errorText(e)
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
    errorMsg.value = '删除失败: ' + errorText(e)
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
    errorMsg.value = '保存失败: ' + errorText(e)
  } finally {
    saving.value = false
  }
}

async function exportProfile() {
  if (!editing.value) return
  saving.value = true
  errorMsg.value = ''
  try {
    const json = await exportAgentProfileConfig(editing.value.id)
    const safeName = safeFileName(editing.value.name || editing.value.id)
    const { save: saveDialog } = await import('@tauri-apps/plugin-dialog')
    const filePath = await saveDialog({
      defaultPath: `${safeName}.agent-profile.json`,
      filters: [{ name: 'Agent Profile JSON', extensions: ['json'] }],
    })
    if (!filePath) return

    const { writeFile } = await import('@tauri-apps/plugin-fs')
    await writeFile(filePath, new TextEncoder().encode(json))
  } catch (e) {
    errorMsg.value = '导出失败: ' + errorText(e)
  } finally {
    saving.value = false
  }
}

async function importProfile() {
  saving.value = true
  errorMsg.value = ''
  try {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const filePath = await open({
      multiple: false,
      filters: [{ name: 'Agent Profile JSON', extensions: ['json'] }],
    })
    if (!filePath) return

    const { readTextFile } = await import('@tauri-apps/plugin-fs')
    const configJson = await readTextFile(filePath)
    const imported = await importAgentProfileConfig(configJson)
    await refresh()
    if (imported?.id) {
      editing.value = await getAgentProfileConfig(imported.id)
      activeTab.value = 'edit'
    }
  } catch (e) {
    errorMsg.value = '导入失败: ' + errorText(e)
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
  <component
    :is="embedded ? 'div' : PanelHost"
    v-bind="embedded ? {} : { show: true, title: 'Agent Profile 配置', side: 'left' }"
    v-on="embedded ? {} : { close: () => emit('close') }"
  >
    <div :class="embedded ? '' : 'p-4'">
      <!-- 标题栏 -->
      <div class="flex items-center gap-2 mb-2">
        <span class="text-base">🧩</span>
        <span class="text-sm font-medium text-ink">Agent Profile 配置</span>
      </div>

      <div v-if="errorMsg" class="mb-2 text-xs text-err">{{ errorMsg }}</div>

      <!-- 内层 Tabs：list / edit -->
      <Tabs v-model="activeTab" :tabs="innerTabs">
        <!-- ═══ Tab: 列表 ═══ -->
        <div v-if="activeTab === 'list'" class="space-y-2">
          <div class="flex justify-end">
            <Button
              variant="default"
              size="sm"
              :loading="saving"
              @click="importProfile"
            >导入 JSON</Button>
          </div>

          <LoadingState v-if="loading" />

          <EmptyState v-else-if="summaries.length === 0" title="无 Agent Profile 配置" />

          <div v-else class="space-y-1">
            <div
              v-for="s in summaries"
              :key="s.id"
              class="flex items-center gap-2 px-2 py-1.5 rounded-lg"
              :class="s.id === editing?.id ? 'bg-accent-soft' : 'hover:bg-surface-2'"
            >
              <button
                @click="selectProfile(s.id)"
                class="flex-1 min-w-0 text-left text-sm flex items-center gap-1.5"
                :class="s.id === editing?.id ? 'text-accent font-medium' : 'text-ink'"
              >
                <span class="truncate">{{ s.name }}</span>
                <Badge v-if="s.source === 'BuiltIn'" variant="neutral" size="sm">内置</Badge>
                <Badge v-if="s.is_active" variant="accent" size="sm">● 活跃</Badge>
              </button>
              <Button
                v-if="!s.is_active"
                variant="ghost"
                size="sm"
                @click="makeActive(s.id)"
              >设为活跃</Button>
              <Button
                variant="ghost"
                size="sm"
                title="复制为新配置"
                @click="duplicate(s.id)"
              >复制</Button>
              <Button
                v-if="s.source !== 'BuiltIn'"
                variant="ghost"
                size="sm"
                title="删除"
                @click="removeProfile(s.id)"
              >删除</Button>
            </div>
          </div>

          <Button
            v-if="editing"
            variant="default"
            size="md"
            class="w-full"
            @click="activeTab = 'edit'"
          >编辑「{{ editing.name }}」→</Button>
        </div>

        <!-- ═══ Tab: 编辑（不在编辑状态时给空提示） ═══ -->
        <div v-if="activeTab === 'edit'" class="space-y-3">
          <EmptyState v-if="!editing" title="未选择配置">
            <template #action>
              <Button variant="primary" size="md" @click="activeTab = 'list'">返回列表</Button>
            </template>
          </EmptyState>

          <template v-else>
            <div v-if="readonly()" class="text-[11px] text-warn bg-warn/10 rounded-lg px-2 py-1.5">
              这是内置默认配置（只读）。点「复制为新配置」可创建可编辑副本。
            </div>

            <!-- 基础字段 -->
            <div class="space-y-2">
              <div class="space-y-1">
                <label class="text-[11px] text-ink-soft block">名称</label>
                <Input v-model="editing.name" :disabled="readonly()" />
              </div>
              <div class="space-y-1">
                <label class="text-[11px] text-ink-soft block">描述</label>
                <Input v-model="editing.description" :disabled="readonly()" />
              </div>
              <div class="grid grid-cols-3 gap-2">
                <div class="space-y-1">
                  <label class="text-[11px] text-ink-soft block">子 Agent 并发</label>
                  <input
                    type="number"
                    min="1"
                    v-model.number="editing.max_concurrent_subagents"
                    :disabled="readonly()"
                    class="w-full bg-surface-2 border border-line rounded-lg px-2.5 py-1.5 text-sm text-ink focus:border-accent outline-none disabled:opacity-60"
                  />
                </div>
                <div class="space-y-1">
                  <label class="text-[11px] text-ink-soft block">后处理</label>
                  <Toggle
                    :model-value="!!editing.enable_postprocess"
                    :disabled="readonly()"
                    @update:model-value="readonly() ? null : (editing.enable_postprocess = $event)"
                  />
                </div>
                <div class="space-y-1">
                  <label class="text-[11px] text-ink-soft block">剧情总结</label>
                  <Toggle
                    :model-value="!!editing.enable_summarizer"
                    :disabled="readonly()"
                    @update:model-value="readonly() ? null : (editing.enable_summarizer = $event)"
                  />
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
                  class="bg-surface-2 rounded-lg p-2 border border-line"
                >
                  <div class="text-xs text-ink font-medium mb-1.5">
                    {{ role.label }}
                    <span class="text-ink-soft">({{ role.key }})</span>
                  </div>
                  <div class="grid grid-cols-2 gap-2">
                    <div class="space-y-1">
                      <label class="text-[10px] text-ink-soft block">模型覆盖</label>
                      <Input
                        :model-value="getModel(role.key)"
                        :placeholder="'默认模型'"
                        :disabled="readonly()"
                        @update:model-value="onModelInput(role.key, $event)"
                      />
                    </div>
                    <div class="space-y-1">
                      <label class="text-[10px] text-ink-soft block">最大工具轮次</label>
                      <input
                        type="number"
                        min="1"
                        :value="getRounds(role.key)"
                        :disabled="readonly()"
                        :placeholder="'默认'"
                        class="w-full bg-surface-2 border border-line rounded-lg px-2 py-1 text-xs text-ink focus:border-accent outline-none disabled:opacity-60"
                        @input="onRoundsInput(role.key, $event.target.value)"
                      />
                    </div>
                  </div>
                  <div v-if="ROLES_WITH_TOOLS.has(role.key)" class="mt-1.5 space-y-1">
                    <label class="text-[10px] text-ink-soft block">
                      工具白名单（逗号分隔；留空=默认全部；<code class="text-accent">不填任何值保存=禁用全部</code>）
                    </label>
                    <Input
                      :model-value="getWhitelistRaw(role.key)"
                      :placeholder="(KNOWN_TOOLS[role.key] || []).join(', ')"
                      :disabled="readonly()"
                      @update:model-value="onWhitelistInput(role.key, $event)"
                    />
                  </div>
                </div>
              </div>
            </div>

            <!-- 保存 -->
            <div class="flex gap-2 pt-1">
              <Button
                v-if="!readonly()"
                variant="primary"
                size="md"
                class="flex-1"
                :loading="saving"
                @click="save"
              >{{ saving ? '保存中…' : '保存' }}</Button>
              <Button
                v-if="readonly()"
                variant="default"
                size="md"
                class="flex-1 border-dashed"
                @click="duplicate(editing.id)"
              >复制为新配置</Button>
              <Button
                variant="default"
                size="md"
                class="flex-1"
                :loading="saving"
                @click="exportProfile"
              >导出 JSON</Button>
            </div>
          </template>
        </div>
      </Tabs>
    </div>
  </component>
</template>
