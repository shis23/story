<script setup>
/**
 * PromptHookAuditLog — prompt hook 审计日志。
 *
 * 读 pluginStore.promptHookAuditRecords(最近 100 条,store 已裁剪)。
 * 用 ui/DataTable 渲染 pluginId / event / stage / status / durationMs / changedKeys。
 * 每条可展开看 inputSummary / outputSummary(用 ui/CodeBlock)。
 * 导出按钮调 utils/promptHookAudit.js 的 exportPromptHookAudit。
 *
 * 审计记录字段(见 utils/promptHooks.js buildAuditRecord):
 *   { kind, pluginId, pluginName, event, stage, status, durationMs,
 *     changedKeys, inputSummary, outputSummary, error }
 */
import { computed, ref } from 'vue'
import { usePluginStore } from '../../stores/index.js'
import { exportPromptHookAudit } from '../../utils/promptHookAudit.js'
import DataTable from '../ui/DataTable.vue'
import Badge from '../ui/Badge.vue'
import Button from '../ui/Button.vue'
import CodeBlock from '../ui/CodeBlock.vue'
import EmptyState from '../ui/EmptyState.vue'
import IconButton from '../ui/IconButton.vue'

const plugin = usePluginStore()

// ─── 倒序(最新在上) ───
const records = computed(() => plugin.promptHookAuditRecords.slice().reverse())

// ─── 展开行(按索引) ───
const expanded = ref(new Set())
function toggleExpand(index) {
  const next = new Set(expanded.value)
  if (next.has(index)) next.delete(index)
  else next.add(index)
  expanded.value = next
}

// ─── status → Badge variant ───
function statusVariant(status) {
  switch (status) {
    case 'ok': return 'ok'
    case 'no_change': return 'neutral'
    case 'error': return 'err'
    case 'timeout': return 'warn'
    case 'cancelled': return 'neutral'
    case 'missing_host': return 'warn'
    case 'unloaded': return 'warn'
    case 'revoked': return 'warn'
    case 'budget_exceeded': return 'warn'
    case 'audit_error': return 'err'
    default: return 'neutral'
  }
}

// ─── summary 序列化 ───
function summaryJson(summary) {
  if (!summary || Object.keys(summary).length === 0) return '{}'
  try {
    return JSON.stringify(summary, null, 2)
  } catch {
    return String(summary)
  }
}

// ─── 导出 ───
async function handleExport() {
  try {
    const json = exportPromptHookAudit(plugin.promptHookAuditRecords)
    const data = new TextEncoder().encode(json)
    const { save } = await import('@tauri-apps/plugin-dialog')
    const filePath = await save({
      defaultPath: `prompt-hook-audit-${Date.now()}.json`,
      filters: [{ name: 'JSON', extensions: ['json'] }],
    })
    if (filePath) {
      const { writeBinaryFile } = await import('@tauri-apps/plugin-fs')
      await writeBinaryFile(filePath, data)
    }
  } catch (e) {
    console.error('导出审计失败:', e)
  }
}

// ─── 复制单条记录(脱敏 JSON) ───
const copiedIndex = ref(null)
async function copyRecord(record, index) {
  try {
    await navigator.clipboard.writeText(JSON.stringify(record, null, 2))
    copiedIndex.value = index
    setTimeout(() => {
      if (copiedIndex.value === index) copiedIndex.value = null
    }, 1500)
  } catch {
    // clipboard 不可用时静默
  }
}

// ─── DataTable 配置 ───
const columns = [
  { key: 'pluginId', label: '插件', width: '140px' },
  { key: 'event', label: '事件', width: '160px' },
  { key: 'stage', label: '阶段', width: '120px' },
  { key: 'status', label: '状态', width: '90px' },
  { key: 'durationMs', label: '耗时', width: '70px' },
  { key: 'changedKeys', label: '变更字段' },
]
</script>

