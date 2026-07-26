// MVU interactions 分发的纯逻辑层：动作扁平化 + value_expr 解释 + 执行计划。
// 实际执行（写变量 / 触发写作）由 composables/useMvuStatusPanel.js 承担。
//
// InteractionAction（serde tag="kind", snake_case）：
//   modify_variable { key, value_expr } / trigger_next_turn { hint }
//   / multi { actions } / run_original_js { js_snippet, description }（留桩不执行）

const MAX_FLATTEN_DEPTH = 5

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

  // "<ident> ± N"：ident 必须是目标变量本身（"hp - 10" 之于 key="hp"）
  const keyedDeltaMatch = raw.match(/^([\p{L}\p{N}_.]+)\s*([+-])\s*(\d+(?:\.\d+)?)$/u)
  if (keyedDeltaMatch && !/^\d/.test(keyedDeltaMatch[1])) {
    if (key != null && keyedDeltaMatch[1] === key) {
      const delta = Number(keyedDeltaMatch[3]) * (keyedDeltaMatch[2] === '-' ? -1 : 1)
      return { ok: true, value: (hasNumericCurrent ? current : 0) + delta }
    }
    return {
      ok: false,
      reason: `表达式引用变量 ${keyedDeltaMatch[1]}，与目标 key 不符，无法原生解释`,
    }
  }

  try {
    return { ok: true, value: JSON.parse(raw) }
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
export function planMvuInteraction(mapping, { variables } = {}) {
  const vars = Array.isArray(variables) ? variables : []
  const plan = {
    label: mapping?.element_label || '',
    writes: [],
    hints: [],
    skipped: [],
  }
  for (const action of flattenInteractionActions(mapping?.actions)) {
    if (action.kind === 'modify_variable') {
      const currentValue = vars.find((v) => v?.key === action.key)?.value
      const result = evaluateMvuValueExpr(action.value_expr, currentValue, action.key)
      if (result.ok) {
        plan.writes.push({ key: action.key, value: result.value })
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
