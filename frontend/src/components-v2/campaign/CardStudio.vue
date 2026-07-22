<script setup>
import { computed, ref, watch } from 'vue'
import { alertDialog } from '../../components/base/BaseDialog.js'
import {
  cardstudioCompleteManualStage,
  cardstudioCreateProject,
  cardstudioGetProject,
  cardstudioImportCompiled,
  cardstudioListProjects,
  cardstudioRunChecks,
  cardstudioRunStage,
  cardstudioSetOptions,
  cardstudioUpdateArtifacts,
} from '../../tauri-api.js'
import Button from '../ui/Button.vue'
import EmptyState from '../ui/EmptyState.vue'
import Input from '../ui/Input.vue'

const emit = defineEmits(['imported', 'close'])

const STAGE_META = [
  { id: 'brief', label: '意图' },
  { id: 'basic', label: '角色基础' },
  { id: 'personality', label: '性格' },
  { id: 'worldview', label: '世界书' },
  { id: 'opening', label: '开场白' },
  { id: 'review', label: '检查' },
  { id: 'compile_import', label: '导入' },
]

const projects = ref([])
const project = ref(null)
const loading = ref(false)
const busy = ref(false)
const statusText = ref('')
const newName = ref('')
const newBrief = ref('')
const userNote = ref('')
const allowAiFreewrite = ref(false)
const checkReport = ref(null)
const lastImport = ref(null)

const draft = ref(emptyArtifacts())

function emptyArtifacts() {
  return {
    name: '',
    description: '',
    personality: '',
    scenario: '',
    first_mes: '',
    tags: [],
    creator: '',
    worldview_entries: [],
    notes: '',
    personality_mode: null,
    personality_prompts: [],
    world_type: null,
    opening_outline: null,
  }
}

const stages = computed(() => STAGE_META.map((s) => {
  const st = project.value?.stage_status?.[s.id] || 'pending'
  return { ...s, status: st }
}))

const currentStage = computed(() => project.value?.current_stage || 'brief')
const isLlmStage = computed(() => ['basic', 'personality', 'worldview', 'opening'].includes(currentStage.value))

const worldviewText = computed({
  get() {
    return (draft.value.worldview_entries || [])
      .map((e) => {
        const keys = (e.keys || []).join(',')
        const flag = e.constant ? '蓝灯' : '绿灯'
        return `[${flag}|${keys}] ${e.content || ''}`
      })
      .join('\n')
  },
  set(v) {
    draft.value.worldview_entries = String(v || '')
      .split('\n')
      .map((line) => line.trim())
      .filter(Boolean)
      .map((line, idx) => {
        const m = line.match(/^\[(蓝灯|绿灯)\|([^\]]*)\]\s*(.*)$/)
        if (m) {
          return {
            keys: m[2].split(',').map((s) => s.trim()).filter(Boolean),
            content: m[3] || '',
            constant: m[1] === '蓝灯',
            order: (idx + 1) * 10,
          }
        }
        return {
          keys: [],
          content: line,
          constant: true,
          order: (idx + 1) * 10,
        }
      })
  },
})

const tagsText = computed({
  get() {
    return (draft.value.tags || []).join(', ')
  },
  set(v) {
    draft.value.tags = String(v || '')
      .split(/[,，]/)
      .map((s) => s.trim())
      .filter(Boolean)
  },
})

async function refreshProjects() {
  loading.value = true
  try {
    projects.value = await cardstudioListProjects()
  } finally {
    loading.value = false
  }
}

function syncDraftFromProject() {
  if (!project.value) {
    draft.value = emptyArtifacts()
    return
  }
  draft.value = {
    ...emptyArtifacts(),
    ...project.value.artifacts,
    tags: [...(project.value.artifacts?.tags || [])],
    personality_prompts: [...(project.value.artifacts?.personality_prompts || [])],
    worldview_entries: (project.value.artifacts?.worldview_entries || []).map((e) => ({ ...e, keys: [...(e.keys || [])] })),
  }
}

async function openProject(id) {
  busy.value = true
  statusText.value = ''
  checkReport.value = null
  lastImport.value = null
  try {
    project.value = await cardstudioGetProject(id)
    allowAiFreewrite.value = !!project.value?.allow_ai_freewrite
    syncDraftFromProject()
  } catch (e) {
    await alertDialog('打开项目失败: ' + e)
  } finally {
    busy.value = false
  }
}

async function createProject() {
  if (!newName.value.trim()) {
    await alertDialog('请填写项目名')
    return
  }
  busy.value = true
  try {
    const created = await cardstudioCreateProject(newName.value.trim(), newBrief.value.trim())
    newName.value = ''
    newBrief.value = ''
    await refreshProjects()
    project.value = created
    allowAiFreewrite.value = !!created?.allow_ai_freewrite
    syncDraftFromProject()
    statusText.value = '已创建写卡项目（明月秋青方法论 pack）'
  } catch (e) {
    await alertDialog('创建失败: ' + e)
  } finally {
    busy.value = false
  }
}

