/**
 * errorText — 把后端/插件错误对象转成用户可读文案。
 *
 * Tauri 命令错误是结构化 DTO（crates/tauri-app/src/error.rs）：
 *   { type: 'validation' | 'llm' | 'pipeline' | ..., message: string }
 * 直接模板字符串拼接会得到 "[object Object]"（2026-08-31 验收：待采纳
 * 屏障拒绝的提示条只显示 [object Object]）。
 */
export function isCancelledError(e) {
  return e?.type === 'cancelled'
}

export function errorText(e) {
  if (e == null) return '未知错误'
  if (typeof e === 'string') return e
  if (typeof e === 'object') {
    // JS Error 实例：保留 "TypeError: xxx" 形式（含错误类别）
    if (e instanceof Error) {
      const s = e.toString()
      if (s && s !== '[object Object]') return s
    }
    // Tauri 命令错误 DTO：{ type, message }
    if (typeof e.message === 'string' && e.message) return e.message
    const known = {
      cancelled: '已停止', storage: '存储失败', validation: '输入无效',
      not_found: '内容不存在', llm: '模型请求失败', pipeline: '写作流程失败',
      internal: '内部错误', import: '导入失败',
    }
    if (known[e.type]) return known[e.type]
    if (typeof e.toString === 'function') {
      const s = e.toString()
      if (s && s !== '[object Object]') return s
    }
  }
  return typeof e === 'object' ? '未知错误' : String(e)
}
