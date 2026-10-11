<script setup>
/**
 * 嵌入模型设置（可选项）。
 *
 * 不配置时远记忆（剧情摘要、消息归档）照样生成，按关键词召回；配置后新记忆
 * 带向量，已有记忆可以一键补建向量（换了嵌入模型后维度不同的旧向量也会重建）。
 */
import { ref, reactive, computed, onMounted } from 'vue'
import { confirmDialog } from '../../components/base/BaseDialog.js'
import {
  getEmbedConfig,
  configureEmbedder,
  clearEmbedConfig,
  getMemoryIndexStatus,
  rebuildMemoryEmbeddings,
} from '../../tauri-api.js'
import Input from '../ui/Input.vue'
import Button from '../ui/Button.vue'
import Badge from '../ui/Badge.vue'
import { errorText } from '../../utils/errorText.js'

const PRESETS = [
  {
    id: 'siliconflow',
    label: 'SiliconFlow bge-large-zh',
    endpoint: 'https://api.siliconflow.cn/v1/embeddings',
    model: 'BAAI/bge-large-zh-v1.5',
  },
  {
    id: 'openai',
    label: 'OpenAI text-embedding-3-small',
    endpoint: 'https://api.openai.com/v1/embeddings',
    model: 'text-embedding-3-small',
  },
]

const saved = ref(null) // get_embed_config 结果（null = 未配置）
const status = ref(null)
const form = reactive({ endpoint: '', model: '', apiKey: '', dim: '' })
const editing = ref(false)
const saving = ref(false)
const rebuilding = ref(false)
const error = ref('')
const notice = ref('')

const configured = computed(() => !!saved.value)
const statusLine = computed(() => {
  const s = status.value
  if (!s) return ''
  if (!s.archived_records) return '还没有远记忆'
  if (!s.embedder_configured) return `${s.archived_records} 条远记忆，按关键词召回`
  if (!s.missing_vectors) return `${s.archived_records} 条远记忆，均已带向量`
  return `${s.archived_records} 条远记忆，其中 ${s.missing_vectors} 条还没有向量（按关键词召回）`
})

async function refresh() {
  try {
    const [config, index] = await Promise.all([getEmbedConfig(), getMemoryIndexStatus()])
    saved.value = config || null
    status.value = index
  } catch (e) {
    error.value = '读取嵌入配置失败: ' + errorText(e)
  }
}

function startEdit() {
  error.value = ''
  notice.value = ''
  form.endpoint = saved.value?.endpoint || ''
  form.model = saved.value?.model || ''
  form.dim = saved.value?.dim ? String(saved.value.dim) : ''
  form.apiKey = ''
  editing.value = true
}

function applyPreset(preset) {
  form.endpoint = preset.endpoint
  form.model = preset.model
  form.dim = ''
}

async function handleSave() {
  error.value = ''
  notice.value = ''
  if (!form.endpoint.trim() || !form.model.trim()) {
    error.value = '请填写接口地址和模型名'
    return
  }
  saving.value = true
  try {
    await configureEmbedder({
      endpoint: form.endpoint.trim(),
      apiKey: form.apiKey.trim(),
      model: form.model.trim(),
      dim: form.dim,
      keepExistingKey: !!saved.value?.has_key,
    })
    editing.value = false
    await refresh()
    notice.value = status.value?.missing_vectors
      ? `已保存。可以为 ${status.value.missing_vectors} 条远记忆补建向量。`
      : '已保存，接口测试通过。'
  } catch (e) {
    error.value = errorText(e)
  } finally {
    saving.value = false
  }
}

async function handleClear() {
  const ok = await confirmDialog('停用后远记忆只按关键词召回，已有向量会保留。确定停用吗？', {
    title: '停用嵌入模型',
  })
  if (!ok) return
  error.value = ''
  notice.value = ''
  try {
    await clearEmbedConfig()
    editing.value = false
    await refresh()
    notice.value = '已停用嵌入模型，远记忆按关键词召回。'
  } catch (e) {
    error.value = '停用失败: ' + errorText(e)
  }
}

