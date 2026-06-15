<script setup>
import { promptModules } from '../mock.js'
import { ref, onMounted } from 'vue'
import { listConnections, setActiveConnection as apiSetActive } from '../tauri-api.js'

const modules = ref(promptModules)
const connections = ref([])
const activeConnId = ref(null)
const showConnList = ref(false)

const emit = defineEmits(['open-connection-config'])

onMounted(async () => {
  await loadConnections()
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

// 单选组：点新的，同组其他取消
function selectSingle(category, id) {
  modules.value[category].forEach(m => m.selected = (m.id === id))
}
// 多选组：切换
function toggleMulti(category, id) {
  const m = modules.value[category].find(x => x.id === id)
  if (m) m.selected = !m.selected
}

const categoryMeta = {
  perspective: { label: '视角', single: true },
  style: { label: '文风', single: true },
  cot: { label: '思维链', single: true },
  quality: { label: '约束', single: false },
}

defineExpose({ loadConnections })
</script>

<template>
  <div class="mx-4 my-3 bg-surface rounded-2xl border border-line p-4">
    <div class="flex items-center gap-2 mb-3">
      <span class="text-base">🎬</span>
      <span class="text-sm font-medium text-ink">导演 Agent</span>
      <span class="ml-auto text-xs text-ink-soft">小说预设v2</span>
    </div>

    <!-- 各模块组 -->
    <div class="space-y-3">
      <div v-for="(mods, cat) in categoryMeta" :key="cat">
        <div class="text-[11px] text-ink-soft mb-1.5">{{ mods.label }}</div>
        <div class="flex flex-wrap gap-1.5">
          <button
            v-for="m in modules[cat]"
            :key="m.id"
            @click="mods.single ? selectSingle(cat, m.id) : toggleMulti(cat, m.id)"
            class="px-2.5 py-1 rounded-full text-xs transition-all"
            :class="m.selected
              ? 'bg-accent text-white'
              : 'bg-bg text-ink-soft hover:bg-line'"
          >
            {{ m.name }}
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
    <button class="w-full mt-3 py-2 text-xs text-accent border border-dashed border-accent-border rounded-lg hover:bg-accent-soft">
      💾 保存当前为新预设
    </button>
  </div>
</template>
