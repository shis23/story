<script setup>
import { ref, reactive, onMounted, computed } from 'vue'
import { confirmDialog } from '../../components/base/BaseDialog.js'
import {
  listConnectionTemplates,
  listConnections,
  getConnection,
  createConnection,
  updateConnection,
  deleteConnection,
  setActiveConnection,
  testConnection,
  listModels,
} from '../../tauri-api.js'
import PanelHost from '../shell/PanelHost.vue'
import Input from '../ui/Input.vue'
import Select from '../ui/Select.vue'
import Button from '../ui/Button.vue'
import Badge from '../ui/Badge.vue'
import LoadingState from '../ui/LoadingState.vue'
import EmptyState from '../ui/EmptyState.vue'
import {
  normalizeOptionalMaxTokens,
  normalizeReasoningMode,
} from '../../utils/connectionSampling.js'

const emit = defineEmits(['close', 'changed'])

// 内置模板
const templates = ref([])
// 已配置连接
const connections = ref([])
const loading = ref(true) // 初始加载模板+连接

// 表单状态
const form = reactive({
  templateId: 'deepseek',
  name: '',
  baseUrl: '',
  protocol: 'openai',
  model: '',
  apiKey: '',
  toolMode: 'native',
  temperature: 1.0,
  topP: 0.95,
  maxTokens: null,
  reasoning: 'disabled',
  // P3-3：厂商扩展参数 JSON 文本（透传到请求体顶层，如 thinking/reasoning_effort）
  extraJson: '',
})

// 编辑模式：非空时表单更新已有连接（apiKey 空 = 保留原密钥）
const editingId = ref(null)
const editingHasKey = ref(false)
const loadingEdit = ref(false)

// 默认展开：抽屉底部有采样参数，折叠时用户常以为「没有」
const showAdvanced = ref(true)
const showKey = ref(false)
const testing = ref(false)
const testResult = ref(null)
const saving = ref(false)
const error = ref('')
const extraParseError = ref('')

const isEditing = computed(() => !!editingId.value)
const formTitle = computed(() => (isEditing.value ? '编辑连接' : '新建连接'))
const saveLabel = computed(() => {
  if (saving.value) return isEditing.value ? '保存中…' : '保存中…'
  return isEditing.value ? '保存修改' : '保存'
})
const apiKeyPlaceholder = computed(() => {
  if (isEditing.value && editingHasKey.value) return '已保存密钥（留空不改）'
  if (isEditing.value) return 'sk-...（此连接尚无密钥）'
  return 'sk-...'
})

// 把 extraJson 文本解析为对象（空串=不传）。解析失败设 extraParseError 并返回 null。
function parseExtraParams() {
  const raw = (form.extraJson || '').trim()
  if (!raw) {
    extraParseError.value = ''
    return null
  }
  try {
    const obj = JSON.parse(raw)
    if (obj === null || typeof obj !== 'object' || Array.isArray(obj)) {
      extraParseError.value = '扩展参数必须是 JSON 对象（{}）'
      return null
    }
    extraParseError.value = ''
    return obj
  } catch (e) {
    extraParseError.value = 'JSON 格式错误：' + e.message
    return null
  }
}

// 在线拉取的模型列表（与模板预置合并去重）
const fetchedModels = ref([])
const fetchingModels = ref(false)

// 协议下拉
const protocolOptions = [
  { value: 'openai', label: 'OpenAI' },
  { value: 'anthropic', label: 'Anthropic' },
]

// 工具模式下拉
const toolModeOptions = [
  { value: 'native', label: '原生工具调用' },
  { value: 'text_fallback', label: '文本回退' },
]

const reasoningOptions = [
  { value: 'prompted', label: 'Prompted（角色化指引 + 必须捕获）' },
  { value: 'native', label: 'Native（厂商推理 + 必须捕获）' },
  { value: 'disabled', label: 'Disabled（不要求推理）' },
]

// 当前选中的模板对象（用于显示可选模型）
const selectedTemplate = computed(
  () => templates.value.find((t) => t.id === form.templateId) || null
)

// 合并的模型选项：模板预置 + 在线拉取（去重）
const modelOptions = computed(() => {
  const tpl = selectedTemplate.value?.models || []
  return [...new Set([...tpl, ...fetchedModels.value])].map((m) => ({ value: m, label: m }))
})

