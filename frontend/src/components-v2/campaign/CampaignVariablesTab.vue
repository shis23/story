<script setup>
import { onMounted, ref, watch } from 'vue'
import { alertDialog } from '../../components/base/BaseDialog.js'
import {
  addCampaignVariable,
  getCampaign,
  getCampaignVariableSchema,
  getCampaignVariables,
  getCard,
  getCharacterVariables,
  listInstances,
  setCampaignVariable,
  setCharacterVariable,
  syncCampaignVariableSchema,
} from '../../tauri-api.js'
import {
  formatJsonValue,
  inferVarType,
  parseVariableInput,
  variableDisplayName,
} from '../../utils/campaignDisplay.js'
import LoadingState from '../ui/LoadingState.vue'
import Button from '../ui/Button.vue'
import { errorText } from '../../utils/errorText.js'

const props = defineProps({
  campaignId: { type: String, required: true },
})

const campaignVariables = ref([])
const instanceGroups = ref([])
const showAddGlobal = ref(false)
const addingGlobal = ref(false)
const syncingSchema = ref(false)
const syncMessage = ref('')
const globalForm = ref({
  key: '',
  label: '',
  valueType: 'string',
  defaultValue: '',
  description: '',
})
const loading = ref(false)
const error = ref(null)
let loadToken = 0

function variableLabel(scopeLabel, key) {
  return `${scopeLabel}变量 ${key}`
}

function controlValue(variable) {
  return inferVarType(variable.value) === 'json'
    ? formatJsonValue(variable.value)
    : variable.value
}

async function load() {
  if (!props.campaignId) return
  const token = ++loadToken
  loading.value = true
  error.value = null
  try {
    const [globals, globalSchema, instances] = await Promise.all([
      getCampaignVariables(props.campaignId),
      getCampaignVariableSchema(props.campaignId),
      listInstances(props.campaignId),
    ])
    const globalSchemaByKey = new Map(
      (globalSchema || []).map((field) => [field.key, field]),
    )
    let definitionsById = new Map()
    try {
      const campaign = await getCampaign(props.campaignId)
      if (campaign?.card_id) {
        const card = await getCard(campaign.card_id)
        definitionsById = new Map(
          (card?.character_definitions || []).map((definition) => [
            definition.id,
            new Map((definition.variable_schema || []).map((field) => [field.key, field])),
          ]),
        )
      }
    } catch (schemaError) {
      console.warn('读取变量中文标签失败，使用内置名称：', schemaError)
    }
    const groups = await Promise.all((instances || []).map(async (instance) => ({
      id: instance.id,
      name: instance.name || instance.character_name || '未命名角色',
      roleType: instance.role_type || '',
      variables: (await getCharacterVariables(props.campaignId, instance.id)).map((variable) => ({
        ...variable,
        schemaLabel: definitionsById.get(instance.definition_id)?.get(variable.key)?.label || '',
      })),
    })))
    if (token !== loadToken) return
    campaignVariables.value = (globals || []).map((variable) => ({
      ...variable,
      schemaLabel: globalSchemaByKey.get(variable.key)?.label || '',
      schemaDescription: globalSchemaByKey.get(variable.key)?.description || '',
    }))
    instanceGroups.value = groups
  } catch (e) {
    if (token === loadToken) error.value = errorText(e)
  } finally {
    if (token === loadToken) loading.value = false
  }
}

function parseNewGlobalValue(rawValue, valueType) {
  const raw = String(rawValue ?? '').trim()
  if (valueType === 'int') {
    if (!/^-?\d+$/.test(raw)) throw new Error('整数初始值格式不正确')
    return Number.parseInt(raw, 10)
  }
  if (valueType === 'float') {
    if (!raw) throw new Error('小数初始值不能为空')
    const value = Number(raw)
    if (!Number.isFinite(value)) throw new Error('小数初始值格式不正确')
    return value
  }
  if (valueType === 'bool') {
    if (raw === 'true') return true
    if (raw === 'false') return false
    throw new Error('布尔初始值只能填写 true 或 false')
  }
  if (valueType === 'json') {
    try {
      return JSON.parse(raw)
    } catch {
      throw new Error('JSON 初始值格式不正确')
    }
  }
  return String(rawValue ?? '')
}