async function handleRebuild() {
  error.value = ''
  notice.value = ''
  rebuilding.value = true
  try {
    const report = await rebuildMemoryEmbeddings()
    if (!report.candidates) {
      notice.value = '所有远记忆都已带向量。'
    } else if (report.error) {
      error.value = `补建了 ${report.embedded}/${report.candidates} 条，其余因接口出错未完成：${report.error}`
    } else {
      notice.value = `已为 ${report.embedded} 条远记忆补建向量。`
    }
  } catch (e) {
    error.value = '补建向量失败: ' + errorText(e)
  } finally {
    rebuilding.value = false
    await refresh()
  }
}

onMounted(refresh)
</script>

<template>
  <div class="space-y-3">
    <div class="flex items-center justify-between gap-2">
      <div class="text-xs text-ink-soft">嵌入模型（可选）</div>
      <Badge v-if="configured" variant="accent" size="sm">已启用</Badge>
      <Badge v-else size="sm">未启用</Badge>
    </div>
    <p class="text-[11px] text-ink-soft leading-relaxed">
      远记忆不依赖嵌入模型：未配置时按关键词召回。配置后召回按语义匹配，更容易找回措辞不同的旧剧情。
    </p>

    <div v-if="statusLine" class="text-[11px] text-ink-soft">{{ statusLine }}</div>

    <!-- 当前配置 -->
    <div
      v-if="configured && !editing"
      class="p-2.5 rounded-lg border border-line bg-surface space-y-1"
    >
      <div class="text-sm text-ink truncate">{{ saved.model }}</div>
      <div class="text-[11px] text-ink-soft truncate">{{ saved.endpoint }}</div>
      <div class="text-[11px] text-ink-soft">维度 {{ saved.dim }} · {{ saved.has_key ? '已保存密钥' : '无密钥' }}</div>
    </div>

    <!-- 编辑表单 -->
    <div v-if="editing" class="space-y-2">
      <div class="flex flex-wrap gap-1.5">
        <button
          v-for="preset in PRESETS"
          :key="preset.id"
          type="button"
          class="px-2 py-1 rounded-md border border-line text-[11px] text-ink-soft hover:border-accent hover:text-ink"
          @click="applyPreset(preset)"
        >{{ preset.label }}</button>
      </div>
      <div class="space-y-1">
        <label class="text-[10px] text-ink-soft">接口地址（OpenAI 兼容 /v1/embeddings）</label>
        <Input v-model="form.endpoint" placeholder="https://api.siliconflow.cn/v1/embeddings" />
      </div>
      <div class="space-y-1">
        <label class="text-[10px] text-ink-soft">模型名</label>
        <Input v-model="form.model" placeholder="BAAI/bge-large-zh-v1.5" />
      </div>
      <div class="space-y-1">
        <label class="text-[10px] text-ink-soft">API Key</label>
        <Input
          v-model="form.apiKey"
          type="password"
          :placeholder="saved?.has_key ? '已保存密钥（留空不改）' : '本地服务可留空'"
        />
      </div>
      <div class="space-y-1">
        <label class="text-[10px] text-ink-soft">向量维度（留空自动识别）</label>
        <Input v-model="form.dim" placeholder="自动" />
      </div>
    </div>

    <div v-if="error" class="p-2 rounded-lg bg-err/10 text-err text-xs">{{ error }}</div>
    <div v-if="notice" class="p-2 rounded-lg bg-ok/10 text-ok text-xs">{{ notice }}</div>

    <div class="flex flex-wrap gap-2">
      <template v-if="editing">
        <Button variant="ghost" size="sm" @click="editing = false">取消</Button>
        <Button variant="primary" size="sm" :loading="saving" @click="handleSave">
          {{ saving ? '测试并保存中…' : '测试并保存' }}
        </Button>
      </template>
      <template v-else>
        <Button variant="default" size="sm" @click="startEdit">
          {{ configured ? '修改' : '配置嵌入模型' }}
        </Button>
        <Button
          v-if="configured && status?.missing_vectors"
          variant="default"
          size="sm"
          :loading="rebuilding || status?.backfill_running"
          @click="handleRebuild"
        >补建向量（{{ status.missing_vectors }} 条）</Button>
        <Button v-if="configured" variant="danger" size="sm" @click="handleClear">停用</Button>
      </template>
    </div>
  </div>
</template>
