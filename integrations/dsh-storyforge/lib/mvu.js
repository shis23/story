/**
 * MVU 原生状态面板与交互按钮的纯逻辑（宿主半边算好，浏览器半边只画）。
 *
 * 前半部分的逻辑逐字移植桌面前端（注释从简；`test/mvu.test.mjs` 用同一组样例和桌面实现对拍）：
 * - `frontend/src/utils/mvuKey.js`：变量键记法归一、模板段展开、跨记法查找；
 * - `frontend/src/utils/mvuStatusBarModel.js`：取值、进度条百分比、图标映射；
 * - `frontend/src/utils/campaignMvuStatusBar.js`：每个绑定卡角色定义的实例一节；
 * - `frontend/src/utils/mvuInteractions.js`：交互动作扁平化、value_expr 保守解释、执行计划；
 * - `frontend/src/utils/shellVariableOutbox.js`：写入作用域（`instance:<id>:<key>` 前缀）与
 *   `__storyforge*` 内部命名空间拦截。
 * 后半部分是 DSH 专用的装配：面板数据、交互的写入目标。
 */

//#region mvuKey.js
export function normalizeMvuKey(raw) {
  let s = String(raw ?? '').trim()
  // 斜杠记法 → 点记法：仅当以 / 或 stat_data/ 开头（避免误伤含 / 的普通名字）
  if (s.startsWith('/') || s.startsWith('stat_data/')) {
    s = s.replace(/^\/+/, '').replaceAll('/', '.')
  }
  while (s.startsWith('stat_data.')) s = s.slice('stat_data.'.length)
  return s
    .split('.')
    .map((seg) => seg.trim())
    .filter(Boolean)
    .map((seg) =>
      seg.length >= 2 && seg.startsWith('<') && seg.endsWith('>') ? `{${seg.slice(1, -1)}}` : seg,
    )
    .join('.')
}

export function expandMvuTemplateKey(key, instanceName) {
  const name = String(instanceName ?? '').trim()
  const raw = String(key ?? '')
  if (!name || !raw.includes('{')) return raw
  return raw
    .split('.')
    .map((seg) => (/^\{.+\}$/.test(seg.trim()) ? name : seg))
    .join('.')
}

export function findMvuVariableForInstance(variables, key, instanceName) {
  const expanded = expandMvuTemplateKey(normalizeMvuKey(key), instanceName)
  if (expanded && expanded !== String(key ?? '')) {
    const hit = findMvuVariable(variables, expanded)
    if (hit) return hit
  }
  return findMvuVariable(variables, key)
}

export function findMvuVariable(variables, key) {
  const list = Array.isArray(variables) ? variables : []
  const exact = list.find((v) => v?.key === key)
  if (exact) return exact
  const norm = normalizeMvuKey(key)
  if (!norm) return null
  return list.find((v) => typeof v?.key === 'string' && normalizeMvuKey(v.key) === norm) || null
}
//#endregion

//#region mvuStatusBarModel.js
export function getMvuValue(variables, key, instanceName = '') {
  const found = findMvuVariableForInstance(variables, key, instanceName)
  return found ? found.value : null
}

export function toMvuNumber(value) {
  if (value == null) return 0
  const number = Number(value)
  return Number.isNaN(number) ? 0 : number
}

export function mvuBarPercent(binding, variables, instanceName = '') {
  const value = toMvuNumber(getMvuValue(variables, binding?.variable_key, instanceName))
  const max = binding?.display?.max ?? 100
  if (max <= 0) return 0
  return Math.max(0, Math.min(100, (value / max) * 100))
}

export function mvuIconFor(binding, variables, fallbackIcon = '-', instanceName = '') {
  const value = getMvuValue(variables, binding?.variable_key, instanceName)
  const mapping = binding?.display?.mapping || {}
  const key = String(value)
  return mapping[key] || mapping._default || fallbackIcon
}