async function saveArtifacts() {
  if (!project.value) return
  busy.value = true
  try {
    project.value = await cardstudioUpdateArtifacts(project.value.id, draft.value)
    project.value = await cardstudioSetOptions(project.value.id, {
      allowAiFreewrite: allowAiFreewrite.value,
    })
    allowAiFreewrite.value = !!project.value?.allow_ai_freewrite
    syncDraftFromProject()
    statusText.value = '产物已保存'
  } catch (e) {
    await alertDialog('保存失败: ' + e)
  } finally {
    busy.value = false
  }
}

async function completeManual() {
  if (!project.value) return
  await saveArtifacts()
  busy.value = true
  try {
    project.value = await cardstudioCompleteManualStage(project.value.id, currentStage.value)
    syncDraftFromProject()
    statusText.value = `阶段 ${currentStage.value} 已完成`
  } catch (e) {
    await alertDialog(String(e))
  } finally {
    busy.value = false
  }
}

async function runStage() {
  if (!project.value) return
  await saveArtifacts()
  busy.value = true
  statusText.value = `正在生成：${currentStage.value}…`
  try {
    project.value = await cardstudioRunStage(project.value.id, currentStage.value, userNote.value || null)
    syncDraftFromProject()
    userNote.value = ''
    statusText.value = `阶段 ${currentStage.value} 生成完成（若已推进，请看阶段轨）`
  } catch (e) {
    await alertDialog(String(e))
    try {
      project.value = await cardstudioGetProject(project.value.id)
      syncDraftFromProject()
    } catch {
      /* ignore */
    }
  } finally {
    busy.value = false
  }
}

async function runChecks() {
  if (!project.value) return
  await saveArtifacts()
  busy.value = true
  try {
    checkReport.value = await cardstudioRunChecks(project.value.id)
    statusText.value = checkReport.value.ok ? '检查通过' : '检查未通过'
  } catch (e) {
    await alertDialog(String(e))
  } finally {
    busy.value = false
  }
}

async function importCompiled() {
  if (!project.value) return
  await saveArtifacts()
  busy.value = true
  try {
    const result = await cardstudioImportCompiled(project.value.id)
    lastImport.value = result
    project.value = await cardstudioGetProject(project.value.id)
    syncDraftFromProject()
    statusText.value = `已导入角色卡：${result.character?.name || ''}（card ${result.card_id}）`
    emit('imported', result)
  } catch (e) {
    await alertDialog(String(e))
  } finally {
    busy.value = false
  }
}

function stageBadgeClass(status) {
  if (status === 'done') return 'text-ok'
  if (status === 'ready') return 'text-accent'
  if (status === 'failed') return 'text-err'
  return 'text-ink-soft'
}

refreshProjects().catch(() => {})

watch(
  () => project.value?.id,
  () => {
    checkReport.value = null
  },
)
</script>

