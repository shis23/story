<script setup>
import { ref, onMounted, computed } from 'vue'
import { listConnections, setActiveConnection as apiSetActive, listModules, getActiveProfile, saveProfile, updateModule } from '../tauri-api.js'

const connections = ref([])
const activeConnId = ref(null)
const showConnList = ref(false)

// 模块系统
const allModules = ref([])
const activeProfile = ref(null)
const loadingModules = ref(false)
const saving = ref(false)

const emit = defineEmits(['open-connection-config'])

onMounted(async () => {
  await loadConnections()
  await loadModules()
})

async function loadConnections() {
  try {
    connections.value = await listConnections()
    activeConnId.value = connections.value.find((c) => c.active)?.id || null
  } catch (e) {
    console.error('加载连接失败:', e)
  }
}

const activeConnName = () => {
  const c = connections.value.find((x) => x.id === activeConnId.value)
  return c ? c.name : '未选择'
}

async function selectConn(id) {
  showConnList.value = false
  try {
    await apiSetActive(id)
    activeConnId.value = id
    await loadConnections()
  } catch (e) {
    console.error('切换连接失败:', e)
  }
}

// ─── 模块系统 ──────────────────────────────────────────────────────────

const categoryLabels = {
  Perspective: '视角',
  Style: '文风',
  Cot: '思维链',
  Quality: '约束',
  Output: '输出',
  Tone: '基调',
}

const singleCategories = new Set(['Perspective', 'Style', 'Cot', 'Tone'])

async function loadModules() {
  loadingModules.value = true
  try {
    const [modules, profile] = await Promise.all([listModules(), getActiveProfile()])
    allModules.value = modules || []
    activeProfile.value = profile || null
  } catch (e) {
    console.error('加载模块失败:', e)
  } finally {
    loadingModules.value = false
  }
}

// 按 category 分组的模块
const modulesByCategory = computed(() => {
  const groups = {}
  for (const m of allModules.value) {
    if (!groups[m.category]) groups[m.category] = []
    groups[m.category].push(m)
  }
  return groups
})

// 当前 Profile 中每个 category 选中的模块 ID
function getSelectedIds(category) {
  if (!activeProfile.value?.selections) return []
  // selections: { "Director": { "Perspective": ["id1"], ... }, ... }
  // 我们显示 Editor 的选择（最直观）
  const editorSelections = activeProfile.value.selections['Editor'] || {}
  return editorSelections[category] || []
}

function isSelected(category, moduleId) {
  return getSelectedIds(category).includes(moduleId)
}

async function toggleModule(category, moduleId) {
  if (!activeProfile.value) return

  const isSingle = singleCategories.has(category)
  const selections = JSON.parse(JSON.stringify(activeProfile.value.selections))

  // 确保 Editor 的 category 存在
  if (!selections['Editor']) selections['Editor'] = {}
  if (!selections['Editor'][category]) selections['Editor'][category] = []

  let ids = selections['Editor'][category]
  const idx = ids.indexOf(moduleId)

  if (isSingle) {
    // 单选：切换到新选项
    ids = [moduleId]
  } else {
    // 多选：toggle
    if (idx >= 0) {
      ids.splice(idx, 1)
    } else {
      ids.push(moduleId)
    }
  }
  selections['Editor'][category] = ids

  // 同步更新 Subagent("*") 的 Output 选择
  // 注意：后端 AgentRole 序列化为扁平字符串，Subagent("*") → "Subagent:*"
  if (category === 'Output') {
    if (!selections['Subagent:*']) selections['Subagent:*'] = {}
    selections['Subagent:*']['Output'] = [...ids]
  }

  // 保存
  saving.value = true
  try {
    const profile = { ...activeProfile.value, selections }
    await saveProfile(JSON.stringify(profile))
    activeProfile.value = profile
  } catch (e) {
    console.error('保存 Profile 失败:', e)
  } finally {
    saving.value = false
  }
}

async function toggleModuleEnabled(module) {
  try {
    await updateModule(module.id, null, !module.enabled)
    await loadModules()
  } catch (e) {
    console.error('更新模块状态失败:', e)
  }
}

