// MVU interactions 分发的纯逻辑层：动作扁平化 + value_expr 解释 + 执行计划。
// 实际执行（写变量 / 触发写作）由 composables/useMvuStatusPanel.js 承担。
//
// InteractionAction（serde tag="kind", snake_case）：
//   modify_variable { key, value_expr } / trigger_next_turn { hint }
//   / multi { actions } / run_original_js { js_snippet, description }（留桩不执行）

import {
  expandMvuTemplateKey,
  findMvuVariableForInstance,
  normalizeMvuKey,
} from './mvuKey.js'

const MAX_FLATTEN_DEPTH = 5

// M-6：evaluateMvuValueExpr 的 JSON.parse 字面量边界。JSON.parse 本身不执行 JS，
// 但卡可塞任意大/深嵌套 JSON 字面量到变量，下游 buildMvuStatDataTree 按 `.` 分段
// 递归建树无深度限制，深嵌套会放大内存/CPU 压力。这两条上限把字面量约束在合理范围。
const MAX_MVU_EXPR_BYTES = 64 * 1024 // 64KB，足够任何合理的 stat 字面量
const MAX_MVU_EXPR_DEPTH = 32 // 嵌套层数上限

/**
 * M-6：校验 MVU value_expr 的 JSON 字面量——字节上限 + 嵌套深度上限。
 * @param {string} raw 原始表达式字符串
 * @returns {{ok:false,reason:string}|{ok:true}}
 */
function validateMvuJsonLiteral(raw) {
  if (raw.length > MAX_MVU_EXPR_BYTES) {
    return { ok: false, reason: `表达式过长（${raw.length} 字节，上限 ${MAX_MVU_EXPR_BYTES}）` }
  }
  // 深度只能对结构化 JSON 计算；裸词字符串字面量（下面的分支）走另一条路径，
  // 这里仅约束 JSON.parse 成功后的值。
  return { ok: true }
}

/**
 * M-6：测量一个已解析 JSON 值的最大嵌套深度。
 * @param {unknown} value
 * @param {number} depth
 * @returns {number}
 */
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

// 含这些标记的表达式视为 JS，不原生解释（留给 WebView/后处理路线）
const JS_EXPR_MARKERS = /[=(){}[\];`]|getvar|setvar|Math\.|\$\{|=>|await|fetch/

// 混入算术运算的字符串不得当字面量落盘（防 "hp - 10" 类被写成字符串）
const ARITHMETIC_MIX = /[+*/%]|\s-|-\s|\d-|-\d/

/**
 * 解释 modify_variable 的 value_expr：
 * - "+N"：数值增量（"+5" 不是合法 JSON，只能是增量语义）
 * - "-N"：当前值是数值 → 增量；否则字面量负数
 * - "<key> ± N"（分析器一号示例格式，如 "hp - 10"）：前导标识符必须等于
 *   目标 key，按增量解释；引用其他变量 → 不支持
 * - 合法 JSON → 字面量（数字/字符串/布尔/对象）
 * - 无 JS 标记且无算术混排的裸词（如 战斗中）→ 字符串字面量
 * - 其余 → 不支持（needs WebView/后处理），绝不落成字符串
 */
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

  // "<ident> ± N"：ident 必须是目标变量本身（"hp - 10" 之于 key="hp"）。
  // 记法归一后比对，兜住存量产物 key 与表达式记法不一致的情况
  // （如 key="hp"，expr="stat_data.hp - 10"）
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
    // M-6：先做字节上限校验，再解析；解析后做嵌套深度校验。
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

/**
 * 把一个 InteractionMapping 编成执行计划（纯函数，可测）：
 * { label, writes: [{key, value}], hints: [string], skipped: [{kind, reason, ...}] }
 */
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
      // 跨记法 + 模板键展开查现有变量；写回优先用已存储的键（避免同一变量
      // 两种记法并存），无现有变量时用展开后的归一化键落新变量
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
