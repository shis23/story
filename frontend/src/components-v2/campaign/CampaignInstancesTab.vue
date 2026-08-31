<script setup>
import { computed, ref, onMounted, watch } from 'vue'
import { confirmDialog, alertDialog } from '../../components/base/BaseDialog.js'
import MvuStatusBar from '../st/MvuStatusBar.vue'
import {
  addCampaignInstance, listInstances, getCharacterVariables, setCharacterVariable,
  promoteTemporaryInstance, getCampaign, getCard, metaGetMvuTranslation,
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
import Input from '../ui/Input.vue'
import LoadingState from '../ui/LoadingState.vue'
import Overlay from '../ui/Overlay.vue'
import Textarea from '../ui/Textarea.vue'
import { errorText } from '../../utils/errorText.js'

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
const showAddCharacter = ref(false)
const addMode = ref('card')
const addOptionsLoading = ref(false)
const addingCharacter = ref(false)
const campaignCard = ref(null)
const selectedDefinitionId = ref('')
const customName = ref('')
const customPersona = ref('')
const customBehavior = ref('')
let instanceDetailLoadToken = 0

const availableDefinitions = computed(() => {
  const joinedDefinitionIds = new Set(
    instances.value
      .map((instance) => instance.definition_id)
      .filter(Boolean),
  )
  return (campaignCard.value?.character_definitions || [])
    .filter((definition) => !joinedDefinitionIds.has(definition.id))
})

function roleLabel(roleType, temporary = false) {
  if (temporary) return '临时角色'
  const labels = {
    protagonist: '主角',
    supporting: '常驻配角',
    extra: '临场角色',
  }
  return labels[String(roleType || '').toLowerCase()] || '未分类'
}

function roleVariant(roleType, temporary = false) {
  if (temporary) return 'warn'
  if (String(roleType || '').toLowerCase() === 'protagonist') return 'accent'
  if (String(roleType || '').toLowerCase() === 'supporting') return 'ok'
  return 'neutral'
}

// ─── 加载 ───
async function load() {
  if (!props.campaignId) return
  loading.value = true
  error.value = null
  try {
    instances.value = await listInstances(props.campaignId)
  } catch (e) {
    error.value = errorText(e)
  } finally {
    loading.value = false
  }
}

onMounted(load)

watch(() => props.campaignId, () => {
  showAddCharacter.value = false
  expandedInstanceId.value = null
  instanceVariables.value = []
  instanceMvuStatusBar.value = null
  instanceDetailLoadToken += 1
  if (props.campaignId) load()
})

function resetCustomCharacter() {
  customName.value = ''
  customPersona.value = ''
  customBehavior.value = ''
}

async function openAddCharacter() {
  showAddCharacter.value = true
  addOptionsLoading.value = true
  campaignCard.value = null
  selectedDefinitionId.value = ''
  resetCustomCharacter()
  try {
    const campaign = await getCampaign(props.campaignId)
    if (!campaign?.card_id) throw new Error('当前活动没有关联角色卡')
    campaignCard.value = await getCard(campaign.card_id)
    selectedDefinitionId.value = availableDefinitions.value[0]?.id || ''
    addMode.value = availableDefinitions.value.length > 0 ? 'card' : 'custom'
  } catch (e) {
    showAddCharacter.value = false
    await alertDialog('加载可添加角色失败: ' + errorText(e))
  } finally {
    addOptionsLoading.value = false
  }
}

function closeAddCharacter() {
  if (addingCharacter.value) return
  showAddCharacter.value = false
}

async function finishAddingCharacter(payload) {
  addingCharacter.value = true
  try {
    await addCampaignInstance(payload)
    showAddCharacter.value = false
    await load()
    emit('refresh')
  } catch (e) {
    await alertDialog('添加角色失败: ' + errorText(e))
  } finally {
    addingCharacter.value = false
  }
}

async function addSelectedDefinition() {
  if (!selectedDefinitionId.value) {
    await alertDialog('请选择要加入本局的角色')
    return
  }
  await finishAddingCharacter({
    campaignId: props.campaignId,
    definitionId: selectedDefinitionId.value,
    name: null,
    persona: null,
    behavior: null,
  })
}

async function addCustomCharacter() {
  const name = customName.value.trim()
  if (!name) {
    await alertDialog('请填写临时角色名称')
    return
  }
  await finishAddingCharacter({
    campaignId: props.campaignId,
    definitionId: null,
    name,
    persona: customPersona.value.trim() || null,
    behavior: customBehavior.value.trim() || null,
  })
}

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
    await alertDialog('设置变量失败: ' + errorText(e))
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
    await alertDialog('升格失败: ' + errorText(e))
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
  <div class="mb-3 flex items-center justify-between gap-3">
    <div>
      <p class="text-xs font-medium text-ink-soft">本局角色</p>
      <p class="mt-0.5 text-[11px] text-ink-faint">
        {{ instances.length }} 位已加入，续写时点名即可安排登场
      </p>
    </div>
    <Button
      variant="primary"
      size="sm"
      aria-label="添加角色"
      @click="openAddCharacter"
    >
      <span aria-hidden="true">＋</span>
      添加角色
    </Button>
  </div>

  <LoadingState v-if="loading" />
  <div v-else-if="error" class="text-center text-err text-sm py-8">加载失败: {{ error }}</div>
  <EmptyState
    v-else-if="instances.length === 0"
    title="暂无角色实例"
    description="从角色卡加入已有角色，或新建一个只属于本局的临时角色。"
  />

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
        <Badge :variant="roleVariant(row.role_type, row.is_temporary)" size="sm">
          {{ roleLabel(row.role_type, row.is_temporary) }}
        </Badge>
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
        :mvu-state="instanceMvuStatusBar"
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

  <Overlay
    :show="showAddCharacter"
    side="center"
    title="添加角色"
    panel-width-class="w-full max-w-[560px]"
    @update:show="($event) ? null : closeAddCharacter()"
  >
    <div class="p-5">
      <div class="grid grid-cols-2 rounded-lg border border-line bg-surface-2 p-1">
        <button
          type="button"
          aria-label="切换到从角色卡加入"
          class="rounded-md px-3 py-2 text-sm transition-colors"
          :class="addMode === 'card'
            ? 'bg-surface text-ink font-medium shadow-card'
            : 'text-ink-soft hover:text-ink'"
          @click="addMode = 'card'"
        >
          从角色卡加入
        </button>
        <button
          type="button"
          aria-label="切换到新建临时角色"
          class="rounded-md px-3 py-2 text-sm transition-colors"
          :class="addMode === 'custom'
            ? 'bg-surface text-ink font-medium shadow-card'
            : 'text-ink-soft hover:text-ink'"
          @click="addMode = 'custom'"
        >
          新建临时角色
        </button>
      </div>

      <LoadingState v-if="addOptionsLoading" class="py-8" />

      <div v-else-if="addMode === 'card'" class="mt-5">
        <div v-if="availableDefinitions.length" class="space-y-2">
          <p class="text-xs leading-relaxed text-ink-soft">
            选择角色卡中尚未进入本局的角色。加入后会继承完整人设、行为规则与变量。
          </p>
          <div class="max-h-[340px] space-y-2 overflow-y-auto pr-1 sf-drawer-scroll">
            <label
              v-for="definition in availableDefinitions"
              :key="definition.id"
              class="flex cursor-pointer items-start gap-3 rounded-lg border p-3 transition-colors"
              :class="selectedDefinitionId === definition.id
                ? 'border-accent-border bg-accent-soft/45'
                : 'border-line bg-surface hover:border-accent-border/60'"
            >
              <input
                v-model="selectedDefinitionId"
                type="radio"
                name="campaign-character-definition"
                :value="definition.id"
                class="mt-1 h-4 w-4 shrink-0 accent-[var(--color-accent)]"
              />
              <span class="min-w-0 flex-1">
                <span class="flex flex-wrap items-center gap-2">
                  <span class="text-sm font-medium text-ink">{{ definition.name }}</span>
                  <Badge :variant="roleVariant(definition.role_type)" size="sm">
                    {{ roleLabel(definition.role_type) }}
                  </Badge>
                  <span v-if="definition.group" class="text-[11px] text-ink-faint">
                    {{ definition.group }}
                  </span>
                </span>
                <span
                  v-if="definition.persona_prompt"
                  class="mt-1 block line-clamp-2 text-xs leading-relaxed text-ink-soft"
                >
                  {{ definition.persona_prompt }}
                </span>
              </span>
            </label>
          </div>
        </div>
        <EmptyState
          v-else
          title="卡内角色均已加入"
          description="仍可新建一个只属于当前活动的临时角色。"
        >
          <template #action>
            <Button
              variant="default"
              size="sm"
              @click="addMode = 'custom'"
            >
              新建临时角色
            </Button>
          </template>
        </EmptyState>
      </div>

      <form v-else class="mt-5 space-y-4" @submit.prevent="addCustomCharacter">
        <div class="rounded-lg border border-line bg-surface-2/55 px-3 py-2.5 text-xs leading-relaxed text-ink-soft">
          临时角色会立即进入本局角色表，拥有基础变量；确认长期保留后可在实例列表中升格。
        </div>
        <label class="block">
          <span class="mb-1.5 block text-xs font-medium text-ink-soft">
            名称 <span class="text-err">*</span>
          </span>
          <Input
            v-model="customName"
            aria-label="临时角色名称"
            placeholder="例如：渡鸦信使"
            :disabled="addingCharacter"
          />
        </label>
        <label class="block">
          <span class="mb-1.5 block text-xs font-medium text-ink-soft">简要人设</span>
          <Textarea
            v-model="customPersona"
            aria-label="临时角色人设"
            placeholder="外貌、性格、说话方式，以及当前身份……"
            :rows="3"
            :disabled="addingCharacter"
          />
        </label>
        <label class="block">
          <span class="mb-1.5 block text-xs font-medium text-ink-soft">行为规则</span>
          <Textarea
            v-model="customBehavior"
            aria-label="临时角色行为规则"
            placeholder="它会做什么，不会做什么，以及眼下的行动倾向……"
            :rows="3"
            :disabled="addingCharacter"
          />
        </label>
      </form>

      <div class="mt-5 flex items-center justify-end gap-2 border-t border-line pt-4">
        <Button variant="ghost" :disabled="addingCharacter" @click="closeAddCharacter">
          取消
        </Button>
        <Button
          v-if="addMode === 'card'"
          variant="primary"
          :loading="addingCharacter"
          :disabled="addOptionsLoading || !selectedDefinitionId"
          aria-label="将所选卡内角色加入本局"
          @click="addSelectedDefinition"
        >
          加入本局
        </Button>
        <Button
          v-else
          variant="primary"
          :loading="addingCharacter"
          :disabled="!customName.trim()"
          aria-label="创建临时角色"
          @click="addCustomCharacter"
        >
          创建临时角色
        </Button>
      </div>
    </div>
  </Overlay>
</template>