// 保存当前选择为新命名预设
async function saveAsNewProfile() {
  if (!activeProfile.value) return
  const name = window.prompt('新预设名称：', '我的预设')
  if (!name) return
  saving.value = true
  try {
    const profile = { ...activeProfile.value, id: `profile-${Date.now()}`, name }
    await saveProfile(JSON.stringify(profile))
    activeProfile.value = profile
    await loadModules() // 刷新（新预设可能成为活跃）
  } catch (e) {
    console.error('保存预设失败:', e)
    alert('保存预设失败: ' + e)
  } finally {
    saving.value = false
  }
}

defineExpose({ loadConnections })
</script>

<template>
  <div class="mx-4 my-3 bg-surface rounded-2xl border border-line p-4">
    <div class="flex items-center gap-2 mb-3">
      <span class="text-base">🎬</span>
      <span class="text-sm font-medium text-ink">导演 Agent</span>
      <span class="ml-auto text-xs text-ink-soft">
        {{ activeProfile?.name || '默认预设' }}
        <span v-if="saving" class="text-accent ml-1">保存中…</span>
      </span>
    </div>

    <!-- 加载中 -->
    <div v-if="loadingModules" class="text-center text-ink-soft text-xs py-4">加载模块…</div>

    <!-- 各模块组 -->
    <div v-else class="space-y-3">
      <div v-for="(mods, cat) in modulesByCategory" :key="cat">
        <div class="text-[11px] text-ink-soft mb-1.5">{{ categoryLabels[cat] || cat }}</div>
        <div class="flex flex-wrap gap-1.5">
          <button
            v-for="m in mods"
            :key="m.id"
            @click="toggleModule(cat, m.id)"
            class="px-2.5 py-1 rounded-full text-xs transition-all relative"
            :class="[
              isSelected(cat, m.id)
                ? 'bg-accent text-white'
                : 'bg-bg text-ink-soft hover:bg-line',
              !m.enabled ? 'opacity-40' : ''
            ]"
            :title="!m.enabled ? '已禁用（点击切换选择）' : m.content?.substring(0, 80)"
          >
            {{ m.name }}
            <span v-if="!m.enabled" class="absolute -top-1 -right-1 text-[8px] text-warn">✕</span>
          </button>
        </div>
      </div>
    </div>

    <!-- 连接选择（接真实后端） -->
    <div class="mt-3 pt-3 border-t border-line">
      <div class="flex items-center justify-between mb-1.5">
        <span class="text-[11px] text-ink-soft">连接</span>
        <button
          @click="emit('open-connection-config')"
          class="text-[11px] text-accent hover:underline"
        >+ 新建/管理</button>
      </div>

      <!-- 无连接提示 -->
      <div v-if="!connections.length" class="p-2.5 rounded-lg border border-dashed border-line text-center">
        <div class="text-xs text-ink-soft mb-1">尚未配置连接</div>
        <button
          @click="emit('open-connection-config')"
          class="text-xs text-accent hover:underline"
        >点此配置 →</button>
      </div>

      <!-- 有连接：下拉选择 -->
      <div v-else class="relative">
        <button
          @click="showConnList = !showConnList"
          class="w-full flex items-center justify-between px-3 py-2 bg-bg rounded-lg text-sm hover:bg-line"
        >
          <span :class="activeConnId ? 'text-ink' : 'text-ink-soft'">{{ activeConnName() }}</span>
          <span class="text-xs text-ink-soft">▾</span>
        </button>
        <div v-if="showConnList" class="absolute left-0 right-0 top-full mt-1 bg-surface border border-line rounded-lg shadow-lg py-1 z-10">
          <button
            v-for="c in connections"
            :key="c.id"
            @click="selectConn(c.id)"
            class="w-full text-left px-3 py-2 text-sm hover:bg-accent-soft"
            :class="c.id === activeConnId ? 'text-accent' : ''"
          >
            {{ c.name }}
            <span class="text-xs text-ink-soft ml-2">{{ c.model }}</span>
            <span v-if="c.active" class="text-[10px] text-accent ml-1">●</span>
          </button>
        </div>
      </div>
    </div>

    <!-- 保存预设 -->
    <button
      @click="saveAsNewProfile"
      class="w-full mt-3 py-2 text-xs text-accent border border-dashed border-accent-border rounded-lg hover:bg-accent-soft"
    >
      💾 保存当前为新预设
    </button>
  </div>
</template>
