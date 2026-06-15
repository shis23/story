<script setup>
import { ref } from 'vue'
import { formatContent } from '../utils/formatContent.js'
import {
  updateWorldInfoRoute,
  updateWorldInfoEntry,
  addWorldInfoEntry,
  deleteWorldInfoEntry,
} from '../tauri-api.js'

const props = defineProps({
  character: { type: Object, required: true },
})
const emit = defineEmits(['close', 'update-route', 'save-entry', 'delete-entry', 'add-entry'])

const showWorldInfo = ref(false)
const expandedEntry = ref(null) // 当前展开的条目索引

// 编辑状态
const editingIndex = ref(null) // 正在编辑的条目索引（null=未编辑）
const editDraft = ref({ keys: '', content: '', constant: false }) // 编辑草稿
const addingNew = ref(false) // 是否在新增条目
const newDraft = ref({ keys: '', content: '', constant: false })

// 路由选项
const routeOptions = [
  { value: 'Constant', label: '🔵 蓝灯（常驻）', desc: '进导演常驻上下文' },
  { value: 'Selective', label: '🟢 绿灯（向量池）', desc: '进向量检索池' },
  { value: 'Both', label: '🔵🟢 两者', desc: '常驻 + 向量检索' },
  { value: 'Disabled', label: '⚫ 禁用', desc: '不使用此条目' },
]

function toggleEntry(i) {
  expandedEntry.value = expandedEntry.value === i ? null : i
}

// 更新路由
async function setRoute(entryIndex, newRoute) {
  try {
    await updateWorldInfoRoute(props.character.id, entryIndex, newRoute)
    emit('update-route', { index: entryIndex, route: newRoute })
  } catch (e) {
    console.error('更新路由失败:', e)
  }
}

// 进入编辑模式
function startEdit(i, entry) {
  editingIndex.value = i
  editDraft.value = {
    keys: (entry.keys || []).join(', '),
    content: entry.content || '',
    constant: entry.constant,
  }
}

function cancelEdit() {
  editingIndex.value = null
}

async function saveEdit(i) {
  const keys = editDraft.value.keys
    .split(/[,，]/)
    .map((k) => k.trim())
    .filter(Boolean)
  try {
    await updateWorldInfoEntry(
      props.character.id,
      i,
      keys,
      editDraft.value.content,
      editDraft.value.constant
    )
    emit('save-entry', { index: i, keys, content: editDraft.value.content, constant: editDraft.value.constant })
    editingIndex.value = null
  } catch (e) {
    console.error('保存条目失败:', e)
    alert('保存失败: ' + e)
  }
}

async function handleDelete(i) {
  if (!confirm('确定删除此世界书条目？')) return
  try {
    await deleteWorldInfoEntry(props.character.id, i)
    emit('delete-entry', { index: i })
    if (expandedEntry.value === i) expandedEntry.value = null
  } catch (e) {
    console.error('删除条目失败:', e)
    alert('删除失败: ' + e)
  }
}

// 新增条目
function startAdd() {
  addingNew.value = true
  newDraft.value = { keys: '', content: '', constant: false }
}

async function saveNew() {
  const keys = newDraft.value.keys
    .split(/[,，]/)
    .map((k) => k.trim())
    .filter(Boolean)
  if (!newDraft.value.content.trim() && keys.length === 0) {
    alert('关键词和内容至少填一项')
    return
  }
  try {
    const newIndex = await addWorldInfoEntry(
      props.character.id,
      keys,
      newDraft.value.content,
      newDraft.value.constant
    )
    emit('add-entry', {
      entry: {
        keys,
        content: newDraft.value.content,
        constant: newDraft.value.constant,
        route: newDraft.value.constant ? 'Constant' : 'Selective',
      },
    })
    addingNew.value = false
    expandedEntry.value = newIndex
  } catch (e) {
    console.error('新增条目失败:', e)
    alert('新增失败: ' + e)
  }
}

</script>