export function mvuDisplayValue(variables, key, fallback = '—', instanceName = '') {
  return getMvuValue(variables, key, instanceName) ?? fallback
}
//#endregion

//#region campaignMvuStatusBar.js
export function findInstanceDefinition(card, instance) {
  const definitionId = instance?.definition_id
  if (!definitionId || !Array.isArray(card?.character_definitions)) return null
  return card.character_definitions.find((definition) => definition?.id === definitionId) || null
}

export function buildInstanceMvuStatusBarProps({ instance, card, translationDetail, variables }) {
  const definition = findInstanceDefinition(card, instance)
  const translation = translationDetail?.translation
  if (!definition || !translation) return null

  const uiBindings = Array.isArray(translation.ui_bindings) ? translation.ui_bindings : []
  const fallbackFragments = Array.isArray(translation.fallback_fragments)
    ? translation.fallback_fragments
    : []

  if (uiBindings.length === 0 && fallbackFragments.length === 0) return null

  return {
    definitionId: definition.id,
    sourceCharacterId: card?.source_character_id || translationDetail?.source_character_id || null,
    instanceName: instance?.name || '',
    uiBindings,
    variables: Array.isArray(variables) ? variables : [],
    fallbackCount: fallbackFragments.length,
  }
}

export function mergeVariablesForMvuStatus(campaignVariables, instanceVariables) {
  const merged = new Map()
  for (const v of Array.isArray(campaignVariables) ? campaignVariables : []) {
    if (v && typeof v.key === 'string') merged.set(v.key, v.value)
  }
  for (const v of Array.isArray(instanceVariables) ? instanceVariables : []) {
    if (v && typeof v.key === 'string') merged.set(v.key, v.value)
  }
  return Array.from(merged, ([key, value]) => ({ key, value }))
}

export function buildCampaignMvuStatusSections({ card, translationDetail, instances, campaignVariables }) {
  if (!Array.isArray(instances)) return []
  const sections = []
  for (const instance of instances) {
    const mvuState = buildInstanceMvuStatusBarProps({
      instance,
      card,
      translationDetail,
      variables: mergeVariablesForMvuStatus(campaignVariables, instance?.variables),
    })
    if (mvuState) {
      sections.push({
        instanceId: instance.id,
        instanceName: instance.name || '',
        mvuState,
      })
    }
  }
  return sections
}
//#endregion

//#region mvuInteractions.js
const MAX_FLATTEN_DEPTH = 5
const MAX_MVU_EXPR_BYTES = 64 * 1024
const MAX_MVU_EXPR_DEPTH = 32

function validateMvuJsonLiteral(raw) {
  if (raw.length > MAX_MVU_EXPR_BYTES) {
    return { ok: false, reason: `表达式过长（${raw.length} 字节，上限 ${MAX_MVU_EXPR_BYTES}）` }
  }
  return { ok: true }
}

function jsonDepth(value, depth = 0) {
  if (value === null || typeof value !== 'object') return depth
  if (Array.isArray(value)) {
    return value.reduce((max, item) => Math.max(max, jsonDepth(item, depth + 1)), depth + 1)
  }
  return Object.values(value).reduce(
    (max, item) => Math.max(max, jsonDepth(item, depth + 1)),
    depth + 1,
  )
}

export function flattenInteractionActions(actions, depth = 0) {
  if (!Array.isArray(actions) || depth >= MAX_FLATTEN_DEPTH) return []
  const flat = []
  for (const action of actions) {
    if (!action || typeof action !== 'object') continue
    if (action.kind === 'multi') {
      flat.push(...flattenInteractionActions(action.actions, depth + 1))
    } else {
      flat.push(action)
    }
  }
  return flat
}

