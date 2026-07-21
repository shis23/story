<script setup>
import { ref, onMounted } from 'vue'
import { confirmDialog, alertDialog } from '../../components/base/BaseDialog.js'
import { regexPlacementClass, regexPlacementLabel } from '../../utils/regexPlacement.js'
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
} from '../../tauri-api.js'
import PanelHost from '../shell/PanelHost.vue'
import Tabs from '../ui/Tabs.vue'
import Button from '../ui/Button.vue'
import Badge from '../ui/Badge.vue'
import LoadingState from '../ui/LoadingState.vue'
import EmptyState from '../ui/EmptyState.vue'

const emit = defineEmits(['close'])

// ─── 顶层 Tab：presets / regex ───
const activeTopTab = ref('presets')
const topTabs = [
  { key: 'presets', label: '预设' },
  { key: 'regex', label: '全局正则' },
]

const presets = ref([])
const globalRegexScripts = ref([])
const loading = ref(false)
const loadingGlobalRegex = ref(false)
const expandedId = ref(null)
const detail = ref(null)
const detailTab = ref('prompts') // 'prompts' | 'regex'
const editingPrompt = ref(null) // 正在编辑的 prompt index
const editContent = ref('')
const saving = ref(false)
const importingGlobalRegex = ref(false)

const detailSubTabs = [
  { key: 'prompts', label: '提示词' },
  { key: 'regex', label: '正则' },
]

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

// ─── 提示词编辑 ───

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
    await alertDialog('操作失败: ' + e)
  } finally {
    saving.value = false
  }
}

// 角色 badge variant 映射
function roleBadgeVariant(role) {
  if (role === 'system') return 'accent'
  if (role === 'user') return 'ok'
  return 'neutral'
}

// placement badge variant 映射（与 regexPlacementClass 对齐）
function placementBadgeVariant(regex) {
  const cls = regexPlacementClass(regex)
  if (cls.includes('accent')) return 'accent'
  if (cls.includes('ok')) return 'ok'
  if (cls.includes('running')) return 'accent'
  if (cls.includes('warn')) return 'warn'
  return 'neutral'
}
</script>

