<script setup>
import { ref, onMounted, watch } from 'vue'
import { confirmDialog, alertDialog } from '../../components/base/BaseDialog.js'
import MvuStatusBar from '../../components/MvuStatusBar.vue'
import {
  listInstances, getCharacterVariables, setCharacterVariable,
  promoteTemporaryInstance, getCampaign, getCard, metaGetMvuTranslation
} from '../../tauri-api.js'
import { formatJsonValue as formatJson, inferVarType } from '../../utils/campaignDisplay.js'
import {
  buildInstanceMvuStatusBarProps,
  shouldApplyInstanceMvuLoad,
} from '../../utils/campaignMvuStatusBar.js'
import DataTable from '../ui/DataTable.vue'
import Badge from '../ui/Badge.vue'
import Button from '../ui/Button.vue'
import EmptyState from '../ui/EmptyState.vue'
import LoadingState from '../ui/LoadingState.vue'

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
const instanceMvuStatusBar = ref(null)
const promotingInstanceId = ref(null)
let instanceDetailLoadToken = 0

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

watch(() => props.campaignId, () => {
  expandedInstanceId.value = null
  instanceVariables.value = []
  instanceMvuStatusBar.value = null
  instanceDetailLoadToken += 1
  if (props.campaignId) load()
})

function isCurrentInstanceLoad(inst, token) {
  return shouldApplyInstanceMvuLoad({
    expandedInstanceId: expandedInstanceId.value,
    instanceId: inst?.id,
    token,
    currentToken: instanceDetailLoadToken,
  })
}

async function loadInstanceMvuStatusBar(inst, variables, token) {
  instanceMvuStatusBar.value = null
  if (!props.campaignId || !inst?.definition_id) return

  try {
    const campaign = await getCampaign(props.campaignId)
    if (!campaign?.card_id) return

    const card = await getCard(campaign.card_id)
    if (!card?.source_character_id) return

    const translationDetail = await metaGetMvuTranslation(card.source_character_id)
    if (!isCurrentInstanceLoad(inst, token)) return

    instanceMvuStatusBar.value = buildInstanceMvuStatusBarProps({
      instance: inst,
      card,
      translationDetail,
      variables,
    })
  } catch {
    if (isCurrentInstanceLoad(inst, token)) {
      instanceMvuStatusBar.value = null
    }
  }
}

async function refreshInstanceVariables(inst, token) {
  const variables = await getCharacterVariables(props.campaignId, inst.id)
  if (!isCurrentInstanceLoad(inst, token)) return
  instanceVariables.value = variables
  await loadInstanceMvuStatusBar(inst, variables, token)
}

// ─── 展开/收起实例变量 ───
async function toggleInstance(inst) {
  if (expandedInstanceId.value === inst.id) {
    expandedInstanceId.value = null
    instanceVariables.value = []
    instanceMvuStatusBar.value = null
    instanceDetailLoadToken += 1
  } else {
    const token = ++instanceDetailLoadToken
    expandedInstanceId.value = inst.id
    instanceVariables.value = []
    instanceMvuStatusBar.value = null
    try {
      await refreshInstanceVariables(inst, token)
    } catch (e) {
      if (isCurrentInstanceLoad(inst, token)) {
        instanceVariables.value = []
        instanceMvuStatusBar.value = null
      }
    }
  }
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
    } else if (varType === 'json') {
      try {
        parsed = JSON.parse(value)
      } catch {
        parsed = value  // fallback to raw string if invalid JSON
      }
    } else {
      // string 保持原样
      parsed = value
    }

    await setCharacterVariable(props.campaignId, instanceId, key, parsed)
    if (expandedInstanceId.value !== instanceId) return

    const inst = instances.value.find((item) => item.id === instanceId)
    if (inst) {
      const token = ++instanceDetailLoadToken
      await refreshInstanceVariables(inst, token)
    } else {
      instanceVariables.value = await getCharacterVariables(props.campaignId, instanceId)
      instanceMvuStatusBar.value = null
    }
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

