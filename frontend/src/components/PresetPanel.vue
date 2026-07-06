<script setup>
import { ref, onMounted } from 'vue'
import { confirmDialog, alertDialog } from './base/BaseDialog.js'
import BaseOverlay from './base/BaseOverlay.vue'
import {
  listPresets,
  getPreset,
  deletePreset,
  updatePresetPrompt,
  updatePresetRegex,
  importPresetAsModules,
  setActivePreset,
  listGlobalRegexScripts,
  importGlobalRegexSettings,
  clearGlobalRegexScripts,
  updateGlobalRegex,
} from '../tauri-api.js'

const emit = defineEmits(['close'])

const presets = ref([])
const globalRegexScripts = ref([])
const loading = ref(false)
const loadingGlobalRegex = ref(false)
const expandedId = ref(null)
const detail = ref(null)
const detailTab = ref('prompts') // 'prompts' | 'regex'
const globalRegexExpanded = ref(true)
const editingPrompt = ref(null) // 正在编辑的 prompt index
const editContent = ref('')
const saving = ref(false)
const importingGlobalRegex = ref(false)

onMounted(() => { refresh() })

async function refresh() {
  loading.value = true
  loadingGlobalRegex.value = true
  try {
    const [presetList, globalRegexList] = await Promise.all([
      listPresets(),
      listGlobalRegexScripts(),
    ])
    presets.value = presetList
    globalRegexScripts.value = globalRegexList
  } finally {
    loading.value = false
    loadingGlobalRegex.value = false
  }
}

async function refreshGlobalRegexScripts() {
  loadingGlobalRegex.value = true
  try {
    globalRegexScripts.value = await listGlobalRegexScripts()
  } finally {
    loadingGlobalRegex.value = false
  }
}

async function togglePreset(preset) {
  if (expandedId.value === preset.id) {
    expandedId.value = null
    detail.value = null
    editingPrompt.value = null
  } else {
    expandedId.value = preset.id
    detail.value = await getPreset(preset.id)
    detailTab.value = 'prompts'
    editingPrompt.value = null
  }
}

async function handleDelete(preset) {
  const ok = await confirmDialog(`确定删除预设「${preset.name}」？`, { title: '删除确认' })
  if (!ok) return
  try {
    await deletePreset(preset.id)
    if (expandedId.value === preset.id) {
      expandedId.value = null
      detail.value = null
    }
    await refresh()
  } catch (e) {
    await alertDialog('删除失败: ' + e)
  }
}

// ─── 编辑功能 ──────────────────────────────────────────────────────────

function startEditPrompt(index) {
  editingPrompt.value = index
  editContent.value = detail.value.prompts[index].content
}

function cancelEdit() {
  editingPrompt.value = null
  editContent.value = ''
}

async function saveEditPrompt(index) {
  saving.value = true
  try {
    await updatePresetPrompt(expandedId.value, index, editContent.value, null)
    // 更新本地数据
    detail.value.prompts[index].content = editContent.value
    editingPrompt.value = null
    editContent.value = ''
  } catch (e) {
    await alertDialog('保存失败: ' + e)
  } finally {
    saving.value = false
  }
}

async function togglePromptEnabled(index) {
  const prompt = detail.value.prompts[index]
  saving.value = true
  try {
    await updatePresetPrompt(expandedId.value, index, null, !prompt.enabled)
    prompt.enabled = !prompt.enabled
  } catch (e) {
    await alertDialog('操作失败: ' + e)
  } finally {
    saving.value = false
  }
}

async function toggleRegexDisabled(index) {
  const regex = detail.value.regex_scripts[index]
  saving.value = true
  try {
    await updatePresetRegex(expandedId.value, index, !regex.disabled)
    regex.disabled = !regex.disabled
  } catch (e) {
    await alertDialog('操作失败: ' + e)
  } finally {
    saving.value = false
  }
}

async function importGlobalRegexFromSettings() {
  importingGlobalRegex.value = true
  saving.value = true
  try {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const filePath = await open({
      multiple: false,
      filters: [{ name: 'ST settings JSON', extensions: ['json'] }],
    })
    if (!filePath) return

    const { readTextFile } = await import('@tauri-apps/plugin-fs')
    const settingsJson = await readTextFile(filePath)
    const count = await importGlobalRegexSettings(settingsJson)
    await refreshGlobalRegexScripts()
    await alertDialog(`已导入 ${count} 条全局正则`)
  } catch (e) {
    await alertDialog('导入全局正则失败: ' + e)
  } finally {
    importingGlobalRegex.value = false
    saving.value = false
  }
}