async function addGlobalVariable() {
  const key = globalForm.value.key.trim()
  if (!key) {
    await alertDialog('请填写变量键名')
    return
  }
  let defaultValue
  try {
    defaultValue = parseNewGlobalValue(
      globalForm.value.defaultValue,
      globalForm.value.valueType,
    )
  } catch (e) {
    await alertDialog(String(e?.message || e))
    return
  }
  addingGlobal.value = true
  try {
    await addCampaignVariable({
      campaignId: props.campaignId,
      key,
      label: globalForm.value.label.trim() || key,
      valueType: globalForm.value.valueType,
      defaultValue,
      description: globalForm.value.description.trim() || null,
    })
    globalForm.value = {
      key: '',
      label: '',
      valueType: 'string',
      defaultValue: '',
      description: '',
    }
    showAddGlobal.value = false
    await load()
  } catch (e) {
    await alertDialog(`新增变量失败：${errorText(e)}`)
  } finally {
    addingGlobal.value = false
  }
}

async function syncCardSchema() {
  syncingSchema.value = true
  syncMessage.value = ''
  try {
    const result = await syncCampaignVariableSchema(props.campaignId)
    const added = Number(result?.added || 0)
    syncMessage.value = added > 0 ? `已补充 ${added} 项` : '已是最新'
    await load()
  } catch (e) {
    await alertDialog(`同步卡片变量失败：${errorText(e)}`)
  } finally {
    syncingSchema.value = false
  }
}

async function persist(variable, rawValue, scope, instanceId = null) {
  const type = inferVarType(variable.value)
  const parsed = parseVariableInput(rawValue, type)
  try {
    if (scope === 'campaign') {
      await setCampaignVariable(props.campaignId, variable.key, parsed)
    } else {
      await setCharacterVariable(props.campaignId, instanceId, variable.key, parsed)
    }
    variable.value = parsed
  } catch (e) {
    await alertDialog(`设置变量失败：${errorText(e)}`)
  }
}

onMounted(load)
watch(() => props.campaignId, load)

defineExpose({ refresh: load })
</script>