const JS_EXPR_MARKERS = /[=(){}[\];`＝（）｛｝［］；]|getvar|setvar|Math\.|\$\{|=>|await|fetch/

const ARITHMETIC_MIX = /[+*/%＋－×÷＊／％−±]|\s-|-\s|\d-|-\d/

export function evaluateMvuValueExpr(valueExpr, currentValue, key) {
  const raw = String(valueExpr ?? '').trim()
  if (!raw) return { ok: false, reason: 'empty expression' }

  const current = typeof currentValue === 'number' ? currentValue : Number(currentValue)
  const hasNumericCurrent = currentValue !== null && currentValue !== undefined && Number.isFinite(current)

  const deltaMatch = raw.match(/^([+-])\s*(\d+(?:\.\d+)?)$/)
  if (deltaMatch) {
    const delta = Number(deltaMatch[2]) * (deltaMatch[1] === '-' ? -1 : 1)
    if (deltaMatch[1] === '+' || hasNumericCurrent) {
      return { ok: true, value: (hasNumericCurrent ? current : 0) + delta }
    }
    return { ok: true, value: delta }
  }

  const keyedDeltaMatch = raw.match(/^([\p{L}\p{N}_.]+)\s*([+-])\s*(\d+(?:\.\d+)?)$/u)
  if (keyedDeltaMatch && !/^\d/.test(keyedDeltaMatch[1])) {
    const identMatchesKey =
      key != null &&
      (keyedDeltaMatch[1] === key || normalizeMvuKey(keyedDeltaMatch[1]) === normalizeMvuKey(key))
    if (identMatchesKey) {
      const delta = Number(keyedDeltaMatch[3]) * (keyedDeltaMatch[2] === '-' ? -1 : 1)
      return { ok: true, value: (hasNumericCurrent ? current : 0) + delta }
    }
    return {
      ok: false,
      reason: `表达式引用变量 ${keyedDeltaMatch[1]}，与目标 key 不符，无法原生解释`,
    }
  }

  try {
    const sizeCheck = validateMvuJsonLiteral(raw)
    if (!sizeCheck.ok) return sizeCheck
    const parsed = JSON.parse(raw)
    if (jsonDepth(parsed) > MAX_MVU_EXPR_DEPTH) {
      return { ok: false, reason: `JSON 字面量嵌套过深（上限 ${MAX_MVU_EXPR_DEPTH} 层）` }
    }
    return { ok: true, value: parsed }
  } catch {
    // 不是 JSON，继续
  }

  if (!JS_EXPR_MARKERS.test(raw) && !ARITHMETIC_MIX.test(raw) && raw.length <= 100) {
    return { ok: true, value: raw }
  }
  return { ok: false, reason: '表达式无法原生解释（需 WebView/后处理）' }
}

export function planMvuInteraction(mapping, { variables, instanceName } = {}) {
  const vars = Array.isArray(variables) ? variables : []
  const plan = {
    label: mapping?.element_label || '',
    writes: [],
    hints: [],
    skipped: [],
  }
  for (const action of flattenInteractionActions(mapping?.actions)) {
    if (action.kind === 'modify_variable') {
      const targetVar = findMvuVariableForInstance(vars, action.key, instanceName)
      const result = evaluateMvuValueExpr(action.value_expr, targetVar?.value, action.key)
      if (result.ok) {
        const expandedKey = expandMvuTemplateKey(normalizeMvuKey(action.key), instanceName)
        const writeKey = targetVar?.key || expandedKey || action.key
        plan.writes.push({ key: writeKey, value: result.value })
      } else {
        plan.skipped.push({ kind: action.kind, key: action.key, reason: result.reason })
      }
    } else if (action.kind === 'trigger_next_turn') {
      const hint = String(action.hint || '').trim()
      if (hint) plan.hints.push(hint)
    } else if (action.kind === 'run_original_js') {
      plan.skipped.push({
        kind: action.kind,
        reason: '原始 JS 留桩不执行',
        description: action.description || '',
      })
    } else {
      plan.skipped.push({ kind: String(action.kind || 'unknown'), reason: '未知动作类型' })
    }
  }
  return plan
}
//#endregion

//#region shellVariableOutbox.js（只取作用域解析，实际写入走 sidecar）
export function isReservedNamespace(key) {
  return typeof key === 'string' && key.startsWith('__storyforge')
}

export function toJsonValue(value) {
  if (value === undefined) return null
  if (value === null) return null
  if (typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') {
    return value
  }
  try {
    return JSON.parse(JSON.stringify(value))
  } catch {
    return String(value)
  }
}

/**
 * `persistShellVariableWrite` 的写入前半段：键前缀 → 作用域与目标实例、空键与内部命名空间
 * 拦截。成功时返回 `{ ok: true, scope, instanceId, key, value }`，交给 sidecar 写；失败时
 * 返回与桌面同形的 `{ ok: false, scope, key, error }`。
 */
export function resolveVariableWrite({ instanceId, key, value }) {
  const k = String(key || '').trim()
  if (!k) return { ok: false, scope: 'none', key: '', error: 'empty key' }
  if (isReservedNamespace(k)) {
    return { ok: false, scope: 'none', key: k, error: `reserved namespace key rejected: ${k}` }
  }

  let scope = 'campaign'
  let writeKey = k
  let targetInstance = instanceId || null

  if (k.startsWith('instance:') || k.startsWith('inst:')) {
    const parts = k.split(':')
    if (parts.length >= 3) {
      targetInstance = parts[1]
      writeKey = parts.slice(2).join(':')
      scope = 'instance'
    } else if (parts.length === 2 && targetInstance) {
      writeKey = parts[1]
      scope = 'instance'
    }
  } else if (k.startsWith('campaign:')) {
    writeKey = k.slice('campaign:'.length)
    scope = 'campaign'
  }

  const trimmedWriteKey = writeKey.trim()
  if (!trimmedWriteKey) return { ok: false, scope, key: '', error: 'empty key' }
  writeKey = trimmedWriteKey

  if (isReservedNamespace(writeKey)) {
    return { ok: false, scope, key: writeKey, error: `reserved namespace key rejected: ${writeKey}` }
  }
  if (scope === 'instance' && !targetInstance) {
    return { ok: false, scope, key: writeKey, error: 'instance scope requires instanceId' }
  }
  return {
    ok: true,
    scope,
    instanceId: scope === 'instance' ? targetInstance : null,
    key: writeKey,
    value: toJsonValue(value),
  }
}
//#endregion

//#region DSH 装配
/**
 * sidecar `mvu.status` 的结果 → 桌面口径的状态分节（`buildCampaignMvuStatusSections` 的输入形状）。
 * @returns {Array<{instanceId, instanceName, mvuState}>}
 */
export function statusSections(status) {
  if (!status?.translation) return []
  return buildCampaignMvuStatusSections({
    card: { character_definitions: status.definitions ?? [], source_character_id: status.source_character_id },
    translationDetail: { source_character_id: status.source_character_id, translation: status.translation },
    instances: status.instances ?? [],
    campaignVariables: status.campaign_variables ?? [],
  })
}

/** 进度条分级（桌面 `mvuBarColor` 的 >50 / >25 两档，换成主题无关的色调名）。 */
export function barTone(percent) {
  if (percent > 50) return 'ok'
  if (percent > 25) return 'warn'
  return 'err'
}

function displayText(value) {
  if (value == null) return '—'
  if (typeof value === 'object') {
    try {
      return JSON.stringify(value)
    } catch {
      return String(value)
    }
  }
  return String(value)
}

/** 一节里每个绑定 → 浏览器直接画的一行（与桌面 `MvuStatusBar.vue` 的四种画法一一对应）。 */
export function sectionRows(mvuState) {
  const variables = mvuState?.variables ?? []
  const instanceName = mvuState?.instanceName ?? ''
  return (mvuState?.uiBindings ?? []).map((binding) => {
    const kind = binding?.display?.kind
    const row = { element: String(binding?.element ?? ''), label: String(binding?.variable_key ?? ''), kind: kind ?? 'text' }
    if (kind === 'bar') {
      const percent = mvuBarPercent(binding, variables, instanceName)
      return {
        ...row,
        value: toMvuNumber(getMvuValue(variables, binding.variable_key, instanceName)),
        max: binding.display?.max ?? 100,
        percent,
        tone: barTone(percent),
      }
    }
    if (kind === 'icon') return { ...row, text: mvuIconFor(binding, variables, '·', instanceName) }
    return { ...row, kind: kind === 'tag' ? 'tag' : 'text', text: displayText(mvuDisplayValue(variables, binding?.variable_key, '—', instanceName)) }
  })
}

/**
 * 交互的写入目标（桌面 `dispatchMvuInteraction` 同判据）：卡绑定实例恰好一个 → 该实例；
 * 否则整局只有一个实例 → 它；都不是 → Campaign 作用域。模板键按目标实例名展开。
 */
export function interactionTarget(status, sections) {
  const instances = Array.isArray(status?.instances) ? status.instances : []
  const singleInstanceId =
    (sections.length === 1 ? sections[0].instanceId : null) || (instances.length === 1 ? instances[0].id : null)
  const singleInstanceName =
    (sections.length === 1 ? sections[0].instanceName : '') ||
    (singleInstanceId ? instances.find((instance) => instance.id === singleInstanceId)?.name || '' : '')
  return { instanceId: singleInstanceId, instanceName: singleInstanceName }
}

/**
 * 交互 → 执行计划 + 交给 sidecar 的写入（已解析作用域）与被拦下的写入。
 * 变量取第一节的合并变量（桌面同口径）。
 */
export function planInteraction(status, mapping) {
  const sections = statusSections(status)
  const variables = sections[0]?.mvuState?.variables || []
  const target = interactionTarget(status, sections)
  const plan = planMvuInteraction(mapping, { variables, instanceName: target.instanceName })
  const writes = []
  const rejected = []
  for (const write of plan.writes) {
    const key = target.instanceId ? `instance:${target.instanceId}:${write.key}` : write.key
    const resolved = resolveVariableWrite({ instanceId: target.instanceId, key, value: write.value })
    if (resolved.ok) writes.push(resolved)
    else rejected.push(resolved)
  }
  return { plan, writes, rejected }
}

/** 面板数据：状态分节（已算好每行）、交互按钮、待应用的变量定义。 */
export function panelModel(status) {
  if (!status) return { state: 'none' }
  const translation = status.translation
  const base = {
    cardName: status.card_name ?? '',
    analyzedAt: status.analyzed_at ?? null,
    pendingApply: status.pending_apply ?? [],
  }
  if (!translation) return { ...base, state: 'unanalyzed' }
  const sections = statusSections(status).map((section) => ({
    instanceId: section.instanceId,
    instanceName: section.instanceName,
    fallbackCount: section.mvuState.fallbackCount,
    rows: sectionRows(section.mvuState),
  }))
  const interactions = (Array.isArray(translation.interactions) ? translation.interactions : []).map((mapping, index) => ({
    index,
    label: String(mapping?.element_label ?? ''),
  }))
  return {
    ...base,
    state: 'ready',
    sections,
    interactions,
    bindingCount: Array.isArray(translation.ui_bindings) ? translation.ui_bindings.length : 0,
    ruleCount: Array.isArray(translation.update_rules) ? translation.update_rules.length : 0,
    fallbackCount: Array.isArray(translation.fallback_fragments) ? translation.fallback_fragments.length : 0,
    // 需要原卡脚本才能做的部分（DSH 里不执行卡内 JS），面板只列出说明。
    fallbacks: (Array.isArray(translation.fallback_fragments) ? translation.fallback_fragments : []).map((fragment) => ({
      description: String(fragment?.description ?? ''),
      reason: String(fragment?.reason ?? ''),
    })),
  }
}
//#endregion
