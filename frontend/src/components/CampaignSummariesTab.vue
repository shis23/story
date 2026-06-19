<script setup>
import { ref, onMounted } from 'vue'
import { listRoundSummaries } from '../tauri-api.js'

const props = defineProps({
  campaignId: { type: String, required: true }
})

// ─── 状态 ───
const summaries = ref([])
const loading = ref(false)
const error = ref(null)

// ─── 加载 ───
async function load() {
  if (!props.campaignId) return
  loading.value = true
  error.value = null
  try {
    summaries.value = await listRoundSummaries(props.campaignId)
  } catch (e) {
    error.value = String(e)
  } finally {
    loading.value = false
  }
}

onMounted(load)

// ─── 时间格式化（与其他组件统一用 toLocaleString） ───
function formatTime(ts) {
  if (!ts) return ''
  try {
    return new Date(ts).toLocaleString('zh-CN')
  } catch {
    return String(ts)
  }
}

// ─── 暴露 refresh 给父组件 ───
defineExpose({ refresh: load })
</script>

<template>
  <div v-if="loading" class="text-center text-ink-soft text-sm py-8">加载中…</div>
  <div v-else-if="error" class="text-center text-warn text-sm py-8">加载失败: {{ error }}</div>
  <div v-else-if="summaries.length === 0" class="text-center text-ink-soft text-sm py-8">暂无摘要</div>

  <template v-else>
    <div
      v-for="s in summaries" :key="s.id"
      class="bg-surface rounded-xl border border-line px-3 py-2.5 mb-2"
    >
      <div class="flex items-center gap-2 mb-1">
        <span class="text-xs font-medium text-ink">第 {{ s.turn }} 轮</span>
        <span v-if="s.created_at" class="text-[10px] text-ink-soft">{{ formatTime(s.created_at) }}</span>
      </div>
      <div class="text-xs text-ink-soft leading-relaxed">{{ s.content }}</div>
    </div>
  </template>
</template>
