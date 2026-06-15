<script setup>
import { ref, onMounted } from 'vue'
import { listPresets, getPreset, deletePreset } from '../tauri-api.js'

const emit = defineEmits(['close'])

const presets = ref([])
const loading = ref(false)
const expandedId = ref(null)
const detail = ref(null)
const detailTab = ref('prompts') // 'prompts' | 'regex'

onMounted(() => { refresh() })

async function refresh() {
  loading.value = true
  try {
    presets.value = await listPresets()
  } finally {
    loading.value = false
  }
}

async function togglePreset(preset) {
  if (expandedId.value === preset.id) {
    expandedId.value = null
    detail.value = null
  } else {
    expandedId.value = preset.id
    detail.value = await getPreset(preset.id)
    detailTab.value = 'prompts'
  }
}

async function handleDelete(preset) {
  if (!confirm(`确定删除预设「${preset.name}」？`)) return
  try {
    await deletePreset(preset.id)
    if (expandedId.value === preset.id) {
      expandedId.value = null
      detail.value = null
    }
    await refresh()
  } catch (e) {
    alert('删除失败: ' + e)
  }
}

function roleBadgeClass(role) {
  if (role === 'system') return 'bg-accent/10 text-accent'
  if (role === 'user') return 'bg-ok/10 text-ok'
  return 'bg-ink-soft/10 text-ink-soft'
}
</script>

<template>
  <div class="fixed inset-0 z-50 bg-black/40 backdrop-blur-sm flex items-end sm:items-center justify-center" @click.self="emit('close')">
    <div class="bg-bg w-full max-w-lg max-h-[90vh] overflow-hidden rounded-t-2xl sm:rounded-2xl border border-line flex flex-col">

      <!-- 顶栏 -->
      <div class="sticky top-0 z-10 bg-bg border-b border-line px-4 py-3 flex items-center justify-between shrink-0">
        <button @click="emit('close')" class="text-ink-soft hover:text-ink text-sm">← 返回</button>
        <span class="font-medium text-ink text-sm">预设管理</span>
        <div class="w-12"></div>
      </div>

      <!-- 内容区 -->
      <div class="flex-1 overflow-y-auto p-4 space-y-3">
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
              <div class="text-sm font-medium text-ink truncate">{{ p.name }}</div>
              <div class="text-xs text-ink-soft">
                {{ p.prompt_count }} 条提示词 · {{ p.regex_count }} 条正则
              </div>
            </div>
            <button
              @click.stop="handleDelete(p)"
              class="shrink-0 text-xs px-1.5 py-0.5 rounded text-err/70 hover:bg-err/10"
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
                class="flex-1 py-2 text-xs font-medium transition-colors"
                :class="detailTab === 'prompts' ? 'text-accent border-b-2 border-accent' : 'text-ink-soft'"
              >提示词 ({{ detail.prompts.length }})</button>
              <button
                @click="detailTab = 'regex'"
                class="flex-1 py-2 text-xs font-medium transition-colors"
                :class="detailTab === 'regex' ? 'text-accent border-b-2 border-accent' : 'text-ink-soft'"
              >正则 ({{ detail.regex_scripts.length }})</button>
            </div>

            <div class="px-3 py-2 space-y-2 max-h-60 overflow-y-auto">
              <!-- 提示词列表 -->
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
                    <span v-if="!prompt.enabled" class="text-[10px] text-warn">禁用</span>
                    <span v-if="prompt.is_system_prompt" class="text-[10px] text-accent">系统提示词</span>
                  </div>
                  <div class="text-ink-soft whitespace-pre-wrap line-clamp-4">{{ prompt.content }}</div>
                </div>
                <div v-if="detail.prompts.length === 0" class="text-xs text-ink-soft text-center py-4">无提示词</div>
              </template>

              <!-- 正则列表 -->
              <template v-if="detailTab === 'regex'">
                <div
                  v-for="r in detail.regex_scripts"
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
    </div>
  </div>
</template>