async function clearGlobalRegex() {
  if (globalRegexScripts.value.length === 0) return
  const ok = await confirmDialog('确定清空全局正则？', { title: '清空确认' })
  if (!ok) return

  saving.value = true
  try {
    await clearGlobalRegexScripts()
    globalRegexScripts.value = []
  } catch (e) {
    await alertDialog('清空全局正则失败: ' + e)
  } finally {
    saving.value = false
  }
}

async function toggleGlobalRegexDisabled(index) {
  const regex = globalRegexScripts.value[index]
  if (!regex) return

  saving.value = true
  try {
    await updateGlobalRegex(index, !regex.disabled)
    regex.disabled = !regex.disabled
  } catch (e) {
    await alertDialog('操作全局正则失败: ' + e)
  } finally {
    saving.value = false
  }
}

async function handleImportAsModules(preset) {
  try {
    const count = await importPresetAsModules(preset.id)
    await alertDialog(`已导入 ${count} 条提示词为模块，可在导演 Agent 配置中选择使用`)
  } catch (e) {
    await alertDialog('导入失败: ' + e)
  }
}

async function toggleActivePreset(preset) {
  saving.value = true
  try {
    await setActivePreset(preset.active ? null : preset.id)
    await refresh()
    if (expandedId.value === preset.id) {
      detail.value = await getPreset(preset.id)
    }
  } catch (e) {
    await alertDialog('鎿嶄綔澶辫触: ' + e)
  } finally {
    saving.value = false
  }
}

function roleBadgeClass(role) {
  if (role === 'system') return 'bg-accent/10 text-accent'
  if (role === 'user') return 'bg-ok/10 text-ok'
  return 'bg-ink-soft/10 text-ink-soft'
}

function regexPlacementLabel(regex) {
  return regex.placement === 'input' ? '输入' : '输出'
}

function regexPlacementClass(regex) {
  return regex.placement === 'input' ? 'bg-running/10 text-running' : 'bg-ok/10 text-ok'
}
</script>