<template>
  <div class="space-y-4 max-w-4xl">
    <div class="flex items-center justify-between gap-2">
      <div>
        <div class="text-sm font-medium text-ink">写卡工作室 · 从零创作</div>
        <div class="text-xs text-ink-soft">阶段生成 → 编辑产物 → 检查 → 导入为可玩角色卡</div>
      </div>
      <Button variant="ghost" size="sm" @click="emit('close')">返回卡库</Button>
    </div>

    <div class="rounded-xl border border-line bg-surface p-3 space-y-2 shadow-card">
      <div class="text-xs font-medium text-ink">新建项目</div>
      <Input v-model="newName" placeholder="项目名 / 暂定角色名" />
      <textarea
        v-model="newBrief"
        rows="3"
        class="w-full rounded-md border border-line bg-surface-2 px-3 py-2 text-sm text-ink"
        placeholder="创作意图 brief：世界观类型、主角关系、想要的氛围…"
      />
      <Button variant="primary" size="md" :loading="busy" :disabled="busy" @click="createProject">创建</Button>
    </div>

    <div class="rounded-xl border border-line bg-surface p-3 shadow-card">
      <div class="text-xs font-medium text-ink mb-2">已有项目</div>
      <EmptyState v-if="!loading && projects.length === 0" title="还没有写卡项目" description="先创建一个从零项目" />
      <div v-else class="space-y-1">
        <button
          v-for="p in projects"
          :key="p.id"
          type="button"
          class="w-full text-left rounded-lg px-3 py-2 text-sm hover:bg-surface-2 border border-transparent"
          :class="project?.id === p.id ? 'border-accent bg-surface-2' : ''"
          @click="openProject(p.id)"
        >
          <div class="font-medium text-ink truncate">{{ p.name }}</div>
          <div class="text-xs text-ink-soft truncate">{{ p.current_stage }} · {{ p.updated_at }}</div>
        </button>
      </div>
    </div>

    <template v-if="project">
      <div class="rounded-xl border border-line bg-surface p-3 shadow-card space-y-3">
        <div class="flex flex-wrap gap-2">
          <span
            v-for="s in stages"
            :key="s.id"
            class="text-xs px-2 py-1 rounded-md border border-line"
            :class="[stageBadgeClass(s.status), currentStage === s.id ? 'ring-1 ring-accent' : '']"
          >
            {{ s.label }} · {{ s.status }}
          </span>
        </div>

        <div class="text-xs text-ink-soft" v-if="statusText">{{ statusText }}</div>
        <div class="text-xs text-err" v-if="project.last_error">{{ project.last_error }}</div>
        <div class="text-[11px] text-ink-faint">
          提示词包：{{ project.stage_pack_id || 'mingyue_qiuqing_v1' }} · 性格默认协作（手写衍生优先）
        </div>

        <label class="flex items-center gap-2 text-xs text-ink-soft">
          <input v-model="allowAiFreewrite" type="checkbox" class="rounded border-line" />
          允许 AI 代写性格衍生（默认关闭，对齐明月“手写优先”）
        </label>

        <div class="grid gap-2">
          <label class="text-xs text-ink-soft">角色名</label>
          <Input v-model="draft.name" />
          <label class="text-xs text-ink-soft">描述 description（外貌/背景/关系，不含性格）</label>
          <textarea v-model="draft.description" rows="4" class="w-full rounded-md border border-line bg-surface-2 px-3 py-2 text-sm text-ink" />
          <label class="text-xs text-ink-soft">性格调色盘 personality（可含【待用户手写】）</label>
          <textarea v-model="draft.personality" rows="4" class="w-full rounded-md border border-line bg-surface-2 px-3 py-2 text-sm text-ink" />
          <div v-if="draft.personality_prompts?.length" class="text-xs text-warn bg-warn/10 rounded-lg px-3 py-2 space-y-1">
            <div class="font-medium">需要你补充的性格问题</div>
            <div v-for="(q, i) in draft.personality_prompts" :key="i">· {{ q }}</div>
          </div>
          <label class="text-xs text-ink-soft">场景 scenario</label>
          <textarea v-model="draft.scenario" rows="2" class="w-full rounded-md border border-line bg-surface-2 px-3 py-2 text-sm text-ink" />
          <label class="text-xs text-ink-soft">开场白 first_mes</label>
          <textarea v-model="draft.first_mes" rows="4" class="w-full rounded-md border border-line bg-surface-2 px-3 py-2 text-sm text-ink" />
          <label class="text-xs text-ink-soft">标签（逗号分隔）</label>
          <Input v-model="tagsText" />
          <label class="text-xs text-ink-soft">世界书（每行一条：`[蓝灯|]` 或 `[绿灯|关键词1,关键词2] 内容`）</label>
          <textarea v-model="worldviewText" rows="5" class="w-full rounded-md border border-line bg-surface-2 px-3 py-2 text-sm text-ink font-mono" />
        </div>

        <div class="flex flex-wrap gap-2">
          <Button variant="default" size="sm" :disabled="busy" @click="saveArtifacts">保存产物</Button>
          <Button
            v-if="currentStage === 'brief' || currentStage === 'review'"
            variant="primary"
            size="sm"
            :loading="busy"
            :disabled="busy"
            @click="completeManual"
          >
            {{ currentStage === 'brief' ? '确认意图，进入基础' : '检查通过，进入导入' }}
          </Button>
          <template v-if="isLlmStage">
            <Input v-model="userNote" placeholder="本阶段补充说明（可选）" class="min-w-[12rem] flex-1" />
            <Button variant="primary" size="sm" :loading="busy" :disabled="busy" @click="runStage">AI 生成本阶段</Button>
          </template>
          <Button variant="default" size="sm" :disabled="busy" @click="runChecks">结构检查</Button>
          <Button
            v-if="currentStage === 'compile_import' || project.stage_status?.review === 'done'"
            variant="primary"
            size="sm"
            :loading="busy"
            :disabled="busy"
            @click="importCompiled"
          >
            编译并导入卡库
          </Button>
        </div>

        <div v-if="checkReport" class="text-xs space-y-1">
          <div :class="checkReport.ok ? 'text-ok' : 'text-err'">
            检查结果：{{ checkReport.ok ? '通过' : '未通过' }}
          </div>
          <div v-for="(issue, i) in checkReport.issues" :key="i" class="text-ink-soft">
            [{{ issue.severity }}] {{ issue.message }}
          </div>
        </div>

        <div v-if="lastImport" class="text-xs text-ok">
          导入成功：{{ lastImport.character?.name }} / card {{ lastImport.card_id }}
          <span v-if="lastImport.warnings?.length">；警告 {{ lastImport.warnings.length }} 条</span>
        </div>

        <details v-if="project.last_stage_output" class="text-xs">
          <summary class="cursor-pointer text-ink-soft">最近一次模型原文</summary>
          <pre class="mt-2 whitespace-pre-wrap break-words text-ink-soft bg-surface-2 rounded-lg p-2">{{ project.last_stage_output }}</pre>
        </details>
      </div>
    </template>
  </div>
</template>
