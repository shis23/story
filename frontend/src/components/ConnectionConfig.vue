<script setup>
import { ref, reactive, onMounted, computed } from 'vue'
import {
  listConnectionTemplates,
  listConnections,
  createConnection,
  deleteConnection,
  setActiveConnection,
  testConnection,
  listModels,
} from '../tauri-api.js'

const emit = defineEmits(['close', 'changed'])

// 内置模板
const templates = ref([])
// 已配置连接
const connections = ref([])

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
  maxTokens: 4096,
})

const showAdvanced = ref(false)
const showKey = ref(false)
const testing = ref(false)
const testResult = ref(null)
const saving = ref(false)
const error = ref('')

// 在线拉取的模型列表（与模板预置合并去重）
const fetchedModels = ref([])
const fetchingModels = ref(false)

// 当前选中的模板对象（用于显示可选模型）
const selectedTemplate = computed(
  () => templates.value.find((t) => t.id === form.templateId) || null
)

// 合并的模型选项：模板预置 + 在线拉取（去重）
const modelOptions = computed(() => {
  const tpl = selectedTemplate.value?.models || []
  return [...new Set([...tpl, ...fetchedModels.value])]
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
      // 自动选中第一个（如果当前为空或不在选项里）
      if (!form.model || !modelOptions.value.includes(form.model)) {
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
  fetchedModels.value = [] // 切模板清空在线拉取的模型
}

function selectTemplate(t) {
  applyTemplate(t)
  testResult.value = null
}

// 测试连接（用当前表单的临时配置，不持久化）
async function handleTest() {
  if (!form.baseUrl || !form.apiKey || !form.model) {
    error.value = '请先填完 base_url / model / api_key'
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
      toolMode: form.toolMode,
    })
  } catch (e) {
    testResult.value = { success: false, message: String(e) }
  } finally {
    testing.value = false
  }
}

// 保存连接
async function handleSave() {
  if (!form.name.trim()) {
    error.value = '请填写连接名称'
    return
  }
  if (!form.baseUrl.trim() || !form.apiKey.trim() || !form.model.trim()) {
    error.value = '请填写 base_url / model / api_key'
    return
  }
  saving.value = true
  error.value = ''
  try {
    await createConnection({
      templateId: form.templateId,
      name: form.name,
      baseUrl: form.baseUrl,
      protocol: form.protocol,
      model: form.model,
      apiKey: form.apiKey,
      toolMode: form.toolMode,
      temperature: parseFloat(form.temperature),
      topP: parseFloat(form.topP),
      maxTokens: parseInt(form.maxTokens),
    })
    await loadConnections()
    form.name = ''
    form.apiKey = ''
    emit('changed')
  } catch (e) {
    error.value = '保存失败: ' + e
  } finally {
    saving.value = false
  }
}