<template>
  <BaseOverlay :model-value="true" title="预设管理" size="md" position="left" @close="emit('close')">
    <template #header-extra>
      <span v-if="saving" class="text-xs text-accent">保存中…</span>
    </template>

    <!-- 内容区 -->
    <div class="p-4 space-y-3">
      <section class="bg-surface rounded-lg border border-line overflow-hidden">
        <div class="flex items-center gap-2 px-3 py-2.5">
          <button
            @click="globalRegexExpanded = !globalRegexExpanded"
            class="flex-1 min-w-0 text-left"
          >
            <div class="flex items-center gap-2 min-w-0">
              <span class="text-sm font-medium text-ink truncate">全局正则</span>
              <span class="shrink-0 px-1.5 py-0.5 rounded text-[10px] bg-accent/10 text-accent">
                {{ globalRegexScripts.length }}
              </span>
            </div>
          </button>
          <button
            @click="importGlobalRegexFromSettings"
            :disabled="importingGlobalRegex || saving"
            class="shrink-0 min-h-[36px] px-3 rounded-lg text-xs font-medium bg-accent text-white hover:opacity-90 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
            title="导入 ST settings JSON"
          >
            {{ importingGlobalRegex ? '导入中' : '导入' }}
          </button>
          <button
            @click="clearGlobalRegex"
            :disabled="globalRegexScripts.length === 0 || saving"
            class="shrink-0 min-h-[36px] px-3 rounded-lg text-xs font-medium bg-err/10 text-err hover:bg-err/20 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
            title="清空全局正则"
          >
            清空
          </button>
          <span class="text-ink-soft text-xs shrink-0">{{ globalRegexExpanded ? '▲' : '▼' }}</span>
        </div>

        <div v-if="globalRegexExpanded" class="border-t border-line px-3 py-2 space-y-2">
          <div v-if="loadingGlobalRegex" class="text-xs text-ink-soft text-center py-4">加载中...</div>
          <div v-else-if="globalRegexScripts.length === 0" class="text-xs text-ink-soft text-center py-4">无全局正则</div>
          <template v-else>
            <div
              v-for="(r, i) in globalRegexScripts"
              :key="r.id || i"
              class="bg-bg rounded-lg px-3 py-2 text-xs"
              :class="{ 'opacity-50': r.disabled }"
            >
              <div class="flex items-center gap-2 mb-1">
                <span class="font-medium text-ink truncate">{{ r.script_name || r.id }}</span>
                <span class="px-1.5 py-0.5 rounded text-[10px]" :class="regexPlacementClass(r)">
                  {{ regexPlacementLabel(r) }}
                </span>
                <span v-if="r.disabled" class="text-[10px] text-warn">禁用</span>
                <button
                  @click="toggleGlobalRegexDisabled(i)"
                  class="ml-auto min-h-[36px] px-2.5 rounded text-[10px] shrink-0"
                  :class="!r.disabled ? 'bg-ok/10 text-ok' : 'bg-warn/10 text-warn'"
                  :title="r.disabled ? '启用' : '停用'"
                >
                  {{ r.disabled ? '启用' : '停用' }}
                </button>
              </div>
              <div class="text-ink-soft font-mono text-[11px] break-all">
                <div>匹配: {{ r.find_regex }}</div>
                <div>替换: {{ r.replace_string }}</div>
              </div>
            </div>
          </template>
        </div>
      </section>
      <div v-if="loading" class="text-center text-ink-soft text-sm py-8">加载中…</div>

      <div v-else-if="presets.length === 0" class="text-center text-ink-soft text-sm py-8">
        还没有预设，请先导入
      </div>

      <div
        v-for="p in presets"
        :key="p.id"
        class="bg-surface rounded-xl border border-line overflow-hidden"
      >
        <!-- 预设头部 -->
        <div
          class="flex items-center gap-3 px-3 py-2.5 cursor-pointer hover:bg-bg transition-colors"
          @click="togglePreset(p)"
        >
          <div class="flex-1 min-w-0">
            <div class="flex items-center gap-1.5 min-w-0">
              <span class="text-sm font-medium text-ink truncate">{{ p.name }}</span>
              <span v-if="p.active" class="shrink-0 px-1.5 py-0.5 rounded text-[10px] bg-ok/10 text-ok">运行中</span>
            </div>
            <div class="text-xs text-ink-soft">
              {{ p.prompt_count }} 条提示词 · {{ p.regex_count }} 条正则
            </div>
          </div>
          <button
            @click.stop="toggleActivePreset(p)"
            class="shrink-0 w-9 h-9 flex items-center justify-center rounded-lg transition-colors"
            :class="p.active ? 'text-ok hover:bg-ok/10' : 'text-ink-soft hover:bg-bg'"
            :title="p.active ? '停用运行时预设' : '设为运行时预设'"
          >
            <span
              class="block w-2.5 h-2.5 rounded-full border"
              :class="p.active ? 'bg-ok border-ok' : 'border-ink-soft'"
            />
          </button>
          <button
            @click.stop="handleImportAsModules(p)"
            class="shrink-0 w-9 h-9 flex items-center justify-center rounded-lg text-accent/70 hover:bg-accent/10 transition-colors"
            title="导入为模块（可在导演配置中选择）"
          >📦</button>
          <button
            @click.stop="handleDelete(p)"
            class="shrink-0 w-9 h-9 flex items-center justify-center rounded-lg text-err/70 hover:bg-err/10 transition-colors"
            title="删除"
          >🗑</button>
          <span class="text-ink-soft text-xs shrink-0">{{ expandedId === p.id ? '▲' : '▼' }}</span>
        </div>

        <!-- 展开详情 -->
        <div v-if="expandedId === p.id && detail" class="border-t border-line">
          <!-- 子 tab -->
          <div class="flex border-b border-line">
            <button
              @click="detailTab = 'prompts'"
              class="flex-1 min-h-[44px] text-xs font-medium transition-colors"
              :class="detailTab === 'prompts' ? 'text-accent border-b-2 border-accent' : 'text-ink-soft'"
            >提示词 ({{ detail.prompts.length }})</button>
            <button
              @click="detailTab = 'regex'"
              class="flex-1 min-h-[44px] text-xs font-medium transition-colors"
              :class="detailTab === 'regex' ? 'text-accent border-b-2 border-accent' : 'text-ink-soft'"
            >正则 ({{ detail.regex_scripts.length }})</button>
          </div>

          <div class="px-3 py-2 space-y-2">
            <!-- 提示词列表（可编辑） -->
            <template v-if="detailTab === 'prompts'">
              <div
                v-for="(prompt, i) in detail.prompts"
                :key="i"
                class="bg-bg rounded-lg px-3 py-2 text-xs"
                :class="{ 'opacity-50': !prompt.enabled || prompt.marker }"
              >
                <div class="flex items-center gap-2 mb-1">
                  <span class="px-1.5 py-0.5 rounded text-[10px]" :class="roleBadgeClass(prompt.role)">{{ prompt.role }}</span>
                  <span class="font-medium text-ink truncate">{{ prompt.name || prompt.identifier }}</span>
                  <span v-if="prompt.marker" class="text-[10px] text-ink-soft">marker</span>
                  <span v-if="prompt.is_system_prompt" class="text-[10px] text-accent">系统提示词</span>

                  <!-- 操作按钮 -->
                  <div class="ml-auto flex gap-1 shrink-0">
                    <button
                      @click="togglePromptEnabled(i)"
                      class="min-h-[36px] px-2.5 rounded text-[10px]"
                      :class="prompt.enabled ? 'bg-ok/10 text-ok' : 'bg-warn/10 text-warn'"
                      :title="prompt.enabled ? '点击禁用' : '点击启用'"
                    >{{ prompt.enabled ? '✓' : '✕' }}</button>
                    <button
                      v-if="!prompt.marker"
                      @click="startEditPrompt(i)"
                      class="min-h-[36px] px-2.5 rounded text-[10px] bg-line text-ink-soft hover:bg-accent/10 hover:text-accent transition-colors"
                      title="编辑内容"
                    >✎</button>
                  </div>
                </div>

                <!-- 编辑模式 -->
                <div v-if="editingPrompt === i" class="mt-2">
                  <textarea
                    v-model="editContent"
                    class="w-full h-24 text-xs font-mono p-2 rounded border border-line bg-surface resize-none focus:border-accent focus:outline-none"
                  />
                  <div class="flex justify-end gap-1.5 mt-1.5">
                    <button @click="cancelEdit" class="min-h-[36px] px-3 text-[10px] rounded bg-line text-ink-soft hover:bg-bg transition-colors">取消</button>
                    <button @click="saveEditPrompt(i)" class="min-h-[36px] px-3 text-[10px] rounded bg-accent text-white hover:bg-accent/80 transition-colors">保存</button>
                  </div>
                </div>

                <!-- 查看模式 -->
                <div v-else class="text-ink-soft whitespace-pre-wrap line-clamp-4">{{ prompt.content }}</div>
              </div>
              <div v-if="detail.prompts.length === 0" class="text-xs text-ink-soft text-center py-4">无提示词</div>
            </template>

            <!-- 正则列表（可启停） -->
            <template v-if="detailTab === 'regex'">
              <div
                v-for="(r, i) in detail.regex_scripts"
                :key="r.id"
                class="bg-bg rounded-lg px-3 py-2 text-xs"
                :class="{ 'opacity-50': r.disabled }"
              >
                <div class="flex items-center gap-2 mb-1">
                  <span class="font-medium text-ink truncate">{{ r.script_name || r.id }}</span>
                  <span class="px-1.5 py-0.5 rounded text-[10px]"
                    :class="r.placement === 'input' ? 'bg-running/10 text-running' : 'bg-ok/10 text-ok'"
                  >{{ r.placement === 'input' ? '输入' : '输出' }}</span>
                  <span v-if="r.disabled" class="text-[10px] text-warn">禁用</span>

                  <!-- 启停按钮 -->
                  <button
                    @click="toggleRegexDisabled(i)"
                    class="ml-auto min-h-[36px] px-2.5 rounded text-[10px] shrink-0"
                    :class="!r.disabled ? 'bg-ok/10 text-ok' : 'bg-warn/10 text-warn'"
                    :title="r.disabled ? '点击启用' : '点击禁用'"
                  >{{ !r.disabled ? '✓' : '✕' }}</button>
                </div>
                <div class="text-ink-soft font-mono text-[11px] break-all">
                  <div>匹配: {{ r.find_regex }}</div>
                  <div>替换: {{ r.replace_string }}</div>
                </div>
              </div>
              <div v-if="detail.regex_scripts.length === 0" class="text-xs text-ink-soft text-center py-4">无正则脚本</div>
            </template>
          </div>
        </div>
      </div>
    </div>
  </BaseOverlay>
</template>
