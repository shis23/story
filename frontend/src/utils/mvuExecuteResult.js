// M-04：mvu:execute_result 回传前的前端镜像守卫。
//
// variable_updates 的键由卡派生的 JS 片段（MvuTranslation.fallback_fragments[].js_snippet）
// 产出，后端写入边界（crates/app-pipeline/src/lib.rs 的 mvu_execute_result 分支）不做
// 命名空间过滤——该 Rust 侧守卫由另一 owner 补，这里先在前端镜像拒绝 __storyforge*
// 保留命名空间（卡写 __storyforge_card_shell_variables 即可覆盖卡壳变量桶）。
// 与 shellVariableOutbox.js 的 isReservedNamespace（M-6）同口径。

import { isReservedNamespace } from './shellVariableOutbox.js'

/**
 * 把 iframe 的 mvu:execute_result 消息整理成 invoke('mvu_execute_result') 的载荷。
 * 保留命名空间的键直接丢弃（fail closed），并通过 onDrop 或 console.warn 报告。
 * @param {object} message iframe postMessage 回传的 mvu:execute_result 数据
 * @param {(keys: string[]) => void} [onDrop] 丢弃键的回调（测试/日志注入）
 * @returns {{requestId: string|undefined, variableUpdates: Record<string, any>, sideEffects: any[], error: string|null}}
 */
export function buildMvuExecuteResultData(message = {}, onDrop) {
  const variableUpdates = {}
  const dropped = []
  const raw = message?.variable_updates
  if (raw && typeof raw === 'object' && !Array.isArray(raw)) {
    for (const [key, value] of Object.entries(raw)) {
      if (isReservedNamespace(key)) {
        dropped.push(key)
        continue
      }
      variableUpdates[key] = value
    }
  }
  if (dropped.length) {
    if (typeof onDrop === 'function') onDrop(dropped)
    else console.warn('[MVU] mvu_execute_result dropped reserved-namespace keys:', dropped)
  }
  return {
    requestId: message?.request_id,
    variableUpdates,
    sideEffects: Array.isArray(message?.side_effects) ? message.side_effects : [],
    error: message?.error || null,
  }
}
