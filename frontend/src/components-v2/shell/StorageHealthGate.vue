<script setup>
// V4 存储健康启动拦截：检测到不可恢复的存储损坏时全屏拦截，
// 用户在「从空白开始（解除保护）」与「稍后手动修复（保持只读保护）」间二选一。
// 确认前对应文件的写入被后端写栅栏拒绝——不会把内存空态固化回盘。
// tmp 自动恢复的事件只作提示条展示，可直接关闭。
import { onMounted, ref, computed } from 'vue'
import { storageHealthAcknowledge, storageHealthReport } from '../../tauri-api.js'

const incidents = ref([])
const dismissedInfo = ref(false)
const deferred = ref(false)
const busyPath = ref('')

const blocking = computed(() => incidents.value.filter((i) => i.blocking))
const informational = computed(() =>
  incidents.value.filter((i) => !i.blocking && i.recovered_from_tmp),
)
const showGate = computed(() => blocking.value.length > 0 && !deferred.value)
const showInfoBar = computed(
  () => !showGate.value && informational.value.length > 0 && !dismissedInfo.value,
)

async function refresh() {
  try {
    incidents.value = (await storageHealthReport()) || []
  } catch (e) {
    console.error('storageHealthReport:', e)
  }
}

async function acknowledge(incident) {
  busyPath.value = incident.path
  try {
    await storageHealthAcknowledge(incident.path)
    await refresh()
  } catch (e) {
    console.error('storageHealthAcknowledge:', e)
  } finally {
    busyPath.value = ''
  }
}

function fileName(p) {
  const s = String(p || '')
  const ix = Math.max(s.lastIndexOf('/'), s.lastIndexOf('\\'))
  return ix >= 0 ? s.slice(ix + 1) : s
}

onMounted(refresh)
</script>

<template>
  <!-- 阻断拦截：损坏未确认前盖住整个应用 -->
  <div
    v-if="showGate"
    class="fixed inset-0 z-[100] bg-bg/90 backdrop-blur-sm flex items-center justify-center p-4"
  >
    <div
      class="w-full max-w-lg bg-surface border border-err/40 rounded-xl shadow-float p-5 space-y-4 max-h-[85vh] overflow-y-auto"
    >
      <div>
        <h2 class="text-base font-semibold text-err">检测到存储文件损坏</h2>
        <p class="mt-1 text-sm text-fg-muted leading-relaxed">
          以下数据文件无法读取，且自动恢复失败。为防止数据被覆盖，这些文件已进入<b>只读保护</b>：
          在你确认之前，应用不会向它们写入任何内容。损坏原文已备份为
          <code class="text-xs">.corrupt</code> 文件，可手动修复后重启应用。
        </p>
      </div>

      <div
        v-for="incident in blocking"
        :key="incident.path"
        class="border border-line rounded-lg p-3 space-y-2"
      >
        <div class="text-sm font-medium break-all">{{ fileName(incident.path) }}</div>
        <div class="text-xs text-fg-muted break-all">{{ incident.path }}</div>
        <div v-if="incident.corrupt_backup" class="text-xs text-fg-muted break-all">
          备份：{{ incident.corrupt_backup }}
        </div>
        <div class="text-xs text-err/80 break-all">{{ incident.error }}</div>
        <div class="flex gap-2 pt-1">
          <button
            class="px-3 py-1.5 text-sm rounded-lg text-err border border-err/40 hover:bg-err/10 disabled:opacity-50"
            :disabled="busyPath === incident.path"
            @click="acknowledge(incident)"
          >
            从空白开始（放弃此文件数据）
          </button>
        </div>
      </div>

      <div class="flex justify-end gap-2 pt-1 border-t border-line">
        <button
          class="px-3 py-1.5 text-sm rounded-lg border border-line hover:bg-surface-hover"
          @click="deferred = true"
        >
          稍后手动修复（保持只读保护）
        </button>
      </div>
      <p class="text-xs text-fg-muted">
        「稍后手动修复」期间应用可以浏览，但对上述文件的保存会失败并提示。修复文件后重启应用即可恢复。
      </p>
    </div>
  </div>

  <!-- 非阻断提示：主文件损坏但已从 .tmp 无损恢复 -->
  <div
    v-else-if="showInfoBar"
    class="fixed top-2 left-1/2 -translate-x-1/2 z-[90] max-w-md w-[calc(100%-2rem)] bg-surface border border-warn/40 rounded-lg shadow-float px-3 py-2 flex items-start gap-2"
  >
    <div class="text-xs text-fg leading-relaxed flex-1">
      <b class="text-warn">存储自动恢复：</b>
      {{ informational.map((i) => fileName(i.path)).join('、') }}
      的主文件曾损坏，已从临时备份无损恢复，下次保存将自动修复主文件。
    </div>
    <button
      class="text-xs text-fg-muted hover:text-fg shrink-0"
      aria-label="关闭"
      @click="dismissedInfo = true"
    >
      ✕
    </button>
  </div>
</template>