// 拉取在线模型列表
async function handleFetchModels() {
  if (!form.baseUrl || !form.apiKey) {
    error.value = '请先填完 base_url 和 api_key'
    return
  }
  fetchingModels.value = true
  error.value = ''
  try {
    const models = await listModels(form.baseUrl, form.apiKey)
    if (models.length === 0) {
      error.value = '服务商未返回模型列表，请手动输入或用模板默认'
    } else {
      fetchedModels.value = models
      // P3-2 修复：不自动填充 models[0]。<datalist> 在输入框有值时只显示前缀匹配项,
      // 自动填充会导致下拉只剩第一项（如只显示 minimax-m3）。保持输入框为空,
      // 让用户点开下拉看到全部模型再选;仅在表单完全空(新建连接且无模板默认)时
      // 才填第一个,避免空保存。
      if (!form.model) {
        form.model = models[0]
      }
    }
  } catch (e) {
    error.value = '拉取失败：' + e
  } finally {
    fetchingModels.value = false
  }
}

onMounted(async () => {
  await Promise.all([loadTemplates(), loadConnections()])
  loading.value = false
})

async function loadTemplates() {
  try {
    templates.value = await listConnectionTemplates()
    if (templates.value.length) applyTemplate(templates.value[0])
  } catch (e) {
    error.value = '加载模板失败: ' + e
  }
}

async function loadConnections() {
  try {
    connections.value = await listConnections()
  } catch (e) {
    error.value = '加载连接失败: ' + e
  }
}

// 应用模板到表单（保留用户已填的 apiKey/name）
function applyTemplate(t) {
  form.templateId = t.id
  if (t.base_url) form.baseUrl = t.base_url
  if (t.default_model) form.model = t.default_model
  form.toolMode = t.tool_mode === 'Native' ? 'native' : 'text_fallback'
  form.reasoning = String(t.default_reasoning || 'disabled').toLowerCase()
  fetchedModels.value = [] // 切模板清空在线拉取的模型
}

function selectTemplate(t) {
  if (isEditing.value) return // 编辑模式不覆盖已有字段
  applyTemplate(t)
  testResult.value = null
}

function resetFormToCreateDefaults() {
  editingId.value = null
  editingHasKey.value = false
  form.templateId = templates.value[0]?.id || 'deepseek'
  form.name = ''
  form.baseUrl = templates.value[0]?.base_url || ''
  form.protocol = 'openai'
  form.model = templates.value[0]?.default_model || ''
  form.apiKey = ''
  form.toolMode = 'native'
  form.temperature = 1.0
  form.topP = 0.95
  form.maxTokens = null
  form.reasoning = 'disabled'
  form.extraJson = ''
  fetchedModels.value = []
  testResult.value = null
  extraParseError.value = ''
  error.value = ''
  if (templates.value[0]) applyTemplate(templates.value[0])
}

function cancelEdit() {
  resetFormToCreateDefaults()
}

async function startEdit(id) {
  loadingEdit.value = true
  error.value = ''
  testResult.value = null
  try {
    const detail = await getConnection(id)
    if (!detail) {
      error.value = '连接不存在或已删除'
      return
    }
    editingId.value = detail.id
    editingHasKey.value = !!detail.has_api_key
    form.name = detail.name || ''
    form.baseUrl = detail.base_url || ''
    form.protocol = detail.protocol || 'openai'
    form.model = detail.model || ''
    form.apiKey = '' // 永不回填明文 key
    form.toolMode = detail.tool_mode || 'native'
    form.temperature = detail.temperature ?? 1.0
    form.topP = detail.top_p ?? 0.95
    form.maxTokens = detail.max_tokens ?? null
    form.reasoning = detail.reasoning || 'disabled'
    form.extraJson = detail.extra ? JSON.stringify(detail.extra, null, 2) : ''
    fetchedModels.value = []
    showAdvanced.value = true
  } catch (e) {
    error.value = '加载连接失败: ' + e
  } finally {
    loadingEdit.value = false
  }
}

