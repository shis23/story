<script setup>
import { computed, ref, watch } from 'vue'
import { alertDialog, confirmDialog } from '../../components/base/BaseDialog.js'
import {
  cardstudioCompile,
  cardstudioCompleteManualStage,
  cardstudioCreateFromCharacter,
  cardstudioCreateFromNovel,
  cardstudioCreateProject,
  cardstudioDeleteProject,
  cardstudioExportGate,
  cardstudioExportPng,
  cardstudioGetProject,
  cardstudioImportCompiled,
  cardstudioListProjects,
  cardstudioPrefillFromNovel,
  cardstudioRunChecks,
  cardstudioRunReview,
  cardstudioRunStage,
  cardstudioSetOptions,
  cardstudioSetStage,
  cardstudioUpdateArtifacts,
  extractCharacters,
} from '../../tauri-api.js'
import Button from '../ui/Button.vue'
import EmptyState from '../ui/EmptyState.vue'
import Input from '../ui/Input.vue'
import { errorText } from '../../utils/errorText.js'

const props = defineProps({
  /** optional { characterId, brief } to auto-open revise project */
  seed: { type: Object, default: null },
})
const emit = defineEmits(['imported', 'close', 'go-library'])

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
const novelTitle = ref('')
const novelText = ref('')
const userNote = ref('')
const allowAiFreewrite = ref(false)
const checkReport = ref(null)
const lastImport = ref(null)
const lastCompile = ref(null)
const gateReport = ref(null)
const seedHandledKey = ref('')

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
    style_notes: null,
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
  lastCompile.value = null
  gateReport.value = null
  try {
    project.value = await cardstudioGetProject(id)
    allowAiFreewrite.value = !!project.value?.allow_ai_freewrite
    syncDraftFromProject()
  } catch (e) {
    await alertDialog('打开项目失败: ' + errorText(e))
  } finally {
    busy.value = false
  }
}

async function deleteProjectById(id, name = '') {
  if (!id) return
  const ok = await confirmDialog(`确定删除写卡项目「${name || id}」？此操作不可恢复。`, {
    title: '删除写卡项目',
  })
  if (!ok) return
  busy.value = true
  try {
    await cardstudioDeleteProject(id)
    if (project.value?.id === id) {
      project.value = null
      draft.value = emptyArtifacts()
      checkReport.value = null
      lastImport.value = null
      lastCompile.value = null
      gateReport.value = null
    }
    await refreshProjects()
    statusText.value = '项目已删除'
  } catch (e) {
    await alertDialog('删除失败: ' + errorText(e))
  } finally {
    busy.value = false
  }
}

async function deleteCurrentProject() {
  if (!project.value) return
  await deleteProjectById(project.value.id, project.value.name)
}