// ─── DataTable 配置 ───
const columns = [
  { key: 'name', label: '名称' },
  { key: 'role_type', label: '类型', width: '90px' },
  { key: 'status', label: '状态', width: '90px' },
]

// ─── 暴露 refresh 给父组件 ───
defineExpose({ refresh: load })
</script>

<template>
  <LoadingState v-if="loading" />
  <div v-else-if="error" class="text-center text-err text-sm py-8">加载失败: {{ error }}</div>
  <EmptyState v-else-if="instances.length === 0" title="暂无角色实例" />

  <template v-else>
    <DataTable :columns="columns" :rows="instances" empty-title="暂无角色实例">
      <template #cell-name="{ row }">
        <button
          class="text-sm font-medium text-ink hover:text-accent transition-colors text-left"
          @click="toggleInstance(row)"
        >
          {{ row.name || row.character_name }}
          <Badge v-if="row.is_temporary" variant="warn" size="sm" class="ml-1">临时</Badge>
        </button>
      </template>
      <template #cell-role_type="{ row }">
        <span class="text-xs text-ink-soft">{{ row.role_type || '' }}</span>
      </template>
      <template #cell-status="{ row }">
        <Badge v-if="row.is_active" variant="ok" size="sm">存活</Badge>
        <Badge v-else variant="neutral" size="sm">离场</Badge>
      </template>
      <template #row-action="{ row }">
        <Button
          v-if="row.is_temporary"
          variant="default"
          size="sm"
          :disabled="promotingInstanceId === row.id"
          @click.stop="handlePromoteTemporary(row)"
        >{{ promotingInstanceId === row.id ? '升格中…' : '升格' }}</Button>
      </template>
    </DataTable>

    <!-- 展开区:变量编辑 -->
    <div v-if="expandedInstanceId" class="mt-3 bg-surface rounded-lg border border-line p-3 space-y-2">
      <div class="text-xs font-medium text-ink-soft mb-1">变量</div>
      <MvuStatusBar
        v-if="instanceMvuStatusBar"
        :ui-bindings="instanceMvuStatusBar.uiBindings"
        :variables="instanceMvuStatusBar.variables"
        :fallback-count="instanceMvuStatusBar.fallbackCount"
      />
      <div v-if="instanceVariables.length === 0" class="text-xs text-ink-faint">暂无变量</div>
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
              @change="handleVariableChange(expandedInstanceId, v.key, $event.target.checked, 'bool')"
              class="h-4 w-4 rounded border-line bg-surface-2 text-accent focus:ring-accent"
            />
            <span class="text-xs text-ink">{{ v.value ? 'true' : 'false' }}</span>
          </label>
        </template>

        <!-- int / float → number input (change 失焦保存，忠实原行为) -->
        <template v-else-if="inferVarType(v.value) === 'int' || inferVarType(v.value) === 'float'">
          <input
            type="number"
            :value="v.value"
            :step="inferVarType(v.value) === 'float' ? '0.01' : '1'"
            @change="handleVariableChange(expandedInstanceId, v.key, $event.target.value, inferVarType(v.value))"
            class="flex-1 bg-surface-2 border border-line rounded-lg px-2 py-1.5 text-xs text-ink focus:border-accent"
          />
        </template>

        <!-- json → textarea (只读折叠) -->
        <template v-else-if="inferVarType(v.value) === 'json'">
          <textarea
            :value="formatJson(v.value)"
            @change="handleVariableChange(expandedInstanceId, v.key, $event.target.value, 'json')"
            rows="3"
            class="flex-1 bg-surface-2 border border-line rounded-lg px-2 py-1.5 text-xs text-ink focus:border-accent resize-none font-mono"
          ></textarea>
        </template>

        <!-- string → text input (change 失焦保存) -->
        <template v-else>
          <input
            type="text"
            :value="String(v.value)"
            @change="handleVariableChange(expandedInstanceId, v.key, $event.target.value, 'string')"
            class="flex-1 bg-surface-2 border border-line rounded-lg px-2 py-1.5 text-xs text-ink focus:border-accent"
          />
        </template>
      </div>
    </div>
  </template>
</template>