<template>
  <PanelHost :show="true" title="预设管理" side="left" @close="emit('close')">
    <template #header>
      <div class="flex items-center gap-2 min-w-0">
        <h2 class="text-sm font-semibold text-ink truncate">预设管理</h2>
        <span v-if="saving" class="text-xs text-accent shrink-0">保存中…</span>
      </div>
    </template>

    <div class="px-4 pt-3">
      <Tabs v-model="activeTopTab" :tabs="topTabs">
        <!-- ═══ Tab: 预设 ═══ -->
        <div v-if="activeTopTab === 'presets'" class="space-y-3 pb-4">
          <LoadingState v-if="loading" />

          <EmptyState
            v-else-if="presets.length === 0"
            title="还没有预设"
            description="请先导入"
          />

          <div
            v-for="p in presets"
            :key="p.id"
            v-else
            class="bg-surface rounded-xl border border-line overflow-hidden"
          >
            <!-- 预设头部 -->
            <div
              class="flex items-center gap-3 px-3 py-2.5 cursor-pointer hover:bg-surface-2 transition-colors"
              @click="togglePreset(p)"
            >
              <div class="flex-1 min-w-0">
                <div class="flex items-center gap-1.5 min-w-0">
                  <span class="text-sm font-medium text-ink truncate">{{ p.name }}</span>
                  <Badge v-if="p.active" variant="ok" size="sm">运行中</Badge>
                </div>
                <div class="text-xs text-ink-soft">
                  {{ p.prompt_count }} 条提示词 · {{ p.regex_count }} 条正则
                </div>
              </div>
              <button
                @click.stop="toggleActivePreset(p)"
                class="shrink-0 w-8 h-8 flex items-center justify-center rounded-lg transition-colors"
                :class="p.active ? 'text-ok hover:bg-ok/15' : 'text-ink-soft hover:bg-surface-2'"
                :title="p.active ? '停用运行时预设' : '设为运行时预设'"
              >
                <span
                  class="block w-2.5 h-2.5 rounded-full border"
                  :class="p.active ? 'bg-ok border-ok' : 'border-ink-soft'"
                />
              </button>
              <button
                @click.stop="handleImportAsModules(p)"
                class="shrink-0 w-8 h-8 flex items-center justify-center rounded-lg text-accent/70 hover:bg-accent-soft transition-colors"
                title="导入为模块（可在导演配置中选择）"
              >
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M20.5 16v-7.5a2 2 0 0 0-1-1.73l-6.5-3.7a2 2 0 0 0-2 0L4.5 6.77a2 2 0 0 0-1 1.73V16a2 2 0 0 0 1 1.73l6.5 3.7a2 2 0 0 0 2 0l6.5-3.7a2 2 0 0 0 1-1.73z"/><path d="M4 7.2l8 4.55 8-4.55"/></svg>
              </button>
              <button
                @click.stop="handleDelete(p)"
                class="shrink-0 w-8 h-8 flex items-center justify-center rounded-lg text-err/70 hover:bg-err/15 transition-colors"
                title="删除"
              >
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
              </button>
              <span class="text-ink-soft text-xs shrink-0">{{ expandedId === p.id ? '▲' : '▼' }}</span>
            </div>

            <!-- 展开详情 -->
            <div v-if="expandedId === p.id && detail" class="border-t border-line">
              <!-- 子 tab -->
              <div class="px-3 pt-2">
                <Tabs v-model="detailTab" :tabs="detailSubTabs">
                  <div class="space-y-2">
                    <!-- 提示词列表（可编辑） -->
                    <template v-if="detailTab === 'prompts'">
                      <div
                        v-for="(prompt, i) in detail.prompts"
                        :key="i"
                        class="bg-bg rounded-lg px-3 py-2 text-xs"
                        :class="{ 'opacity-50': !prompt.enabled || prompt.marker }"
                      >
                        <div class="flex items-center gap-2 mb-1 flex-wrap">
                          <Badge :variant="roleBadgeVariant(prompt.role)" size="sm">{{ prompt.role }}</Badge>
                          <span class="font-medium text-ink truncate">{{ prompt.name || prompt.identifier }}</span>
                          <span v-if="prompt.marker" class="text-[10px] text-ink-soft">marker</span>
                          <Badge v-if="prompt.is_system_prompt" variant="accent" size="sm">系统提示词</Badge>

                          <div class="ml-auto flex gap-1 shrink-0">
                            <Button
                              variant="ghost"
                              size="sm"
                              @click="togglePromptEnabled(i)"
                              :title="prompt.enabled ? '点击禁用' : '点击启用'"
                            >{{ prompt.enabled ? '✓' : '✕' }}</Button>
                            <Button
                              v-if="!prompt.marker"
                              variant="ghost"
                              size="sm"
                              title="编辑内容"
                              @click="startEditPrompt(i)"
                            >✎</Button>
                          </div>
                        </div>

                        <!-- 编辑模式 -->
                        <div v-if="editingPrompt === i" class="mt-2 space-y-1.5">
                          <textarea
                            v-model="editContent"
                            class="w-full h-24 text-xs font-mono p-2 rounded border border-line bg-bg resize-none focus:border-accent focus:outline-none"
                          />
                          <div class="flex justify-end gap-1.5">
                            <Button variant="default" size="sm" @click="cancelEdit">取消</Button>
                            <Button variant="primary" size="sm" @click="saveEditPrompt(i)">保存</Button>
                          </div>
                        </div>

                        <!-- 查看模式 -->
                        <div v-else class="text-ink-soft whitespace-pre-wrap line-clamp-4">{{ prompt.content }}</div>
                      </div>
                      <EmptyState v-if="detail.prompts.length === 0" title="无提示词" />
                    </template>

                    <!-- 正则列表（可启停） -->
                    <template v-if="detailTab === 'regex'">
                      <div
                        v-for="(r, i) in detail.regex_scripts"
                        :key="r.id"
                        class="bg-bg rounded-lg px-3 py-2 text-xs"
                        :class="{ 'opacity-50': r.disabled }"
                      >
                        <div class="flex items-center gap-2 mb-1 flex-wrap">
                          <span class="font-medium text-ink truncate">{{ r.script_name || r.id }}</span>
                          <Badge :variant="placementBadgeVariant(r)" size="sm">
                            {{ regexPlacementLabel(r) }}
                          </Badge>
                          <Badge v-if="r.disabled" variant="warn" size="sm">禁用</Badge>

                          <Button
                            variant="ghost"
                            size="sm"
                            class="ml-auto"
                            :title="r.disabled ? '点击启用' : '点击禁用'"
                            @click="toggleRegexDisabled(i)"
                          >{{ !r.disabled ? '✓' : '✕' }}</Button>
                        </div>
                        <div class="text-ink-soft font-mono text-[11px] break-all">
                          <div>匹配: {{ r.find_regex }}</div>
                          <div>替换: {{ r.replace_string }}</div>
                        </div>
                      </div>
                      <EmptyState v-if="detail.regex_scripts.length === 0" title="无正则脚本" />
                    </template>
                  </div>
                </Tabs>
              </div>
            </div>
          </div>
        </div>

        <!-- ═══ Tab: 全局正则 ═══ -->
        <div v-if="activeTopTab === 'regex'" class="space-y-3 pb-4">
          <!-- 操作条 -->
          <div class="flex items-center justify-end gap-2">
            <Badge variant="accent" size="md">{{ globalRegexScripts.length }}</Badge>
            <Button
              variant="primary"
              size="sm"
              :loading="importingGlobalRegex"
              :disabled="saving"
              title="导入 ST settings JSON"
              @click="importGlobalRegexFromSettings"
            >{{ importingGlobalRegex ? '导入中' : '导入' }}</Button>
            <Button
              variant="danger"
              size="sm"
              :disabled="globalRegexScripts.length === 0 || saving"
              title="清空全局正则"
              @click="clearGlobalRegex"
            >清空</Button>
          </div>

          <LoadingState v-if="loadingGlobalRegex" />

          <EmptyState v-else-if="globalRegexScripts.length === 0" title="无全局正则" />

          <template v-else>
            <div
              v-for="(r, i) in globalRegexScripts"
              :key="r.id || i"
              class="bg-surface rounded-lg border border-line px-3 py-2 text-xs"
              :class="{ 'opacity-50': r.disabled }"
            >
              <div class="flex items-center gap-2 mb-1 flex-wrap">
                <span class="font-medium text-ink truncate">{{ r.script_name || r.id }}</span>
                <Badge :variant="placementBadgeVariant(r)" size="sm">
                  {{ regexPlacementLabel(r) }}
                </Badge>
                <Badge v-if="r.disabled" variant="warn" size="sm">禁用</Badge>
                <Button
                  variant="ghost"
                  size="sm"
                  class="ml-auto"
                  :title="r.disabled ? '启用' : '停用'"
                  @click="toggleGlobalRegexDisabled(i)"
                >{{ r.disabled ? '启用' : '停用' }}</Button>
              </div>
              <div class="text-ink-soft font-mono text-[11px] break-all">
                <div>匹配: {{ r.find_regex }}</div>
                <div>替换: {{ r.replace_string }}</div>
              </div>
            </div>
          </template>
        </div>
      </Tabs>
    </div>
  </PanelHost>
</template>