function safeFileBase(name) {
  const base = String(name || 'cardstudio')
    .replace(/[\\/:*?"<>|]+/g, '_')
    .replace(/\s+/g, '_')
    .slice(0, 48)
  return base || 'cardstudio'
}

async function exportStJson() {
  if (!project.value) return
  await saveArtifacts()
  busy.value = true
  try {
    const compiled = await cardstudioCompile(project.value.id)
    lastCompile.value = compiled
    const json = JSON.stringify(compiled.st_card_json ?? {}, null, 2)
    const fileName = `${safeFileBase(compiled.character_name || project.value.name || draft.value.name)}.json`
    let saved = false
    try {
      const { save } = await import('@tauri-apps/plugin-dialog')
      const { writeTextFile } = await import('@tauri-apps/plugin-fs')
      const filePath = await save({
        defaultPath: fileName,
        filters: [{ name: 'ST Character JSON', extensions: ['json'] }],
      })
      if (filePath) {
        await writeTextFile(filePath, json)
        saved = true
        statusText.value = `已导出 ST JSON：${filePath}`
      }
    } catch {
      // fall through to browser download / clipboard
    }
    if (!saved) {
      try {
        const blob = new Blob([json], { type: 'application/json;charset=utf-8' })
        const url = URL.createObjectURL(blob)
        const a = document.createElement('a')
        a.href = url
        a.download = fileName
        a.click()
        URL.revokeObjectURL(url)
        saved = true
        statusText.value = `已下载 ST JSON：${fileName}`
      } catch {
        await navigator.clipboard.writeText(json)
        statusText.value = '已复制 ST JSON 到剪贴板'
        saved = true
      }
    }
    if (compiled.warnings?.length) {
      statusText.value += `（警告 ${compiled.warnings.length} 条）`
    }
  } catch (e) {
    await alertDialog('导出失败: ' + errorText(e))
  } finally {
    busy.value = false
  }
}

async function runExportGate() {
  if (!project.value) return
  await saveArtifacts()
  busy.value = true
  try {
    gateReport.value = await cardstudioExportGate(project.value.id)
    statusText.value = gateReport.value.pass
      ? '出卡质量闸门：全部通过'
      : '出卡质量闸门：存在未过项，见下方报告'
  } catch (e) {
    gateReport.value = null
    await alertDialog('出卡质量闸门运行失败: ' + errorText(e))
  } finally {
    busy.value = false
  }
}

async function exportStPng() {
  if (!project.value) return
  await saveArtifacts()
  busy.value = true
  try {
    const bytes = await cardstudioExportPng(project.value.id)
    const data = bytes instanceof Uint8Array ? bytes : Uint8Array.from(bytes || [])
    if (!data.length) throw new Error('PNG 内容为空')
    const fileName = `${safeFileBase(draft.value.name || project.value.name)}.png`
    let saved = false
    try {
      const { save } = await import('@tauri-apps/plugin-dialog')
      const { writeFile } = await import('@tauri-apps/plugin-fs')
      const filePath = await save({
        defaultPath: fileName,
        filters: [{ name: 'ST Character PNG', extensions: ['png'] }],
      })
      if (filePath) {
        await writeFile(filePath, data)
        saved = true
        statusText.value = `已导出 ST PNG：${filePath}`
      }
    } catch {
      // fall through to browser download
    }
    if (!saved) {
      const blob = new Blob([data], { type: 'image/png' })
      const url = URL.createObjectURL(blob)
      const a = document.createElement('a')
      a.href = url
      a.download = fileName
      a.click()
      URL.revokeObjectURL(url)
      statusText.value = `已下载 ST PNG：${fileName}`
    }
  } catch (e) {
    await alertDialog('PNG 导出失败: ' + errorText(e))
  } finally {
    busy.value = false
  }
}

function isReviseMode(p = project.value) {
  const mode = p?.mode
  return mode === 'from_existing_card' || mode === 'FromExistingCard'
}

function isNovelMode(p = project.value) {
  const mode = p?.mode
  return mode === 'from_novel' || mode === 'FromNovel'
}

function modeLabel(p = project.value) {
  if (isReviseMode(p)) return '修订已有卡（另存）'
  if (isNovelMode(p)) return '小说改编'
  return '从零创作'
}

async function selectStage(stageId) {
  if (!project.value || !stageId || busy.value) return
  if (project.value.current_stage === stageId) return
  busy.value = true
  try {
    project.value = await cardstudioSetStage(project.value.id, stageId)
    syncDraftFromProject()
    statusText.value = `已切换到阶段：${stageId}`
  } catch (e) {
    await alertDialog('切换阶段失败: ' + errorText(e))
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
    await alertDialog('创建失败: ' + errorText(e))
  } finally {
    busy.value = false
  }
}

async function createFromCharacter(characterId, brief = '') {
  if (!characterId) return
  busy.value = true
  statusText.value = '正在反解析角色卡…'
  try {
    const created = await cardstudioCreateFromCharacter(characterId, brief || null)
    await refreshProjects()
    project.value = created
    allowAiFreewrite.value = !!created?.allow_ai_freewrite
    syncDraftFromProject()
    statusText.value = `已从已有卡创建修订项目（默认另存）：${created?.name || ''}`
    // revise flow lands on review
    checkReport.value = await cardstudioRunChecks(created.id)
  } catch (e) {
    await alertDialog('打开修订项目失败: ' + errorText(e))
  } finally {
    busy.value = false
  }
}

async function createFromNovel() {
  if (!novelText.value.trim()) {
    await alertDialog('请粘贴小说正文（MVP 支持节选，建议 < 40 万字）')
    return
  }
  busy.value = true
  try {
    const created = await cardstudioCreateFromNovel(
      newName.value.trim() || novelTitle.value.trim() || '小说改编',
      newBrief.value.trim() || `从小说改编：${novelTitle.value.trim() || '未命名'}`,
      novelTitle.value.trim(),
      novelText.value,
    )
    await refreshProjects()
    project.value = created
    allowAiFreewrite.value = !!created?.allow_ai_freewrite
    syncDraftFromProject()
    statusText.value = `已创建小说改编项目（摘录 ${created?.novel_excerpts?.length || 0} 段），可点「AI 预填」`
  } catch (e) {
    await alertDialog('创建小说项目失败: ' + errorText(e))
  } finally {
    busy.value = false
  }
}

async function prefillFromNovel() {
  if (!project.value || !isNovelMode()) return
  await saveArtifacts()
  busy.value = true
  statusText.value = '正在从小说摘录预填角色卡…'
  try {
    project.value = await cardstudioPrefillFromNovel(project.value.id, userNote.value || null, true)
    syncDraftFromProject()
    userNote.value = ''
    statusText.value = '小说预填完成：已进入检查阶段，可局部重跑各阶段精修'
    checkReport.value = await cardstudioRunChecks(project.value.id)
  } catch (e) {
    await alertDialog('小说预填失败: ' + errorText(e))
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

async function saveArtifacts() {
  if (!project.value) return
  busy.value = true
  try {
    const artifacts = {
      ...draft.value,
      style_notes: draft.value.style_notes?.trim() ? draft.value.style_notes : null,
    }
    project.value = await cardstudioUpdateArtifacts(project.value.id, artifacts)
    project.value = await cardstudioSetOptions(project.value.id, {
      allowAiFreewrite: allowAiFreewrite.value,
    })
    allowAiFreewrite.value = !!project.value?.allow_ai_freewrite
    syncDraftFromProject()
    statusText.value = '产物已保存'
  } catch (e) {
    await alertDialog('保存失败: ' + errorText(e))
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
    await alertDialog(errorText(e))
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
    await alertDialog(errorText(e))
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
    statusText.value = checkReport.value.ok
      ? `规则检查通过（${checkReport.value.score ?? '-'} 分）`
      : `规则检查未通过（${checkReport.value.score ?? '-'} 分）`
  } catch (e) {
    await alertDialog(errorText(e))
  } finally {
    busy.value = false
  }
}

async function runReview(useLlm = true) {
  if (!project.value) return
  await saveArtifacts()
  busy.value = true
  statusText.value = useLlm ? '正在做方法论审查（规则+LLM）…' : '正在做规则审查…'
  try {
    checkReport.value = await cardstudioRunReview(project.value.id, userNote.value || null, useLlm)
    statusText.value = checkReport.value.ok
      ? `审查通过（${checkReport.value.score ?? '-'} 分 · ${checkReport.value.source || 'rule'}）`
      : `审查未通过（${checkReport.value.score ?? '-'} 分 · ${checkReport.value.source || 'rule'}）`
  } catch (e) {
    await alertDialog(errorText(e))
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
    statusText.value = isReviseMode()
      ? `已另存为新卡：${result.character?.name || ''}（card ${result.card_id}；原卡未覆盖）`
      : `已导入角色卡：${result.character?.name || ''}（card ${result.card_id}）`
    emit('imported', result)
  } catch (e) {
    await alertDialog(errorText(e))
  } finally {
    busy.value = false
  }
}

/** 导入成功后的一键导航：可选先跑角色识别，再回卡库 */
async function goLibraryAfterImport(withExtract) {
  if (!lastImport.value) return
  const cardId = lastImport.value.card_id
  if (withExtract) {
    busy.value = true
    statusText.value = '正在识别角色…'
    try {
      const extractId =
        lastImport.value.source_character_id || lastImport.value.character?.id
      await extractCharacters(extractId, { force: true })
      statusText.value = '角色识别完成'
    } catch (e) {
      await alertDialog('角色识别失败（卡已导入，可稍后在卡库重试）: ' + errorText(e))
    } finally {
      busy.value = false
    }
  }
  emit('go-library', { cardId })
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

watch(
  () => props.seed,
  async (seed) => {
    if (!seed?.characterId) {
      // allow same card to open a new revise project next time
      seedHandledKey.value = ''
      return
    }
    // include a nonce if provided so re-open after close always creates a fresh project
    const key = `${seed.characterId}::${seed.brief || ''}::${seed.nonce || ''}`
    if (seedHandledKey.value === key) return
    seedHandledKey.value = key
    await createFromCharacter(seed.characterId, seed.brief || '')
  },
  { immediate: true, deep: true },
)
</script>

<template>
  <div class="space-y-4 max-w-4xl">
    <div class="flex items-center justify-between gap-2">
      <div>
        <div class="text-sm font-medium text-ink">写卡工作室 · 从零 / 小说 / 修订</div>
        <div class="text-xs text-ink-soft">阶段生成、小说预填或反解析已有卡 → 编辑产物 → 检查 → 另存导入</div>
      </div>
      <Button variant="ghost" size="sm" @click="emit('close')">返回卡库</Button>
    </div>

    <div class="rounded-xl border border-line bg-surface p-3 space-y-2 shadow-card">
      <div class="text-xs font-medium text-ink">新建 · 从零</div>
      <Input v-model="newName" placeholder="项目名 / 暂定角色名" />
      <textarea
        v-model="newBrief"
        rows="3"
        class="w-full rounded-md border border-line bg-surface-2 px-3 py-2 text-sm text-ink"
        placeholder="创作意图 brief：世界观类型、主角关系、想要的氛围…"
      />
      <Button variant="primary" size="md" :loading="busy" :disabled="busy" @click="createProject">从零创建</Button>
    </div>

    <div class="rounded-xl border border-line bg-surface p-3 space-y-2 shadow-card">
      <div class="text-xs font-medium text-ink">新建 · 小说改编（B · MVP）</div>
      <div class="text-[11px] text-ink-faint">
        粘贴 txt/节选（建议 &lt; 40 万字）。会切头/中/尾摘录后预填，不是整本多小时蒸馏流水线。
      </div>
      <Input v-model="novelTitle" placeholder="小说标题（可选）" />
      <textarea
        v-model="novelText"
        rows="6"
        class="w-full rounded-md border border-line bg-surface-2 px-3 py-2 text-sm text-ink font-mono"
        placeholder="粘贴小说正文或关键章节节选…"
      />
      <Button variant="primary" size="md" :loading="busy" :disabled="busy" @click="createFromNovel">创建小说项目</Button>
    </div>

    <div class="rounded-xl border border-line bg-surface p-3 shadow-card">
      <div class="text-xs font-medium text-ink mb-2">已有项目</div>
      <EmptyState v-if="!loading && projects.length === 0" title="还没有写卡项目" description="先创建一个从零 / 小说 / 修订项目" />
      <div v-else class="space-y-1">
        <div
          v-for="p in projects"
          :key="p.id"
          class="flex items-center gap-2 rounded-lg px-2 py-1 border"
          :class="project?.id === p.id ? 'border-accent bg-surface-2' : 'border-transparent'"
        >
          <button
            type="button"
            class="flex-1 min-w-0 text-left rounded-md px-1 py-1 text-sm hover:bg-surface-2"
            @click="openProject(p.id)"
          >
            <div class="font-medium text-ink truncate">{{ p.name }}</div>
            <div class="text-xs text-ink-soft truncate">
              {{ p.mode === 'from_existing_card' ? '修订另存' : (p.mode === 'from_novel' ? '小说改编' : '从零') }}
              · {{ p.current_stage }} · {{ p.updated_at }}
            </div>
          </button>
          <Button
            variant="ghost"
            size="sm"
            :disabled="busy"
            @click.stop="deleteProjectById(p.id, p.name)"
          >
            删
          </Button>
        </div>
      </div>
    </div>

    <template v-if="project">
      <div class="rounded-xl border border-line bg-surface p-3 shadow-card space-y-3">
        <div class="flex flex-wrap gap-2">
          <button
            v-for="s in stages"
            :key="s.id"
            type="button"
            class="text-xs px-2 py-1 rounded-md border border-line hover:bg-surface-2"
            :class="[stageBadgeClass(s.status), currentStage === s.id ? 'ring-1 ring-accent' : '']"
            :disabled="busy"
            @click="selectStage(s.id)"
          >
            {{ s.label }} · {{ s.status }}
          </button>
        </div>

        <div class="text-xs text-ink-soft" v-if="statusText">{{ statusText }}</div>
        <div class="text-xs text-err" v-if="project.last_error">{{ project.last_error }}</div>
        <div class="text-[11px] text-ink-faint">
          提示词包：{{ project.stage_pack_id || 'mingyue_qiuqing_v1' }}
          · 模式：{{ modeLabel() }}
          · 性格默认协作（手写衍生优先）
          <span v-if="isReviseMode() || isNovelMode()"> · 点击阶段可局部重跑</span>
        </div>
        <div v-if="project.source_character_id" class="text-[11px] text-ink-faint">
          来源角色：{{ project.source_character_id }}
          <span v-if="project.source_stored_id"> / store {{ project.source_stored_id }}</span>
        </div>
        <div v-if="isNovelMode()" class="text-[11px] text-ink-faint">
          小说：{{ project.novel_title || '（未命名）' }}
          · 摘录 {{ project.novel_excerpts?.length || 0 }} 段
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
          <label class="text-xs text-ink-soft">文风笔记 style_notes（可选，不进角色 description）</label>
          <textarea
            :value="draft.style_notes || ''"
            rows="3"
            class="w-full rounded-md border border-line bg-surface-2 px-3 py-2 text-sm text-ink"
            placeholder="白描、短句、禁用八股……（小说预填可生成）"
            @input="draft.style_notes = $event.target.value"
          />
        </div>

        <div class="flex flex-wrap gap-2">
          <Button variant="default" size="sm" :disabled="busy" @click="saveArtifacts">保存产物</Button>
          <Button
            v-if="isNovelMode()"
            variant="primary"
            size="sm"
            :loading="busy"
            :disabled="busy"
            @click="prefillFromNovel"
          >
            AI 预填（小说→卡）
          </Button>
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
          <Button variant="default" size="sm" :disabled="busy" @click="runChecks">规则检查</Button>
          <Button variant="default" size="sm" :disabled="busy" :loading="busy" @click="runReview(true)">方法论审查</Button>
          <Button variant="default" size="sm" :disabled="busy" :loading="busy" @click="exportStJson">导出 ST JSON</Button>
          <Button variant="default" size="sm" :disabled="busy" :loading="busy" @click="exportStPng">导出 ST PNG</Button>
          <Button variant="default" size="sm" :disabled="busy" :loading="busy" @click="runExportGate">出卡质量闸门</Button>
          <Button
            v-if="currentStage === 'compile_import' || project.stage_status?.review === 'done'"
            variant="primary"
            size="sm"
            :loading="busy"
            :disabled="busy"
            @click="importCompiled"
          >
            {{ isReviseMode() || isNovelMode() ? '编译并另存为新卡' : '编译并导入卡库' }}
          </Button>
          <Button variant="ghost" size="sm" :disabled="busy" @click="deleteCurrentProject">删除项目</Button>
        </div>

        <div v-if="checkReport" class="text-xs space-y-1 rounded-lg border border-line bg-surface-2/50 p-3">
          <div :class="checkReport.ok ? 'text-ok' : 'text-err'">
            检查结果：{{ checkReport.ok ? '通过' : '未通过' }}
            <span class="text-ink-soft"> · {{ checkReport.score ?? '-' }} 分 · {{ checkReport.source || 'rule' }}</span>
          </div>
          <div v-if="checkReport.summary" class="text-ink-soft">{{ checkReport.summary }}</div>
          <div v-for="(issue, i) in checkReport.issues" :key="i" class="text-ink-soft">
            <span :class="{
              'text-err': issue.severity === 'error',
              'text-warn': issue.severity === 'warning',
              'text-ink-faint': issue.severity === 'info',
            }">[{{ issue.severity }}{{ issue.field ? '/' + issue.field : '' }}]</span>
            {{ issue.message }}
            <span v-if="issue.suggestion" class="text-ink-faint"> — {{ issue.suggestion }}</span>
          </div>
        </div>

        <div v-if="gateReport" class="text-xs space-y-1 rounded-lg border border-line bg-surface-2/50 p-3">
          <div :class="gateReport.pass ? 'text-ok' : 'text-err'">
            出卡质量闸门：{{ gateReport.pass ? '全部通过' : '未通过' }}
            <span class="text-ink-soft"> · {{ gateReport.character_name || '（未命名）' }} · JSON+PNG 双路径 round-trip</span>
          </div>
          <div class="text-ink-soft">
            JSON {{ (gateReport.json_checks || []).filter((c) => c.pass).length }}/{{ (gateReport.json_checks || []).length }} 项通过
            · PNG {{ (gateReport.png_checks || []).filter((c) => c.pass).length }}/{{ (gateReport.png_checks || []).length }} 项通过
          </div>
          <template v-for="(group, gi) in [['JSON', gateReport.json_checks], ['PNG', gateReport.png_checks]]" :key="gi">
            <div v-for="(c, i) in (group[1] || []).filter((c) => !c.pass)" :key="group[0] + i">
              <span class="text-err">[{{ group[0] }} FAIL]</span>
              {{ c.name }}：{{ c.detail }}
            </div>
          </template>
          <div v-if="gateReport.warnings?.length" class="text-warn">编译警告：{{ gateReport.warnings.join('；') }}</div>
        </div>

        <div v-if="lastCompile" class="text-xs text-ink-soft">
          最近编译预览：{{ lastCompile.character_name || '（未命名）' }}
          <span v-if="lastCompile.warnings?.length"> · 警告 {{ lastCompile.warnings.length }} 条</span>
        </div>

        <div v-if="lastImport" class="text-xs space-y-2 rounded-lg border border-line bg-surface-2/50 p-3">
          <div class="text-ok">
            导入成功：{{ lastImport.character?.name }} / card {{ lastImport.card_id }}
            <span v-if="lastImport.warnings?.length">；警告 {{ lastImport.warnings.length }} 条</span>
          </div>
          <div class="flex flex-wrap gap-2">
            <Button variant="primary" size="sm" :loading="busy" :disabled="busy" @click="goLibraryAfterImport(true)">识别角色并去卡库</Button>
            <Button variant="default" size="sm" :disabled="busy" @click="goLibraryAfterImport(false)">去卡库查看</Button>
          </div>
        </div>

        <details v-if="project.last_stage_output" class="text-xs">
          <summary class="cursor-pointer text-ink-soft">最近一次模型原文</summary>
          <pre class="mt-2 whitespace-pre-wrap break-words text-ink-soft bg-surface-2 rounded-lg p-2">{{ project.last_stage_output }}</pre>
        </details>
      </div>
    </template>
  </div>
</template>
