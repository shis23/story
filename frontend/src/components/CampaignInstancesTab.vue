<script setup>
import { ref, onMounted } from 'vue'
import { confirmDialog, alertDialog } from './base/BaseDialog.js'
import {
  listInstances, getCharacterVariables, setCharacterVariable,
  promoteTemporaryInstance
} from '../tauri-api.js'

const props = defineProps({
  campaignId: { type: String, required: true }
})

const emit = defineEmits(['refresh'])

// ─── 状态 ───
const instances = ref([])
const loading = ref(false)
const error = ref(null)
const expandedInstanceId = ref(null)
const instanceVariables = ref([])
const promotingInstanceId = ref(null)

// ─── 加载 ───
async function load() {
  if (!props.campaignId) return
  loading.value = true
  error.value = null
  try {
    instances.value = await listInstances(props.campaignId)
  } catch (e) {
    error.value = String(e)
  } finally {
    loading.value = false
  }
}

onMounted(load)

// ─── 展开/收起实例变量 ───
async function toggleInstance(inst) {
  if (expandedInstanceId.value === inst.id) {
    expandedInstanceId.value = null
    instanceVariables.value = []
  } else {
    expandedInstanceId.value = inst.id
    try {
      instanceVariables.value = await getCharacterVariables(props.campaignId, inst.id)
    } catch (e) {
      instanceVariables.value = []
    }
  }
}

// ─── 变量类型推断（从 JSON value 推断，不依赖后端 schema） ───
function inferVarType(value) {
  if (typeof value === 'boolean') return 'bool'
  if (typeof value === 'number') return Number.isInteger(value) ? 'int' : 'float'
  if (Array.isArray(value) || (typeof value === 'object' && value !== null)) return 'json'
  return 'string'
}

// ─── 变量编辑 ───
async function handleVariableChange(instanceId, key, value, varType) {
  try {
    let parsed = value
    if (varType === 'bool') {
      parsed = value === 'true' || value === true
    } else if (varType === 'int') {
      parsed = parseInt(value, 10)
      if (isNaN(parsed)) parsed = value
    } else if (varType === 'float') {
      parsed = parseFloat(value)
      if (isNaN(parsed)) parsed = value
    }
    // string / json 保持原样

    await setCharacterVariable(props.campaignId, instanceId, key, parsed)
    instanceVariables.value = await getCharacterVariables(props.campaignId, instanceId)
  } catch (e) {
    await alertDialog('设置变量失败: ' + e)
  }
}

// ─── 升格临时实例 ───
async function handlePromoteTemporary(inst) {
  const ok = await confirmDialog(`确定将「${inst.name || inst.character_name}」升格为常驻角色？`, { title: '升格确认', kind: 'info' })
  if (!ok) return
  promotingInstanceId.value = inst.id
  try {
    await promoteTemporaryInstance(props.campaignId, inst.id)
    await load()
    emit('refresh')
  } catch (e) {
    await alertDialog('升格失败: ' + e)
  } finally {
    promotingInstanceId.value = null
  }
}

// ─── JSON 格式化 ───
function formatJson(value) {
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

// ─── 暴露 refresh 给父组件 ───
defineExpose({ refresh: load })
</script>

<template>
  <div v-if="loading" class="text-center text-ink-soft text-sm py-8">加载中…</div>
  <div v-else-if="error" class="text-center text-warn text-sm py-8">加载失败: {{ error }}</div>
  <div v-else-if="instances.length === 0" class="text-center text-ink-soft text-sm py-8">暂无角色实例</div>

  <template v-else>
    <div
      v-for="inst in instances" :key="inst.id"
      class="bg-surface rounded-xl border border-line overflow-hidden mb-2"
    >
      <!-- 实例头部 -->
      <div
        class="flex items-center gap-3 px-3 py-2.5 cursor-pointer hover:bg-bg transition-colors"
        @click="toggleInstance(inst)"
      >
        <div class="flex-1 min-w-0">
          <div class="text-sm font-medium text-ink truncate">
            {{ inst.name || inst.character_name }}
            <span v-if="inst.is_temporary" class="ml-1 px-1.5 py-0.5 rounded text-[10px] bg-warn/10 text-warn">临时</span>
          </div>
          <div class="text-xs text-ink-soft">
            {{ inst.role_type || '' }}
            <span v-if="inst.is_active" class="text-ok ml-1">● 存活</span>
            <span v-else class="text-ink-soft ml-1">○ 离场</span>
          </div>
        </div>
        <button
          v-if="inst.is_temporary"
          @click.stop="handlePromoteTemporary(inst)"
          :disabled="promotingInstanceId === inst.id"
          class="min-h-[36px] px-3 rounded-full text-[10px] font-medium bg-accent/10 text-accent hover:bg-accent/20 disabled:opacity-50 transition-colors"
        >{{ promotingInstanceId === inst.id ? '升格中…' : '升格为常驻' }}</button>
        <span class="text-ink-soft text-xs">{{ expandedInstanceId === inst.id ? '▲' : '▼' }}</span>
      </div>

      <!-- 实例展开：变量编辑 -->
      <div v-if="expandedInstanceId === inst.id" class="border-t border-line px-3 py-2 space-y-2">
        <div class="text-xs font-medium text-ink-soft mb-1">变量</div>
        <div v-if="instanceVariables.length === 0" class="text-xs text-ink-soft">暂无变量</div>
        <div
          v-for="v in instanceVariables" :key="v.key"
          class="flex items-center gap-2"
        >
          <span class="text-xs text-ink-soft w-24 truncate shrink-0" :title="v.key">{{ v.key }}</span>

          <!-- bool → checkbox -->
          <template v-if="inferVarType(v.value) === 'bool'">
            <label class="flex items-center gap-1 cursor-pointer">
              <input
                type="checkbox"
                :checked="v.value === true"
                @change="handleVariableChange(inst.id, v.key, $event.target.checked, 'bool')"
                class="accent-accent"
              />
              <span class="text-xs text-ink">{{ v.value ? 'true' : 'false' }}</span>
            </label>
          </template>

          <!-- int / float → number input -->
          <template v-else-if="inferVarType(v.value) === 'int' || inferVarType(v.value) === 'float'">
            <input
              type="number"
              :value="v.value"
              :step="inferVarType(v.value) === 'float' ? '0.01' : '1'"
              @change="handleVariableChange(inst.id, v.key, $event.target.value, inferVarType(v.value))"
              class="flex-1 min-h-[36px] px-2 text-xs rounded border border-line bg-bg focus:outline-none focus:border-accent"
            />
          </template>

          <!-- json → textarea (只读折叠) -->
          <template v-else-if="inferVarType(v.value) === 'json'">
            <textarea
              :value="formatJson(v.value)"
              @change="handleVariableChange(inst.id, v.key, $event.target.value, 'json')"
              rows="3"
              class="flex-1 px-2 py-1.5 text-xs rounded border border-line bg-bg focus:outline-none focus:border-accent resize-none font-mono"
            ></textarea>
          </template>

          <!-- string → text input -->
          <template v-else>
            <input
              type="text"
              :value="String(v.value)"
              @change="handleVariableChange(inst.id, v.key, $event.target.value, 'string')"
              class="flex-1 min-h-[36px] px-2 text-xs rounded border border-line bg-bg focus:outline-none focus:border-accent"
            />
          </template>
        </div>
      </div>
    </div>
  </template>
</template>