// 测试连接（用当前表单的临时配置，不持久化）
async function handleTest() {
  if (!form.baseUrl || !form.model) {
    error.value = '请先填完 base_url / model'
    return
  }
  if (!form.apiKey) {
    if (isEditing.value && editingHasKey.value) {
      error.value = '测试连通性需要填入 API Key（编辑时不会回显已保存密钥）'
    } else {
      error.value = '请先填完 base_url / model / api_key'
    }
    return
  }
  testing.value = true
  testResult.value = null
  error.value = ''
  try {
    testResult.value = await testConnection({
      baseUrl: form.baseUrl,
      apiKey: form.apiKey,
      model: form.model,
      protocol: form.protocol,
      toolMode: form.toolMode,
      reasoning: form.reasoning,
    })
  } catch (e) {
    testResult.value = { success: false, message: String(e) }
  } finally {
    testing.value = false
  }
}

// 保存连接（新建或更新）
async function handleSave() {
  if (!form.name.trim()) {
    error.value = '请填写连接名称'
    return
  }
  if (!form.baseUrl.trim() || !form.model.trim()) {
    error.value = '请填写 base_url / model'
    return
  }
  // 新建必须填 key；编辑可留空表示保留原 key
  if (!isEditing.value && !form.apiKey.trim()) {
    error.value = '请填写 api_key'
    return
  }
  if (isEditing.value && !form.apiKey.trim() && !editingHasKey.value) {
    error.value = '此连接尚无密钥，请填写 api_key'
    return
  }
  // 解析扩展参数 JSON（失败则阻断保存）
  const extra = parseExtraParams()
  if (extraParseError.value) {
    error.value = extraParseError.value
    return
  }
  let maxTokens
  let reasoning
  try {
    maxTokens = normalizeOptionalMaxTokens(form.maxTokens)
    reasoning = normalizeReasoningMode(form.reasoning)
  } catch (e) {
    error.value = String(e.message || e)
    return
  }
  saving.value = true
  error.value = ''
  try {
    const payload = {
      name: form.name,
      baseUrl: form.baseUrl,
      protocol: form.protocol,
      model: form.model,
      apiKey: form.apiKey,
      toolMode: form.toolMode,
      temperature: parseFloat(form.temperature),
      topP: parseFloat(form.topP),
      maxTokens,
      maxTokensExplicit: maxTokens !== null,
      reasoning,
      extra,
    }
    if (isEditing.value) {
      await updateConnection({ id: editingId.value, ...payload })
    } else {
      await createConnection({
        templateId: form.templateId,
        ...payload,
      })
    }
    await loadConnections()
    resetFormToCreateDefaults()
    emit('changed')
  } catch (e) {
    error.value = (isEditing.value ? '更新失败: ' : '保存失败: ') + e
  } finally {
    saving.value = false
  }
}

async function handleDelete(id) {
  const ok = await confirmDialog('确定删除此连接？', { title: '删除确认' })
  if (!ok) return
  try {
    await deleteConnection(id)
    if (editingId.value === id) {
      resetFormToCreateDefaults()
    }
    await loadConnections()
    emit('changed')
  } catch (e) {
    error.value = '删除失败: ' + e
  }
}

async function handleSetActive(id) {
  try {
    await setActiveConnection(id)
    await loadConnections()
    emit('changed')
  } catch (e) {
    error.value = '切换失败: ' + e
  }
}
</script>