<template>
  <section class="flex flex-col gap-3">
    <header class="flex items-center gap-2">
      <h3 class="text-sm font-semibold text-ink">Prompt Hook 审计</h3>
      <Badge variant="neutral" size="sm">{{ records.length }} 条</Badge>
      <Button
        variant="default"
        size="sm"
        class="ml-auto"
        :disabled="records.length === 0"
        @click="handleExport"
      >
        导出
      </Button>
    </header>

    <EmptyState
      v-if="records.length === 0"
      title="无审计记录"
      description="prompt hook 插件执行后,脱敏审计记录(输入/输出摘要、变更字段、耗时)将在此显示"
    />

    <DataTable
      v-else
      :columns="columns"
      :rows="records"
      empty-title="无审计记录"
    >
      <template #cell-pluginId="{ row }">
        <div class="leading-tight">
          <div class="text-xs text-ink">{{ row.pluginName || row.pluginId }}</div>
          <div v-if="row.pluginName && row.pluginId !== row.pluginName" class="text-[10px] text-ink-faint font-mono">{{ row.pluginId }}</div>
        </div>
      </template>
      <template #cell-event="{ row }">
        <span class="text-xs font-mono text-ink-soft break-all">{{ row.event }}</span>
      </template>
      <template #cell-stage="{ row }">
        <span class="text-xs text-ink-soft">{{ row.stage || '—' }}</span>
      </template>
      <template #cell-status="{ row }">
        <Badge :variant="statusVariant(row.status)" size="sm">{{ row.status }}</Badge>
      </template>
      <template #cell-durationMs="{ row }">
        <span class="text-xs font-mono text-ink-soft">{{ row.durationMs }}ms</span>
      </template>
      <template #cell-changedKeys="{ row }">
        <div v-if="row.changedKeys && row.changedKeys.length > 0" class="flex flex-wrap gap-1">
          <Badge v-for="k in row.changedKeys" :key="k" variant="accent" size="sm">{{ k }}</Badge>
        </div>
        <span v-else class="text-xs text-ink-faint">—</span>
      </template>
      <template #row-action="{ row, index }">
        <div class="flex items-center gap-1">
          <IconButton
            size="sm"
            variant="ghost"
            :title="expanded.has(index) ? '收起' : '展开'"
            @click="toggleExpand(index)"
          >
            <span class="text-xs transition-transform" :class="expanded.has(index) ? 'rotate-90' : ''">▶</span>
          </IconButton>
          <IconButton
            size="sm"
            variant="ghost"
            title="复制记录"
            @click="copyRecord(row, index)"
          >
            <span class="text-xs">{{ copiedIndex === index ? '✓' : '⧉' }}</span>
          </IconButton>
        </div>
      </template>
    </DataTable>

    <!-- 展开详情:inputSummary / outputSummary / error -->
    <div
      v-for="(row, index) in records"
      v-show="expanded.has(index)"
      :key="`detail-${index}`"
      class="bg-surface-2 rounded-lg border border-line p-3 space-y-3"
    >
      <div class="flex items-center gap-2">
        <span class="text-xs font-medium text-ink">{{ row.pluginName || row.pluginId }}</span>
        <span class="text-xs text-ink-faint font-mono">{{ row.event }}</span>
        <Badge :variant="statusVariant(row.status)" size="sm">{{ row.status }}</Badge>
        <span class="text-xs text-ink-soft ml-auto">{{ row.durationMs }}ms</span>
      </div>

      <div v-if="row.error" class="rounded-md border border-err/40 bg-err/10 px-3 py-2">
        <div class="text-xs font-medium text-err mb-1">错误</div>
        <pre class="text-xs font-mono text-err/80 whitespace-pre-wrap break-all">{{ JSON.stringify(row.error, null, 2) }}</pre>
      </div>

      <div class="grid grid-cols-1 md:grid-cols-2 gap-2">
        <div>
          <div class="text-[10px] text-ink-faint mb-1">输入摘要</div>
          <CodeBlock :code="summaryJson(row.inputSummary)" language="input" />
        </div>
        <div>
          <div class="text-[10px] text-ink-faint mb-1">输出摘要</div>
          <CodeBlock :code="summaryJson(row.outputSummary)" language="output" />
        </div>
      </div>
    </div>
  </section>
</template>