async function handleDelete(id) {
  if (!confirm('确定删除此连接？')) return
  try {
    await deleteConnection(id)
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
  <!-- 遮罩 -->
  <div class="fixed inset-0 z-50 bg-black/40 flex items-end sm:items-center justify-center" @click.self="emit('close')">
    <div class="bg-bg w-full max-w-md max-h-[90vh] overflow-y-auto rounded-t-2xl sm:rounded-2xl border border-line">
      <!-- 头部 -->
      <div class="sticky top-0 bg-bg border-b border-line px-4 py-3 flex items-center justify-between">
        <span class="text-sm font-medium text-ink">LLM 连接配置</span>
        <button @click="emit('close')" class="text-ink-soft hover:text-ink">✕</button>
      </div>

      <div class="p-4 space-y-5">
        <!-- 已配置连接列表 -->
        <div v-if="connections.length" class="space-y-2">
          <div class="text-xs text-ink-soft">已配置</div>
          <div
            v-for="c in connections"
            :key="c.id"
            class="flex items-center gap-2 p-2.5 rounded-lg border"
            :class="c.active ? 'border-accent bg-accent-soft/40' : 'border-line bg-surface'"
          >
            <div class="flex-1 min-w-0">
              <div class="text-sm text-ink truncate">{{ c.name }}</div>
              <div class="text-[11px] text-ink-soft truncate">{{ c.model }}</div>
            </div>
            <button
              v-if="!c.active"
              @click="handleSetActive(c.id)"
              class="px-2 py-1 text-[11px] rounded-md bg-bg text-ink-soft hover:bg-line"
            >设为活跃</button>
            <span v-else class="text-[11px] text-accent">● 活跃</span>
            <button
              @click="handleDelete(c.id)"
              class="px-2 py-1 text-[11px] rounded-md text-err/70 hover:bg-err/10"
            >删除</button>
          </div>
        </div>

        <div v-else class="p-3 rounded-lg border border-dashed border-line text-center text-xs text-ink-soft">
          还没有配置连接，新建一个开始写作
        </div>

        <!-- 分隔 -->
        <div class="border-t border-line"></div>

        <!-- 新建连接表单 -->
        <div class="space-y-3">
          <div class="text-xs text-ink-soft">新建连接</div>

          <!-- 模板选择 -->
          <div>
            <div class="text-[11px] text-ink-soft mb-1.5">从模板选择</div>
            <div class="flex flex-wrap gap-1.5">
              <button
                v-for="t in templates"
                :key="t.id"
                @click="selectTemplate(t)"
                class="px-2.5 py-1 rounded-full text-xs transition-all"
                :class="form.templateId === t.id
                  ? 'bg-accent text-white'
                  : 'bg-bg text-ink-soft hover:bg-line'"
              >{{ t.name }}</button>
            </div>
          </div>

          <!-- 名称 -->
          <div>
            <label class="text-[11px] text-ink-soft">名称</label>
            <input
              v-model="form.name"
              placeholder="如：我的 DeepSeek"
              class="w-full mt-1 px-3 py-2 text-sm rounded-lg border border-line bg-surface focus:outline-none focus:border-accent"
            />
          </div>

          <!-- base_url -->
          <div>
            <label class="text-[11px] text-ink-soft">Base URL</label>
            <input
              v-model="form.baseUrl"
              placeholder="https://api.deepseek.com"
              class="w-full mt-1 px-3 py-2 text-sm rounded-lg border border-line bg-surface focus:outline-none focus:border-accent font-mono"
            />
          </div>

          <!-- model（datalist：可选可输入 + 拉取按钮）-->
          <div>
            <label class="text-[11px] text-ink-soft">模型</label>
            <div class="flex gap-1.5 mt-1">
              <input
                v-model="form.model"
                list="model-options-list"
                placeholder="模型名（可手输或下拉选）"
                class="flex-1 px-3 py-2 text-sm rounded-lg border border-line bg-surface focus:outline-none focus:border-accent font-mono"
              />
              <datalist id="model-options-list">
                <option v-for="m in modelOptions" :key="m" :value="m" />
              </datalist>
              <button
                @click="handleFetchModels"
                :disabled="fetchingModels"
                class="shrink-0 px-2.5 py-2 text-xs rounded-lg border border-line text-ink-soft hover:bg-line disabled:opacity-50"
                :title="'从 ' + form.baseUrl + ' 拉取模型列表'"
              >{{ fetchingModels ? '⏳' : '🔍' }}</button>
            </div>
            <div v-if="fetchedModels.length" class="text-[10px] text-ink-soft/70 mt-1">
              已拉取 {{ fetchedModels.length }} 个模型
            </div>
          </div>

          <!-- api_key -->
          <div>
            <label class="text-[11px] text-ink-soft">API Key</label>
            <div class="relative mt-1">
              <input
                v-model="form.apiKey"
                :type="showKey ? 'text' : 'password'"
                placeholder="sk-..."
                class="w-full px-3 py-2 pr-10 text-sm rounded-lg border border-line bg-surface focus:outline-none focus:border-accent font-mono"
              />
              <button
                @click="showKey = !showKey"
                class="absolute right-2 top-1/2 -translate-y-1/2 text-ink-soft text-xs"
              >{{ showKey ? '🙈' : '👁' }}</button>
            </div>
            <div v-if="selectedTemplate?.get_key_hint" class="text-[10px] text-ink-soft/70 mt-1">
              {{ selectedTemplate.get_key_hint }}
            </div>
          </div>

          <!-- 高级（采样参数） -->
          <div>
            <button
              @click="showAdvanced = !showAdvanced"
              class="text-[11px] text-accent"
            >{{ showAdvanced ? '▾ 收起采样参数' : '▸ 展开采样参数' }}</button>
            <div v-if="showAdvanced" class="mt-2 grid grid-cols-3 gap-2">
              <div>
                <label class="text-[10px] text-ink-soft">temperature</label>
                <input v-model.number="form.temperature" type="number" step="0.1"
                  class="w-full px-2 py-1 text-xs rounded border border-line bg-surface" />
              </div>
              <div>
                <label class="text-[10px] text-ink-soft">top_p</label>
                <input v-model.number="form.topP" type="number" step="0.05"
                  class="w-full px-2 py-1 text-xs rounded border border-line bg-surface" />
              </div>
              <div>
                <label class="text-[10px] text-ink-soft">max_tokens</label>
                <input v-model.number="form.maxTokens" type="number" step="256"
                  class="w-full px-2 py-1 text-xs rounded border border-line bg-surface" />
              </div>
            </div>
          </div>

          <!-- 错误 -->
          <div v-if="error" class="p-2 rounded-lg bg-err/10 text-err text-xs">{{ error }}</div>

          <!-- 测试结果 -->
          <div
            v-if="testResult"
            class="p-2 rounded-lg text-xs"
            :class="testResult.success ? 'bg-green-500/10 text-green-700' : 'bg-err/10 text-err'"
          >
            {{ testResult.success ? '✓' : '✗' }} {{ testResult.message }}
            <span v-if="testResult.latencyMs" class="text-ink-soft">· {{ testResult.latencyMs }}ms</span>
          </div>

          <!-- 操作按钮 -->
          <div class="flex gap-2 pt-1">
            <button
              @click="handleTest"
              :disabled="testing"
              class="flex-1 py-2 text-xs rounded-lg border border-line text-ink hover:bg-bg disabled:opacity-50"
            >{{ testing ? '测试中…' : '🔌 测试连接' }}</button>
            <button
              @click="handleSave"
              :disabled="saving"
              class="flex-1 py-2 text-xs rounded-lg bg-accent text-white hover:opacity-90 disabled:opacity-50"
            >{{ saving ? '保存中…' : '💾 保存' }}</button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