<template>
  <PanelHost :show="true" title="LLM 连接配置" side="left" @close="emit('close')">
    <template #header>
      <h2 class="text-sm font-semibold text-ink truncate">LLM 连接配置</h2>
    </template>

    <div class="p-4 space-y-5">
      <!-- 初始加载 -->
      <LoadingState v-if="loading" label="加载中…" />

      <template v-else>
        <!-- 已配置连接列表 -->
        <div v-if="connections.length" class="space-y-2">
          <div class="text-xs text-ink-soft">已配置</div>
          <div
            v-for="c in connections"
            :key="c.id"
            class="flex items-center gap-2 p-2.5 rounded-lg border"
            :class="[
              c.active ? 'border-accent bg-accent-soft/40' : 'border-line bg-surface',
              editingId === c.id ? 'ring-1 ring-accent/50' : '',
            ]"
          >
            <div class="flex-1 min-w-0">
              <div class="text-sm text-ink truncate">{{ c.name }}</div>
              <div class="text-[11px] text-ink-soft truncate">{{ c.model }}</div>
            </div>
            <Button
              v-if="!c.active"
              variant="default"
              size="sm"
              @click="handleSetActive(c.id)"
            >设为活跃</Button>
            <Badge v-else variant="accent" size="sm">● 活跃</Badge>
            <Button
              variant="default"
              size="sm"
              :loading="loadingEdit && editingId === c.id"
              @click="startEdit(c.id)"
            >编辑</Button>
            <Button
              variant="danger"
              size="sm"
              @click="handleDelete(c.id)"
            >删除</Button>
          </div>
        </div>

        <EmptyState
          v-else
          title="还没有配置连接"
          description="新建一个开始写作"
        />

        <!-- 分隔 -->
        <div class="border-t border-line"></div>

        <!-- 新建 / 编辑表单 -->
        <div class="space-y-3">
          <div class="flex items-center justify-between gap-2">
            <div class="text-xs text-ink-soft">{{ formTitle }}</div>
            <Button
              v-if="isEditing"
              variant="ghost"
              size="sm"
              @click="cancelEdit"
            >取消编辑</Button>
          </div>

          <!-- 模板选择（仅新建） -->
          <div v-if="!isEditing">
            <div class="text-[11px] text-ink-soft mb-1.5">从模板选择</div>
            <div class="flex flex-wrap gap-1.5">
              <button
                v-for="t in templates"
                :key="t.id"
                @click="selectTemplate(t)"
                class="px-3 py-1.5 rounded-full text-xs transition-colors"
                :class="form.templateId === t.id
                  ? 'bg-accent text-bg'
                  : 'bg-surface-2 text-ink-soft hover:border-accent-border'"
              >{{ t.name }}</button>
            </div>
          </div>
          <div
            v-else
            class="text-[10px] text-ink-faint rounded-md border border-line bg-surface-2/60 px-2.5 py-2"
          >
            正在编辑「{{ form.name || '未命名' }}」。API Key 不会回显；留空保存表示保留原密钥，填新值则覆盖。
          </div>

          <!-- 名称 -->
          <div class="space-y-1">
            <label class="text-[11px] text-ink-soft">名称</label>
            <Input v-model="form.name" placeholder="如：我的 DeepSeek" />
          </div>

          <!-- base_url -->
          <div class="space-y-1">
            <label class="text-[11px] text-ink-soft">Base URL</label>
            <Input v-model="form.baseUrl" placeholder="https://api.deepseek.com" />
          </div>

          <!-- 协议 -->
          <div class="space-y-1">
            <label class="text-[11px] text-ink-soft">协议</label>
            <Select v-model="form.protocol" :options="protocolOptions" />
          </div>

          <!-- model（datalist：可选可输入 + 拉取按钮）-->
          <div class="space-y-1">
            <label class="text-[11px] text-ink-soft">模型</label>
            <div class="flex gap-1.5">
              <input
                v-model="form.model"
                list="model-options-list"
                placeholder="模型名（可手输或下拉选）"
                class="flex-1 bg-surface-2 border border-line rounded-lg px-3 py-1.5 text-sm text-ink placeholder:text-ink-faint font-mono focus:border-accent outline-none"
              />
              <datalist id="model-options-list">
                <option v-for="m in modelOptions" :key="m.value" :value="m.value" />
              </datalist>
              <Button
                variant="default"
                size="md"
                :loading="fetchingModels"
                :title="'从 ' + form.baseUrl + ' 拉取模型列表'"
                @click="handleFetchModels"
              >
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="7"/><path d="M20 20l-3.5-3.5"/></svg>
              </Button>
            </div>
            <div v-if="fetchedModels.length" class="text-[10px] text-ink-faint mt-1">
              已拉取 {{ fetchedModels.length }} 个模型
            </div>
          </div>

          <!-- 工具模式 -->
          <div class="space-y-1">
            <label class="text-[11px] text-ink-soft">工具调用模式</label>
            <Select v-model="form.toolMode" :options="toolModeOptions" />
          </div>

          <!-- 推理与捕获模式 -->
          <div class="space-y-1">
            <label class="text-[11px] text-ink-soft">推理模式</label>
            <Select v-model="form.reasoning" :options="reasoningOptions" />
            <div class="text-[10px] text-ink-faint">
              Prompted/Native 会要求供应商返回 reasoning_content；缺失时本次生成失败，不再静默丢失思维链。
            </div>
          </div>

          <!-- api_key -->
          <div class="space-y-1">
            <label class="text-[11px] text-ink-soft">
              API Key
              <span v-if="isEditing && editingHasKey" class="text-ink-faint font-normal">（可选）</span>
            </label>
            <div class="relative">
              <input
                v-model="form.apiKey"
                :type="showKey ? 'text' : 'password'"
                :placeholder="apiKeyPlaceholder"
                class="w-full bg-surface-2 border border-line rounded-lg px-3 py-1.5 pr-10 text-sm text-ink placeholder:text-ink-faint font-mono focus:border-accent outline-none"
              />
              <button
                @click="showKey = !showKey"
                class="absolute right-1 top-1/2 -translate-y-1/2 w-8 h-8 flex items-center justify-center rounded-md text-ink-soft hover:text-ink"
                :title="showKey ? '隐藏' : '显示'"
              >{{ showKey ? '🙈' : '👁' }}</button>
            </div>
            <div v-if="!isEditing && selectedTemplate?.get_key_hint" class="text-[10px] text-ink-faint mt-1">
              {{ selectedTemplate.get_key_hint }}
            </div>
          </div>

          <!-- 采样参数（默认展开；抽屉可滚到底 + 底部留白） -->
          <div class="rounded-lg border border-line bg-surface p-3 space-y-3">
            <button
              type="button"
              class="w-full flex items-center justify-between text-xs font-medium text-ink"
              @click="showAdvanced = !showAdvanced"
            >
              <span>采样参数</span>
              <span class="text-ink-faint">{{ showAdvanced ? '收起' : '展开' }}</span>
            </button>
            <div v-show="showAdvanced" class="space-y-3">
              <div class="grid grid-cols-1 sm:grid-cols-3 gap-2">
                <div class="space-y-1">
                  <label class="text-[10px] text-ink-soft">temperature</label>
                  <input v-model.number="form.temperature" type="number" step="0.1"
                    class="w-full bg-surface-2 border border-line rounded-lg px-2 py-1.5 text-xs text-ink focus:border-accent outline-none" />
                </div>
                <div class="space-y-1">
                  <label class="text-[10px] text-ink-soft">top_p</label>
                  <input v-model.number="form.topP" type="number" step="0.05"
                    class="w-full bg-surface-2 border border-line rounded-lg px-2 py-1.5 text-xs text-ink focus:border-accent outline-none" />
                </div>
                <div class="space-y-1">
                  <label class="text-[10px] text-ink-soft">max_tokens（留空=默认）</label>
                  <input v-model.number="form.maxTokens" type="number" min="1" step="256" placeholder="留空"
                    class="w-full bg-surface-2 border border-line rounded-lg px-2 py-1.5 text-xs text-ink focus:border-accent outline-none" />
                </div>
              </div>
              <div class="space-y-1">
                <label class="text-[10px] text-ink-soft">扩展参数 JSON（thinking / reasoning_effort 等）</label>
                <textarea
                  v-model="form.extraJson"
                  rows="2"
                  placeholder='{"thinking":{"type":"enabled"},"reasoning_effort":"max"}'
                  class="w-full bg-surface-2 border border-line rounded-lg px-2 py-1.5 text-xs text-ink font-mono focus:border-accent outline-none"
                ></textarea>
                <div v-if="extraParseError" class="text-[10px] text-err">{{ extraParseError }}</div>
              </div>
            </div>
          </div>

          <!-- 错误 -->
          <div v-if="error" class="p-2 rounded-lg bg-err/10 text-err text-xs">{{ error }}</div>

          <!-- 测试结果 -->
          <div
            v-if="testResult"
            class="p-2 rounded-lg text-xs flex items-center gap-1"
            :class="testResult.success ? 'bg-ok/10 text-ok' : 'bg-err/10 text-err'"
          >
            <span>{{ testResult.success ? '✓' : '✗' }}</span>
            <span class="flex-1">{{ testResult.message }}</span>
            <span v-if="testResult.latencyMs" class="text-ink-soft">· {{ testResult.latencyMs }}ms</span>
          </div>

          <!-- 操作按钮 -->
          <div class="flex gap-2 pt-1">
            <Button
              variant="default"
              size="md"
              class="flex-1"
              :loading="testing"
              @click="handleTest"
            >{{ testing ? '测试中…' : '测试连接' }}</Button>
            <Button
              variant="primary"
              size="md"
              class="flex-1"
              :loading="saving"
              @click="handleSave"
            >{{ saveLabel }}</Button>
          </div>
        </div>
      </template>
    </div>
  </PanelHost>
</template>