<template>
  <!-- 全屏弹层 -->
  <div class="fixed inset-0 z-50 bg-black/40 backdrop-blur-sm flex items-center justify-center p-2" @click.self="emit('close')">
    <div class="bg-surface w-full h-full max-w-2xl rounded-2xl overflow-y-auto flex flex-col">
      <!-- 顶栏 -->
      <div class="sticky top-0 z-10 bg-surface/90 backdrop-blur-md border-b border-line px-4 py-3 flex items-center gap-2 shrink-0">
        <button @click="emit('close')" class="text-ink-soft hover:text-ink text-sm">← 返回</button>
        <span class="flex-1 text-center font-medium text-ink">角色详情</span>
        <span class="text-xs text-ink-soft">ST {{ character.spec_version }}</span>
      </div>

      <div class="flex-1 overflow-y-auto p-4 space-y-5">
        <!-- 基本信息 -->
        <div>
          <h2 class="text-2xl font-bold text-ink">{{ character.name }}</h2>
          <p class="text-sm text-ink-soft mt-2 leading-relaxed">{{ character.description }}</p>
        </div>

        <!-- 标签 -->
        <div v-if="character.tags?.length" class="flex flex-wrap gap-1.5">
          <span v-for="t in character.tags" :key="t" class="text-xs px-2 py-0.5 bg-accent-soft text-accent rounded-full">{{ t }}</span>
        </div>

        <!-- 详细字段 -->
        <div class="space-y-4 text-sm">
          <div v-if="character.personality">
            <div class="text-xs text-ink-soft mb-1 font-medium">性格</div>
            <div class="text-ink bg-bg rounded-xl p-3 leading-relaxed">{{ character.personality }}</div>
          </div>
          <div v-if="character.scenario">
            <div class="text-xs text-ink-soft mb-1 font-medium">场景</div>
            <div class="text-ink bg-bg rounded-xl p-3 leading-relaxed">{{ character.scenario }}</div>
          </div>
          <div v-if="character.first_mes">
            <div class="text-xs text-ink-soft mb-1 font-medium">开场白</div>
            <div class="text-ink bg-bg rounded-xl p-3 leading-relaxed prose-fiction"><span v-html="formatContent(character.first_mes)"></span></div>
          </div>
          <div v-if="character.system_prompt">
            <div class="text-xs text-ink-soft mb-1 font-medium">系统提示词</div>
            <div class="text-ink bg-bg rounded-xl p-3 font-mono text-xs leading-relaxed whitespace-pre-wrap">{{ character.system_prompt }}</div>
          </div>
        </div>

        <!-- 世界书 -->
        <div v-if="character.world_info_count > 0">
          <button
            @click="showWorldInfo = !showWorldInfo"
            class="w-full flex items-center justify-between p-3 bg-bg rounded-xl hover:bg-line transition-colors"
          >
            <span class="text-sm font-medium text-ink">📖 世界书（{{ character.world_info_count }} 条）</span>
            <span class="text-xs text-ink-soft">{{ showWorldInfo ? '收起' : '展开' }}</span>
          </button>

          <div v-if="showWorldInfo" class="mt-3 space-y-2">
            <!-- 新增条目按钮 -->
            <button
              v-if="!addingNew"
              @click="startAdd"
              class="w-full py-2 text-xs rounded-lg border border-dashed border-line text-ink-soft hover:border-accent hover:text-accent transition-colors"
            >➕ 新增条目</button>

            <!-- 新增条目表单 -->
            <div v-if="addingNew" class="p-3 bg-bg rounded-xl border border-accent-border space-y-2">
              <div class="text-xs text-ink-soft font-medium">新增条目</div>
              <input
                v-model="newDraft.keys"
                placeholder="关键词（逗号分隔）"
                class="w-full px-2 py-1.5 text-xs rounded border border-line bg-surface focus:outline-none focus:border-accent"
              />
              <textarea
                v-model="newDraft.content"
                placeholder="条目内容"
                rows="3"
                class="w-full px-2 py-1.5 text-xs rounded border border-line bg-surface focus:outline-none focus:border-accent resize-y"
              ></textarea>
              <label class="flex items-center gap-1.5 text-xs text-ink-soft">
                <input type="checkbox" v-model="newDraft.constant" />
                蓝灯（常驻，进导演上下文）
              </label>
              <div class="flex gap-2">
                <button @click="saveNew" class="flex-1 py-1.5 text-xs rounded bg-accent text-white hover:opacity-90">💾 保存</button>
                <button @click="addingNew = false" class="flex-1 py-1.5 text-xs rounded border border-line text-ink-soft hover:bg-line">取消</button>
              </div>
            </div>

            <div
              v-for="(entry, i) in character.world_info_entries"
              :key="i"
              class="p-3 bg-bg rounded-xl border border-line"
            >
              <!-- 编辑模式 -->
              <div v-if="editingIndex === i" class="space-y-2">
                <input
                  v-model="editDraft.keys"
                  placeholder="关键词（逗号分隔）"
                  class="w-full px-2 py-1.5 text-xs rounded border border-line bg-surface focus:outline-none focus:border-accent"
                />
                <textarea
                  v-model="editDraft.content"
                  rows="4"
                  class="w-full px-2 py-1.5 text-xs rounded border border-line bg-surface focus:outline-none focus:border-accent resize-y"
                ></textarea>
                <label class="flex items-center gap-1.5 text-xs text-ink-soft">
                  <input type="checkbox" v-model="editDraft.constant" />
                  蓝灯（常驻）
                </label>
                <div class="flex gap-2">
                  <button @click="saveEdit(i)" class="flex-1 py-1.5 text-xs rounded bg-accent text-white hover:opacity-90">💾 保存</button>
                  <button @click="cancelEdit" class="flex-1 py-1.5 text-xs rounded border border-line text-ink-soft hover:bg-line">取消</button>
                </div>
              </div>

              <!-- 查看模式 -->
              <div v-else>
                <!-- 条目头部 -->
                <div class="flex items-center gap-2 mb-1.5">
                  <!-- 路由选择器 -->
                  <select
                    :value="entry.route"
                    @change="setRoute(i, $event.target.value)"
                    class="text-xs px-1.5 py-0.5 rounded shrink-0 border border-line bg-bg cursor-pointer focus:outline-none focus:border-accent"
                    :class="{
                      'bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-400': entry.route === 'Constant',
                      'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400': entry.route === 'Selective',
                      'bg-purple-100 text-purple-700 dark:bg-purple-900/30 dark:text-purple-400': entry.route === 'Both',
                      'bg-gray-100 text-gray-500 dark:bg-gray-800/30 dark:text-gray-500': entry.route === 'Disabled',
                    }"
                  >
                    <option v-for="opt in routeOptions" :key="opt.value" :value="opt.value">
                      {{ opt.label }}
                    </option>
                  </select>
                  <span class="text-xs text-ink-soft truncate flex-1 cursor-pointer" @click="toggleEntry(i)">{{ entry.keys?.join(', ') }}</span>
                  <button @click="startEdit(i, entry)" class="shrink-0 text-xs px-1.5 py-0.5 rounded text-ink-soft hover:bg-line hover:text-ink" title="编辑">✏️</button>
                  <button @click="handleDelete(i)" class="shrink-0 text-xs px-1.5 py-0.5 rounded text-err/70 hover:bg-err/10" title="删除">🗑</button>
                  <span class="shrink-0 text-xs text-ink-soft/60 cursor-pointer" @click="toggleEntry(i)">{{ expandedEntry === i ? '▾' : '▸' }}</span>
                </div>

                <!-- 内容预览（折叠时） -->
                <div v-if="expandedEntry !== i" class="text-xs text-ink leading-relaxed line-clamp-2 cursor-pointer" @click="toggleEntry(i)">
                  {{ entry.content }}
                </div>

                <!-- 内容全文（展开时） -->
                <div v-else class="text-xs text-ink leading-relaxed whitespace-pre-wrap mt-2 pt-2 border-t border-line">
                  {{ entry.content }}
                </div>
              </div>
            </div>
          </div>
        </div>

        <!-- 前端资产 -->
        <div v-if="character.has_renderable_assets" class="p-3 bg-warn/10 rounded-xl border border-warn/30">
          <div class="text-sm text-warn">🎨 含前端渲染资产（HTML/JS/CSS）</div>
          <div class="text-xs text-ink-soft mt-1">插件运行时可用后可在此渲染</div>
        </div>

        <!-- 创建者 -->
        <div class="text-xs text-ink-soft text-center pt-2 pb-4">
          创建者：{{ character.creator || '未知' }}
        </div>
      </div>
    </div>
  </div>
</template>