<template>
  <LoadingState v-if="loading" label="正在读取故事状态…" />
  <div v-else-if="error" class="rounded-xl border border-err/25 bg-err/5 px-4 py-8 text-center text-sm text-err">
    变量加载失败：{{ error }}
  </div>

  <div v-else class="space-y-6">
    <div class="flex flex-wrap items-end justify-between gap-3 border-b border-line pb-4">
      <div>
        <div class="text-[11px] tracking-[0.16em] text-accent uppercase">Story State</div>
        <h3 class="mt-1 text-lg font-semibold text-ink">故事变量</h3>
        <p class="mt-1 text-xs leading-relaxed text-ink-soft">
          集中查看整局状态与每个角色的当前值。修改后失焦即保存。
        </p>
      </div>
      <div class="text-[11px] text-ink-faint">
        {{ campaignVariables.length }} 项全局 · {{ instanceGroups.length }} 个角色
      </div>
    </div>

    <section>
      <div class="mb-2 flex flex-wrap items-center gap-2">
        <span class="h-2 w-2 rounded-full bg-accent"></span>
        <h4 class="text-sm font-semibold text-ink">全局变量</h4>
        <span class="text-[11px] text-ink-faint">影响整个活动</span>
        <span v-if="syncMessage" class="text-[11px] text-ok">{{ syncMessage }}</span>
        <div class="ml-auto flex items-center gap-1.5">
          <Button
            size="sm"
            variant="ghost"
            aria-label="同步卡片全局变量"
            :loading="syncingSchema"
            @click="syncCardSchema"
          >
            同步卡片
          </Button>
          <Button
            size="sm"
            variant="primary"
            aria-label="新增全局变量"
            @click="showAddGlobal = !showAddGlobal"
          >
            {{ showAddGlobal ? '取消' : '新增变量' }}
          </Button>
        </div>
      </div>
      <div
        v-if="showAddGlobal"
        class="mb-3 rounded-xl border border-accent-border bg-accent-soft/20 p-3"
      >
        <div class="grid gap-2 sm:grid-cols-2">
          <label class="space-y-1">
            <span class="text-[11px] text-ink-soft">键名</span>
            <input
              v-model="globalForm.key"
              aria-label="变量键名"
              placeholder="例如 faction_tension"
              class="h-9 w-full rounded-lg border border-line bg-surface px-3 font-mono text-xs text-ink outline-none focus:border-accent"
            />
          </label>
          <label class="space-y-1">
            <span class="text-[11px] text-ink-soft">中文名称</span>
            <input
              v-model="globalForm.label"
              aria-label="变量中文名"
              placeholder="例如 阵营紧张度"
              class="h-9 w-full rounded-lg border border-line bg-surface px-3 text-sm text-ink outline-none focus:border-accent"
            />
          </label>
          <label class="space-y-1">
            <span class="text-[11px] text-ink-soft">类型</span>
            <select
              v-model="globalForm.valueType"
              aria-label="变量类型"
              class="h-9 w-full rounded-lg border border-line bg-surface px-3 text-sm text-ink outline-none focus:border-accent"
            >
              <option value="string">文本</option>
              <option value="int">整数</option>
              <option value="float">小数</option>
              <option value="bool">布尔</option>
              <option value="json">JSON</option>
            </select>
          </label>
          <label class="space-y-1">
            <span class="text-[11px] text-ink-soft">初始值</span>
            <input
              v-model="globalForm.defaultValue"
              aria-label="变量初始值"
              :placeholder="globalForm.valueType === 'bool' ? 'true 或 false' : '变量初始值'"
              class="h-9 w-full rounded-lg border border-line bg-surface px-3 font-mono text-xs text-ink outline-none focus:border-accent"
            />
          </label>
        </div>
        <label class="mt-2 block space-y-1">
          <span class="text-[11px] text-ink-soft">说明（会帮助后处理正确判断何时更新）</span>
          <input
            v-model="globalForm.description"
            aria-label="变量说明"
            placeholder="例如：数值越高，阵营越可能爆发公开冲突"
            class="h-9 w-full rounded-lg border border-line bg-surface px-3 text-sm text-ink outline-none focus:border-accent"
          />
        </label>
        <div class="mt-3 flex justify-end">
          <Button
            size="sm"
            variant="primary"
            aria-label="保存全局变量"
            :loading="addingGlobal"
            @click="addGlobalVariable"
          >
            保存变量
          </Button>
        </div>
      </div>
      <div
        v-if="campaignVariables.length === 0"
        class="rounded-xl border border-dashed border-line px-4 py-7 text-center text-xs text-ink-faint"
      >
        暂无全局变量
      </div>
      <div v-else class="overflow-hidden rounded-xl border border-line bg-surface shadow-card divide-y divide-line">
        <div
          v-for="variable in campaignVariables"
          :key="variable.key"
          class="grid gap-2 px-4 py-3 sm:grid-cols-[minmax(7rem,0.42fr)_minmax(0,1fr)] sm:items-center"
        >
          <div class="min-w-0">
            <div class="truncate text-sm font-medium text-ink" :title="variableDisplayName(variable.key, variable.schemaLabel)">
              {{ variableDisplayName(variable.key, variable.schemaLabel) }}
            </div>
            <div class="mt-0.5 truncate font-mono text-[10px] text-ink-faint" :title="variable.key">
              {{ variable.key }} · {{ inferVarType(variable.value) }}
            </div>
            <div
              v-if="variable.schemaDescription"
              class="mt-1 text-[11px] leading-relaxed text-ink-soft"
            >
              {{ variable.schemaDescription }}
            </div>
          </div>

          <label v-if="inferVarType(variable.value) === 'bool'" class="inline-flex items-center gap-2 text-xs text-ink">
            <input
              type="checkbox"
              class="h-4 w-4 rounded border-line bg-surface-2 text-accent focus:ring-accent"
              :aria-label="variableLabel('全局', variable.key)"
              :checked="variable.value === true"
              @change="persist(variable, $event.target.checked, 'campaign')"
            />
            {{ variable.value ? 'true' : 'false' }}
          </label>
          <textarea
            v-else-if="inferVarType(variable.value) === 'json'"
            :aria-label="variableLabel('全局', variable.key)"
            :value="controlValue(variable)"
            rows="3"
            class="w-full resize-y rounded-lg border border-line bg-bg px-3 py-2 font-mono text-xs leading-relaxed text-ink outline-none transition-colors focus:border-accent"
            @change="persist(variable, $event.target.value, 'campaign')"
          ></textarea>
          <input
            v-else
            :type="inferVarType(variable.value) === 'string' ? 'text' : 'number'"
            :step="inferVarType(variable.value) === 'float' ? '0.01' : '1'"
            :aria-label="variableLabel('全局', variable.key)"
            :value="controlValue(variable)"
            class="h-9 w-full rounded-lg border border-line bg-bg px-3 text-sm text-ink outline-none transition-colors focus:border-accent"
            @change="persist(variable, $event.target.value, 'campaign')"
          />
        </div>
      </div>
    </section>

    <section>
      <div class="mb-2 flex items-center gap-2">
        <span class="h-2 w-2 rounded-full border-2 border-accent"></span>
        <h4 class="text-sm font-semibold text-ink">角色变量</h4>
        <span class="text-[11px] text-ink-faint">按角色分组</span>
      </div>
      <div
        v-if="instanceGroups.length === 0"
        class="rounded-xl border border-dashed border-line px-4 py-7 text-center text-xs text-ink-faint"
      >
        当前活动还没有角色实例
      </div>
      <div v-else class="space-y-3">
        <details
          v-for="(group, index) in instanceGroups"
          :key="group.id"
          :open="index === 0"
          class="group overflow-hidden rounded-xl border border-line bg-surface shadow-card"
        >
          <summary class="flex cursor-pointer list-none items-center gap-3 px-4 py-3 hover:bg-surface-2/50">
            <span class="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-accent-border bg-accent-soft text-sm font-semibold text-accent">
              {{ group.name.slice(0, 1) }}
            </span>
            <span class="min-w-0 flex-1">
              <span class="block truncate text-sm font-medium text-ink">{{ group.name }}</span>
              <span class="block text-[10px] text-ink-faint">{{ group.roleType || '角色' }}</span>
            </span>
            <span class="text-[11px] text-ink-faint">{{ group.variables.length }} 项</span>
            <svg class="text-ink-faint transition-transform group-open:rotate-180" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M6 9l6 6 6-6"/></svg>
          </summary>

          <div v-if="group.variables.length === 0" class="border-t border-line px-4 py-5 text-center text-xs text-ink-faint">
            暂无角色变量
          </div>
          <div v-else class="border-t border-line divide-y divide-line">
            <div
              v-for="variable in group.variables"
              :key="variable.key"
              class="grid gap-2 px-4 py-3 sm:grid-cols-[minmax(7rem,0.42fr)_minmax(0,1fr)] sm:items-center"
            >
              <div class="min-w-0">
                <div class="truncate text-sm font-medium text-ink" :title="variableDisplayName(variable.key, variable.schemaLabel)">
                  {{ variableDisplayName(variable.key, variable.schemaLabel) }}
                </div>
                <div class="mt-0.5 truncate font-mono text-[10px] text-ink-faint" :title="variable.key">
                  {{ variable.key }} · {{ inferVarType(variable.value) }}
                </div>
              </div>

              <label v-if="inferVarType(variable.value) === 'bool'" class="inline-flex items-center gap-2 text-xs text-ink">
                <input
                  type="checkbox"
                  class="h-4 w-4 rounded border-line bg-surface-2 text-accent focus:ring-accent"
                  :aria-label="variableLabel(`${group.name} `, variable.key)"
                  :checked="variable.value === true"
                  @change="persist(variable, $event.target.checked, 'instance', group.id)"
                />
                {{ variable.value ? 'true' : 'false' }}
              </label>
              <textarea
                v-else-if="inferVarType(variable.value) === 'json'"
                :aria-label="variableLabel(`${group.name} `, variable.key)"
                :value="controlValue(variable)"
                rows="3"
                class="w-full resize-y rounded-lg border border-line bg-bg px-3 py-2 font-mono text-xs leading-relaxed text-ink outline-none transition-colors focus:border-accent"
                @change="persist(variable, $event.target.value, 'instance', group.id)"
              ></textarea>
              <input
                v-else
                :type="inferVarType(variable.value) === 'string' ? 'text' : 'number'"
                :step="inferVarType(variable.value) === 'float' ? '0.01' : '1'"
                :aria-label="variableLabel(`${group.name} `, variable.key)"
                :value="controlValue(variable)"
                class="h-9 w-full rounded-lg border border-line bg-bg px-3 text-sm text-ink outline-none transition-colors focus:border-accent"
                @change="persist(variable, $event.target.value, 'instance', group.id)"
              />
            </div>
          </div>
        </details>
      </div>
    </section>
  </div>
</template>
